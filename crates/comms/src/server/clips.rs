use super::*;
use axum::extract::State;
use std::path::PathBuf;

pub(super) fn router(database: PathBuf) -> Router {
    Router::new()
        .route("/clips", get(list))
        .route("/clips/document", get(document))
        .with_state(database)
}

#[derive(Default, Deserialize)]
struct ListQuery {
    q: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn list(State(database): State<PathBuf>, Query(query): Query<ListQuery>) -> HttpResponse {
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let offset = query.offset.unwrap_or(0);
    if offset > i64::MAX as usize - limit - 1 {
        return error_response(StatusCode::BAD_REQUEST, "offset is too large");
    }
    match tokio::task::spawn_blocking(move || {
        Store::open(&database)
            .map_err(|_| comms::store::ClipError::Storage)?
            .search_web_clip_documents(query.q.as_deref().unwrap_or("").trim(), limit + 1, offset)
    })
    .await
    {
        Ok(Ok(mut rows)) => {
            let next_offset = (rows.len() > limit).then_some(offset + limit);
            rows.truncate(limit);
            (
                StatusCode::OK,
                Json(json!({"data_class": "c2", "documents": rows, "next_offset": next_offset})),
            )
        }
        _ => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "clip storage unavailable",
        ),
    }
}

#[derive(Deserialize)]
struct DocumentQuery {
    key: String,
}

async fn document(
    State(database): State<PathBuf>,
    Query(query): Query<DocumentQuery>,
) -> HttpResponse {
    match tokio::task::spawn_blocking(move || {
        Store::open(&database)
            .map_err(|_| comms::store::ClipError::Storage)?
            .web_clip_document(&query.key)
    })
    .await
    {
        Ok(Ok(Some(document))) => {
            let mut body = json!(document);
            body["data_class"] = json!("c2");
            (StatusCode::OK, Json(body))
        }
        Ok(Ok(None)) => error_response(StatusCode::NOT_FOUND, "clip not found"),
        _ => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "clip storage unavailable",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    #[tokio::test]
    async fn clips_are_discoverable_and_readable_through_the_agent_gate() {
        let dir = std::env::temp_dir().join(format!("comms-clips-api-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let database = dir.join("clips.db");
        let store = Store::open(&database).unwrap();
        for (id, behavior, text) in [
            ("first", "overwrite", "Old Erika answer"),
            ("second", "append-specific", "\nNew Erika answer 🦀"),
            ("other", "create", "Unrelated"),
        ] {
            store
                .put_web_clip(comms::store::WebClipInput {
                    request_id: id.into(),
                    url: "https://example.org/thread".into(),
                    payload: json!({"version": 1, "behavior": behavior, "title": "AI Studio",
                    "markdown": text, "destination": {"path": "Clippings", "name": "Example"}}),
                })
                .unwrap();
        }
        let key = "note:Clippings/Example";
        let expected = store.web_clip_document(key).unwrap().unwrap().markdown;
        let registry = sjel_pseudonymize::EntityRegistry::builder()
            .add_person("Erika")
            .build();
        let policy = std::sync::Arc::new(sjel_server::agent_policy::PolicyFiles::new(
            dir.join("policy"),
        ));
        policy
            .set_mode("comms", sjel_server::agent_policy::Mode::ReadOnly)
            .unwrap();
        let access = sjel_server::agent::AgentAccess::new(
            sha2::Sha256::digest(b"clip-agent-test").into(),
            vec![7; 32],
            registry,
        )
        .for_capability("comms", AGENT_ROUTES, Some(policy.clone()));
        let auth = sjel_server::InboundAuth::with_token(Some("clip-owner-test".into()))
            .refuse_without_token()
            .with_agent_access(access);
        let app = sjel_server::authenticated(router(database), auth);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::new();
        for path in ["/clips", "/clips/document?key=note%3AClippings%2FExample"] {
            assert_eq!(
                client
                    .get(format!("{base}{path}"))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                401
            );
        }
        let list: Value = client
            .get(format!("{base}/clips"))
            .bearer_auth("clip-agent-test")
            .query(&[("q", "new ERIKA"), ("limit", "1")])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(list["documents"].as_array().unwrap().len(), 1);
        assert_eq!(list["documents"][0]["document_key"], key);
        assert_eq!(list["documents"][0]["versions"], 2);
        assert_eq!(list["documents"][0]["data_class"], "c2");
        assert!(list["documents"][0].get("markdown").is_none());
        let page: Value = client
            .get(format!("{base}/clips/document"))
            .bearer_auth("clip-agent-test")
            .query(&[("key", key)])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(page["data_class"], "c2");
        assert!(page["markdown"].as_str().unwrap().contains("answer"));
        assert!(!page["markdown"].as_str().unwrap().contains("Erika"));
        let owner: Value = client
            .get(format!("{base}/clips/document"))
            .bearer_auth("clip-owner-test")
            .query(&[("key", key)])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(owner["markdown"], expected);
        assert_eq!(
            client
                .get(format!("{base}/clips/document"))
                .bearer_auth("clip-agent-test")
                .query(&[("key", "missing")])
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        let first: Value = client
            .get(format!("{base}/clips"))
            .bearer_auth("clip-agent-test")
            .query(&[("limit", "1")])
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(first["next_offset"], 1);
        let second: Value = client
            .get(format!("{base}/clips"))
            .bearer_auth("clip-agent-test")
            .query(&[("limit", "1"), ("offset", "1")])
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(second["documents"][0]["document_key"], key);
        assert!(second["next_offset"].is_null());
        let literal: Value = client
            .get(format!("{base}/clips"))
            .bearer_auth("clip-agent-test")
            .query(&[("q", "%")])
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(literal["documents"].as_array().unwrap().is_empty());
        policy.set_cap("default", Some("c2")).unwrap();
        assert_eq!(
            client
                .get(format!("{base}/clips"))
                .bearer_auth("clip-agent-test")
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        policy.set_cap("default", Some("c1")).unwrap();
        assert_eq!(
            client
                .get(format!("{base}/clips"))
                .bearer_auth("clip-agent-test")
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        assert_eq!(
            client
                .get(format!("{base}/clips/document"))
                .bearer_auth("clip-agent-test")
                .query(&[("key", key)])
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        policy.set_cap("default", None).unwrap();
        policy
            .set_mode("comms", sjel_server::agent_policy::Mode::Off)
            .unwrap();
        assert_eq!(
            client
                .get(format!("{base}/clips"))
                .bearer_auth("clip-agent-test")
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        server.abort();
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
