use super::*;

#[cfg(test)]
#[tokio::test]
async fn people_decisions_verify_the_target_before_replacing_a_saved_link() {
    let path =
        std::env::temp_dir().join(format!("places-person-decision-{}.db", std::process::id()));
    for tail in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
    }
    let state = AppState {
        database_path: Arc::new(path.clone()),
    };
    let mock = Router::new().route(
        "/api/entities/{id}",
        get(
            |axum::extract::Path(id): axum::extract::Path<String>| async move {
                match id.as_str() {
                    "ent:missing" => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))),
                    "ent:unavailable" => (
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(json!({"error": "offline"})),
                    ),
                    "ent:malformed" => (StatusCode::OK, Json(json!({"error": "wrong body"}))),
                    "ent:wrong-id" => (
                        StatusCode::OK,
                        Json(json!({"id": "ent:other", "kind": "person"})),
                    ),
                    "ent:organisation" => (
                        StatusCode::OK,
                        Json(json!({"id": id, "kind": "organisation"})),
                    ),
                    _ => (StatusCode::OK, Json(json!({"id": id, "kind": "person"}))),
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let decision = |id: Option<&str>| sjel_links::Decision {
        name: "Synthetic Traveller".into(),
        entity_id: id.map(str::to_owned),
    };
    let (status, _) =
        people_decide_at(state.clone(), decision(Some("ent:person")), base.clone()).await;
    assert_eq!(status, StatusCode::OK);
    for (id, expected) in [
        ("ent:missing", StatusCode::BAD_REQUEST),
        ("ent:organisation", StatusCode::BAD_REQUEST),
        ("ent:unavailable", StatusCode::SERVICE_UNAVAILABLE),
        ("ent:malformed", StatusCode::SERVICE_UNAVAILABLE),
        ("ent:wrong-id", StatusCode::SERVICE_UNAVAILABLE),
        ("trip:plan:1", StatusCode::BAD_REQUEST),
    ] {
        let (status, _) = people_decide_at(state.clone(), decision(Some(id)), base.clone()).await;
        assert_eq!(status, expected, "{id}");
        let store = PlacesStore::open(&path).unwrap();
        assert_eq!(
            places::people::decisions(&store).unwrap()["synthetic traveller"].as_deref(),
            Some("ent:person")
        );
    }
    server.abort();
    let offline = "http://127.0.0.1:0".to_string();
    let (status, _) =
        people_decide_at(state.clone(), decision(Some("ent:person")), offline.clone()).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let store = PlacesStore::open(&path).unwrap();
    assert_eq!(
        places::people::decisions(&store).unwrap()["synthetic traveller"].as_deref(),
        Some("ent:person")
    );
    let (status, _) = people_decide_at(state, decision(None), offline).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        places::people::decisions(&store).unwrap()["synthetic traveller"],
        None
    );
    drop(store);
    for tail in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
    }
}
