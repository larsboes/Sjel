use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;

use comms::config::Config;
use comms::store::{
    CloudAttemptClaim, CloudDerivativeApproval, CloudQueueRequest, FeedItem, Store, StoredDigest,
};
use comms::{cloud_dispatch, cloud_run, digest, mail_model, media, relevance, summarize};
use sjel_runtime::Category;

fn policy(root: &Path, selection: &str, allow: &[Category]) {
    std::fs::write(
        root.join("config/runtime/test.json"),
        serde_json::json!({ "selection": selection, "allow": allow }).to_string(),
    )
    .unwrap();
}

fn inference(base: &str, afm: bool, window: Option<u32>) -> sjel_inference::InferenceConfig {
    let backend = if afm { "foundation-models" } else { "local" };
    let model = if afm {
        "apple-foundationmodel"
    } else {
        "other-model"
    };
    serde_json::from_value(serde_json::json!({
        "backends": {
            backend: { "api": "openai", "base_url": base },
            "hosted": { "api": "openai", "base_url": "https://example.invalid/v1" }
        },
        "roles": {
            "summarization": { "backend": backend, "model": model, "max_input_tokens": window },
            "summarization_light": { "backend": backend, "model": model, "max_input_tokens": window },
            "embedding": { "backend": backend, "model": "embedder" },
            "cloud_public": {
                "backend": "hosted", "model": "cloud-model", "provider_name": "Some Provider",
                "cloud_data_tier": "public", "billing_mode": "free_only",
                "max_requests_per_day": 10, "max_input_tokens": 24000
            }
        }
    })).unwrap()
}

fn stored_digest(id: &str) -> StoredDigest {
    StoredDigest {
        source: "feed".into(),
        item_id: id.into(),
        text: Some("Keep this digest".into()),
        state: "generated".into(),
        shape: "standard".into(),
        depth: "standard".into(),
        focus: String::new(),
        producer: "previous:producer".into(),
        source_chars: 4000,
        redactions: 0,
        attempts: 2,
        last_error: Some("previous failure".into()),
        diagram: None,
        diagram_state: None,
        diagram_error: None,
        chart: None,
        chart_state: None,
        chart_error: None,
        generated_at: String::new(),
    }
}

