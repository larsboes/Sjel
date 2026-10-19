use super::*;

#[tokio::test]
async fn plan_reads_project_only_their_saved_traveler_decisions() {
    let path = std::env::temp_dir().join(format!(
        "trips-traveler-projection-{}.db",
        std::process::id()
    ));
    let store = TripsStore::open(&path).unwrap();
    let input = serde_json::from_value(json!({
        "title": "Synthetic trip", "origin": {"id":"o","name":"Origin"},
        "destinations":[{"id":"d","name":"Destination"}],
        "date_start":"2026-10-16", "date_end":"2026-10-17",
        "travelers":["Synthetic Alex", "Synthetic Team", "Synthetic Unknown"]
    }))
    .unwrap();
    let plan = store.create_plan(&input).unwrap();
    trips::people::decide(&store, "Synthetic Alex", Some("ent:chosen-alex")).unwrap();
    trips::people::decide(&store, "Synthetic Team", None).unwrap();
    trips::people::decide(&store, "Unrelated", Some("ent:unrelated")).unwrap();
    let state = AppState {
        database_path: Arc::new(path.clone()),
        obsidian: None,
        travel: Arc::new(Default::default()),
        export_lock: Arc::new(tokio::sync::Mutex::new(())),
    };
    let (status, Json(detail)) = get_plan(State(state.clone()), Path(plan.id.clone())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["traveler_links"][0]["entity_id"], "ent:chosen-alex");
    assert_eq!(detail["traveler_links"][1]["status"], "not_person");
    assert_eq!(detail["traveler_links"][2]["status"], "unresolved");
    assert_eq!(detail["travelers"], json!(input.travelers));
    assert!(detail["items"].is_array());
    assert!(!detail.to_string().contains("ent:unrelated"));
    let (status, Json(list)) = list_plans(State(state.clone())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list[0]["traveler_links"], detail["traveler_links"]);
    let (status, Json(updated)) = update_plan(
        State(state),
        Path(plan.id.clone()),
        Json(UpdatePlan {
            title: Some("Renamed synthetic trip".into()),
            ..Default::default()
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["traveler_links"], detail["traveler_links"]);
    let persisted = serde_json::to_value(store.get_plan(&plan.id).unwrap().unwrap()).unwrap();
    assert!(persisted.get("traveler_links").is_none());
    drop(store);
    for tail in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
    }
}

#[cfg(test)]
#[tokio::test]
async fn people_decisions_verify_the_target_before_replacing_a_saved_link() {
    let path =
        std::env::temp_dir().join(format!("trips-person-decision-{}.db", std::process::id()));
    for tail in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
    }
    let state = AppState {
        database_path: Arc::new(path.clone()),
        obsidian: None,
        travel: Arc::new(Default::default()),
        export_lock: Arc::new(tokio::sync::Mutex::new(())),
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
        let store = TripsStore::open(&path).unwrap();
        assert_eq!(
            trips::people::decisions(&store).unwrap()["synthetic traveller"].as_deref(),
            Some("ent:person")
        );
    }
    server.abort();
    let offline = "http://127.0.0.1:0".to_string();
    let (status, _) =
        people_decide_at(state.clone(), decision(Some("ent:person")), offline.clone()).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let store = TripsStore::open(&path).unwrap();
    assert_eq!(
        trips::people::decisions(&store).unwrap()["synthetic traveller"].as_deref(),
        Some("ent:person")
    );
    let (status, _) = people_decide_at(state, decision(None), offline).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        trips::people::decisions(&store).unwrap()["synthetic traveller"],
        None
    );
    drop(store);
    for tail in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
    }
}