fn exercise(root: &Path) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let mut cfg = Config::load();
    cfg.database_path = root.join("comms.db");
    cfg.google_env_path = root.join("absent-google.env");
    cfg.inference = inference(&base, false, Some(24000));
    let store = Store::open(&cfg.database_path).unwrap();
    let mut item = FeedItem::new("https://example.test/cached", "news", "article");
    item.data_class = "c0".into();
    item.transcript = Some("source ".repeat(500));
    store.upsert_feed(&item).unwrap();
    store
        .upsert_content_digest(&stored_digest(&item.id))
        .unwrap();
    store
        .update_content_diagram(
            "feed",
            &item.id,
            Some("graph TD; A-->B;"),
            "generated",
            None,
            "old:diagram",
        )
        .unwrap();
    store
        .update_content_chart(
            "feed",
            &item.id,
            Some("chart"),
            "generated",
            None,
            "old:chart",
        )
        .unwrap();
    store
        .update_feed_summary(&item.id, "Keep this summary", "old:summary")
        .unwrap();
    store
        .record_summary_attempt(&item.id, "timeout", "old:summary")
        .unwrap();
    let before = store.content_digest("feed", &item.id).unwrap().unwrap();
    let summary_before = store.get_feed(&item.id).unwrap().unwrap();
    let producers = digest::producer_revisions(&cfg);
    policy(root, "on-the-go", &[]);
    assert_eq!(
        sjel_runtime::current().unwrap().effective,
        sjel_runtime::Selection::OnTheGo
    );

    for action in [
        digest::generate(
            &store,
            &cfg,
            "feed",
            &item.id,
            &summarize::Directive::default(),
        ),
        digest::generate_diagram(&store, &cfg, "feed", &item.id),
        digest::generate_chart(&store, &cfg, "feed", &item.id),
    ] {
        assert!(sjel_runtime::is_deferred(&action.unwrap_err().to_string()));
    }
    digest::store_cloud_failure(
        &store,
        "feed",
        &item.id,
        "new:producer",
        "deferred by runtime profile: remote-models",
    );
    let after = store.content_digest("feed", &item.id).unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(digest::to_contract(&before)).unwrap(),
        serde_json::to_value(digest::to_contract(&after)).unwrap(),
    );
    assert!(media::summarize_item(&store, &cfg, &item.id).is_err());
    assert!(media::summarize_pending(&store, &cfg).is_err());
    let summary_after = store.get_feed(&item.id).unwrap().unwrap();
    assert_eq!(summary_before.summary, summary_after.summary);
    assert_eq!(
        summary_before.summary_attempts,
        summary_after.summary_attempts
    );
    assert_eq!(
        summary_before.summary_next_attempt,
        summary_after.summary_next_attempt
    );
    assert_eq!(
        summary_before.summary_provenance,
        summary_after.summary_provenance
    );
    assert_eq!(producers, digest::producer_revisions(&cfg));
    assert!(mail_model::run_pass(&cfg, &store, mail_model::Mode::Shadow, 10, 0).is_err());

    let embedding = cfg.embedding_role().unwrap();
    let profile = relevance::InterestProfile {
        key: "lens".into(),
        label: "Lens".into(),
        focus: String::new(),
        text: "source".into(),
        fingerprint: "revision".into(),
    };
    let cached_match = relevance::RelevanceMatch {
        profile_key: profile.key.clone(),
        profile_label: profile.label.clone(),
        score: 0.8,
        rationale: "Keep these scores".into(),
        mode: "semantic".into(),
        profile_revision: profile.fingerprint.clone(),
    };
    store
        .replace_feed_relevance(&item.id, &[cached_match])
        .unwrap();
    for allow in [vec![], vec![Category::BulkIndexing]] {
        policy(root, "on-the-go", &allow);
        let outcome = relevance::score_items(
            std::slice::from_ref(&item),
            std::slice::from_ref(&profile),
            Some(&embedding),
            None,
        );
        assert!(outcome
            .deferred
            .as_deref()
            .is_some_and(sjel_runtime::is_deferred));
        assert!(
            outcome.items.is_empty(),
            "a deferred pass must emit no lexical replacement"
        );
        assert_eq!(
            store.feed_relevance(&item.id).unwrap()[0].rationale,
            "Keep these scores"
        );
    }
    policy(root, "on-the-go", &[]);
    let mut pending = item.clone();
    pending.url = "https://example.test/pending".into();
    pending.id = comms::store::feed_id(&pending.url);
    store.upsert_feed(&pending).unwrap();
    assert!(digest::generate(
        &store,
        &cfg,
        "feed",
        &pending.id,
        &summarize::Directive::default()
    )
    .is_err());
    let deferred = store.content_digest("feed", &pending.id).unwrap().unwrap();
    assert_eq!(deferred.state, "policy_deferred");
    assert_eq!(deferred.attempts, 0);
    assert!(store
        .items_needing_digest("feed", &producers, &producers, 3, 500)
        .unwrap()
        .contains(&pending.id));

    store
        .stage_cloud_derivative(&CloudDerivativeApproval {
            source: "feed".into(),
            item_id: item.id.clone(),
            source_revision: "rev".into(),
            preview_hash: "hash".into(),
            original_data_class: "c0".into(),
            derivative_data_class: "c0".into(),
            transformation: comms::cloud_derivative::PASSTHROUGH_VERSION.into(),
            document: "reviewed document".into(),
            redaction_count: 0,
        })
        .unwrap();
    let queued = store
        .queue_cloud_derivative(&CloudQueueRequest {
            source: "feed".into(),
            item_id: item.id.clone(),
            source_revision: "rev".into(),
            preview_hash: "hash".into(),
            provider_role: "cloud_public".into(),
            task: cloud_dispatch::DIGEST_TASK_VERSION.into(),
        })
        .unwrap();
    let job_id = queued.job_id.unwrap();
    assert!(sjel_runtime::is_deferred(
        &cloud_run::run_job(&store, &cfg, &job_id).unwrap_err()
    ));
    assert_eq!(store.cloud_provider_calls_today("cloud_public").unwrap(), 0);
    assert_eq!(
        store
            .cloud_job_for_dispatch(&job_id)
            .unwrap()
            .unwrap()
            .provider_calls,
        0
    );
    assert!(store.list_egress_entries(10).unwrap().is_empty());

    // A switch after claiming but before sending refunds the exact live reservation.
    policy(root, "normal", &[]);
    let CloudAttemptClaim::Started(attempt) = store
        .claim_cloud_job_attempt(&job_id, "cloud_public", "cloud-model", 1)
        .unwrap()
    else {
        panic!("claim")
    };
    policy(root, "on-the-go", &[]);
    let hosted = cfg.inference.role("cloud_public").unwrap();
    assert!(sjel_runtime::is_deferred(
        &cloud_dispatch::analyze(&hosted, "reviewed document").unwrap_err()
    ));
    assert!(!store
        .release_cloud_job_attempt(&job_id, attempt + 100)
        .unwrap());
    assert!(store.release_cloud_job_attempt(&job_id, attempt).unwrap());
    assert!(!store.release_cloud_job_attempt(&job_id, attempt).unwrap());
    assert_eq!(store.cloud_provider_calls_today("cloud_public").unwrap(), 0);
    assert_eq!(
        store
            .cloud_provider_recent_outcomes("cloud_public", 60)
            .unwrap(),
        (0, 0)
    );
    assert_eq!(
        store
            .cloud_job_for_dispatch(&job_id)
            .unwrap()
            .unwrap()
            .provider_calls,
        0
    );
    assert!(store.list_egress_entries(10).unwrap().is_empty());
    assert!(
        listener.accept().is_err(),
        "no prohibited local call or probe was made"
    );

    // AFM remains usable for bounded work, but unknown/overflow windows never send.
    cfg.inference = inference(&base, true, None);
    assert!(digest::generate(
        &store,
        &cfg,
        "feed",
        &item.id,
        &summarize::Directive::new(summarize::Depth::Detailed, [])
    )
    .is_err());
    cfg.inference = inference(&base, true, Some(4096));
    assert!(matches!(
        media::summarize(&"x".repeat(20000), &cfg, "c0"),
        media::SummarizeOutcome::OverWindow | media::SummarizeOutcome::PolicyDeferred(_)
    ));
    assert!(
        listener.accept().is_err(),
        "unknown/overflow AFM work was sent"
    );
    listener.set_nonblocking(false).unwrap();
    let server = listener.try_clone().unwrap();
    let thread = std::thread::spawn(move || loop {
        let (mut stream, _) = server.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let health = line.starts_with("GET /health ");
        assert!(health || line.starts_with("POST /v1/chat/completions "));
        let mut length = 0;
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line.trim().is_empty() {
                break;
            }
            if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
        }
        let payload = if health {
            assert_eq!(length, 0);
            r#"{"model_available":true}"#
        } else {
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["model"], "apple-foundationmodel");
            r#"{"choices":[{"message":{"content":"Allowed AFM summary"}}]}"#
        };
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len()).unwrap();
        if !health {
            break;
        }
    });
    assert!(matches!(
        media::summarize(&"x".repeat(600), &cfg, "c0"),
        media::SummarizeOutcome::Ok(_)
    ));
    thread.join().unwrap();
    policy(root, "normal", &[]);
    assert!(relevance::runtime_admission(Some(&embedding), None).is_ok());
    cfg.inference = inference(&base, false, Some(24000));
    assert_eq!(digest::producer_revisions(&cfg), producers);
    assert!(store
        .items_needing_digest("feed", &producers, &producers, 3, 500)
        .unwrap()
        .contains(&pending.id));
    let CloudAttemptClaim::Started(attempt) = store
        .claim_cloud_job_attempt(&job_id, "cloud_public", "cloud-model", 1)
        .unwrap()
    else {
        panic!("refunded claim remains retryable")
    };
    assert!(store.release_cloud_job_attempt(&job_id, attempt).unwrap());
}

#[test]
fn runtime_profiles_preserve_work_and_resume() {
    if let Some(root) = std::env::var_os("COMMS_RUNTIME_TEST_ROOT") {
        exercise(Path::new(&root));
        return;
    }
    // Child-only environment: sibling tests and the real overlay are never changed.
    let root = std::env::temp_dir().join(format!("comms-runtime-{}", std::process::id()));
    std::fs::create_dir_all(root.join("config/runtime")).unwrap();
    std::fs::write(root.join("config/comms.json"), "{}").unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "runtime_profiles_preserve_work_and_resume",
            "--nocapture",
        ])
        .env("COMMS_RUNTIME_TEST_ROOT", &root)
        .env("SJEL_OVERLAY_ROOT", &root)
        .env("SJEL_PERSONAL_ROOT", &root)
        .env("SJEL_MACHINE_TOML", root.join("machines/test.toml"))
        .env("SJEL_COMMS_CONFIG", root.join("config/comms.json"))
        .env("SJEL_INFERENCE_CONFIG", root.join("absent-inference.json"))
        .status()
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert!(status.success());
}
