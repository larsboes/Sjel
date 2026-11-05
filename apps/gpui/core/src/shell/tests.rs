//! Unit tests for `shell`.

use super::*;

#[test]
fn project_inventory_link_types_require_review_revisions() {
    assert_eq!(inventory_reference("a:é").as_deref(), Some("inv:613ac3a9"));
    assert!(inventory_reference("").is_none());
    assert!(inventory_reference(&"x".repeat(257)).is_none());
    let record: ProjectLinkRecord = serde_json::from_value(serde_json::json!({"path":"P/P.md","title":"P","content":"  exact\n","updated_at":"stamp","kind":"project","archived":false})).unwrap();
    assert_eq!(record.content, "  exact\n");
    assert!(serde_json::from_value::<ProjectLinkRecord>(serde_json::json!({"path":"P/P.md","title":"P","content":"x","kind":"project","archived":false})).is_err());
    let link: ProjectInventoryLink = serde_json::from_value(serde_json::json!({"record_path":"P/P.md","item_ref":"inv:61","verified_revision":4,"reviewed_at":"stamp","provenance":"operator_review"})).unwrap();
    assert_eq!(link.item_ref, "inv:61");
}

#[test]
fn project_mail_contract_preserves_content_and_render_failure() {
    let page: ProjectMailPage = serde_json::from_value(serde_json::json!({"limit": 500, "proposals": [{
            "id": 8, "version": 1, "project": "P", "title": "T", "evidence": "quoted\ntext",
            "source_id": "mail:v1:thread-1:message_2", "data_class": "c2", "task_suggestion": "follow up", "status": "pending"
        }]})).unwrap();
    assert_eq!(
        page.proposals[0].gmail_url().as_deref(),
        Some("https://mail.google.com/mail/u/0/#all/thread-1")
    );
    let mut proposal = page.proposals[0].clone();
    proposal.source_id = "mail:v1:../evil:message".into();
    assert!(proposal.gmail_url().is_none());
    let target: ProjectMailTarget = serde_json::from_value(
        serde_json::json!({"path":"P/P.md", "title":"P", "content":"  base\n"}),
    )
    .unwrap();
    assert_eq!(target.content, "  base\n");
    assert!(serde_json::from_value::<ProjectMailTarget>(
        serde_json::json!({"path":"P/P.md", "title":"P"})
    )
    .is_err());
    let result: ProjectMailDecision = serde_json::from_value(serde_json::json!({"applied": true, "rendered": false, "target_path":"P/P.md", "render_error":"disk full"})).unwrap();
    assert!(result.applied && !result.rendered);
    assert_eq!(result.render_error.as_deref(), Some("disk full"));
}

#[test]
fn a_refusal_keeps_its_reason_in_json_or_text() {
    assert_eq!(error_text(br#"{"error":"bad"}"#).as_deref(), Some("bad"));
    assert_eq!(error_text(br#"{"detail":"x"}"#), None);
    assert_eq!(
        error_text(b"no cheaper split for this route\n").as_deref(),
        Some("no cheaper split for this route")
    );
    assert_eq!(error_text(b""), None);
    assert_eq!(error_text(b"<html>502</html>"), None);
    assert_eq!(error_text("x".repeat(301).as_bytes()), None);
}

#[test]
fn only_the_inbound_auth_refusal_triggers_a_read_retry() {
    assert!(authentication_rejection(
        br#"{"error":"invalid or missing authentication token"}"#
    ));
    assert!(!authentication_rejection(br#"{"error":"not allowed"}"#));
    assert!(!authentication_rejection(
        b"invalid or missing authentication token"
    ));
}

#[test]
fn auth_retry_never_replays_writes_or_a_second_refusal() {
    assert!(should_retry_auth(&Method::GET, false, true, 0));
    assert!(should_retry_auth(&Method::HEAD, false, true, 0));
    assert!(!should_retry_auth(&Method::POST, false, true, 0));
    assert!(!should_retry_auth(&Method::GET, true, true, 0));
    assert!(!should_retry_auth(&Method::GET, false, false, 0));
    assert!(!should_retry_auth(&Method::GET, false, true, 1));
}

#[test]
fn software_keys_stay_one_path_segment() {
    assert_eq!(software_key("com.example.app").unwrap(), "com.example.app");
    assert_eq!(
        software_key("npm:@playwright/cli").unwrap(),
        "npm:%40playwright%2Fcli"
    );
    assert!(software_key("../etc").is_ok_and(|k| !k.contains('/')));
    assert!(software_key("a b").is_err());
    assert!(software_key("x?y=1").is_err());
    assert!(software_key("").is_err());
    assert!(software_key("..").is_err());
}

#[test]
fn deterministic_fallback_only_answers_capability_status_questions() {
    assert!(is_capability_status_question(
        "Which capabilities are up right now?"
    ));
    assert!(is_capability_status_question("Show capability health"));
    assert!(!is_capability_status_question("hi"));
    assert!(!is_capability_status_question("List my active projects"));
}

#[test]
fn a_document_original_is_only_asked_for_by_a_digest() {
    let id = "a".repeat(64);
    assert_eq!(
        document_original_path(&id).as_deref(),
        Some(format!("/documents/api/documents/{id}/original").as_str())
    );
    assert!(document_original_path("../etc/passwd").is_none());
    assert!(document_original_path(&"a".repeat(63)).is_none());
    assert!(document_original_path(&"a".repeat(65)).is_none());
    assert!(document_original_path(&"g".repeat(64)).is_none());
    assert!(document_original_path("").is_none());
}

#[test]
fn document_detail_decodes_separate_proposal_evidence_and_review_history() {
    let document_id = "a".repeat(64);
    let proposal_id = "b".repeat(64);
    let detail: DocumentDetail = serde_json::from_value(serde_json::json!({
        "document": {
            "id": document_id,
            "filename": "invoice.txt",
            "media_type": "text/plain",
            "byte_len": 10,
            "imported_at": "2026-10-22T10:00:00Z",
            "status": "inbox",
            "category": null,
            "project": null,
            "extracted_title": null,
            "extractor": "plain-text",
            "extracted_chars": 7,
            "suggested_category": "Finance",
            "suggestion_reason": "local rule",
            "data_class": "c2"
        },
        "text": "invoice",
        "proposals": [{
            "id": proposal_id,
            "generation_id": "generation-one",
            "generated_at": "2026-10-22T10:01:00Z",
            "proposal": {
                "document_id": document_id,
                "source": {"kind": "rules", "identity": "finance", "version": "v1"},
                "outcome": {
                    "status": "proposed",
                    "category": "Finance",
                    "evidence": {"source": "extracted_text", "quote": "invoice"}
                },
                "confidence": null,
                "review": "accepted"
            },
            "review_history": [{
                "id": 1,
                "outcome": "accepted",
                "reviewed_at": "2026-10-22T10:02:00Z"
            }]
        }]
    }))
    .unwrap();
    assert_eq!(detail.proposals.len(), 1);
    assert_eq!(detail.proposals[0].id, proposal_id);
    assert_eq!(
        detail.proposals[0].proposal.review,
        Some(ProposalReview::Accepted)
    );
    assert_eq!(detail.proposals[0].review_history.len(), 1);
    assert_eq!(
        detail.proposals[0].proposal.outcome,
        ProposalOutcome::Proposed {
            category: "Finance".into(),
            evidence: ProposalEvidence {
                source: ProposalEvidenceSource::ExtractedText,
                quote: "invoice".into(),
            },
        }
    );
}

#[test]
fn endpoint_must_be_loopback_http() {
    assert!(Shell::new("http://127.0.0.1:8082").is_ok());
    assert!(Shell::new("http://localhost:9000/").is_ok());
    assert_eq!(
        Shell::new("https://127.0.0.1:8082").err(),
        Some(ShellError::NotLoopback)
    );
    assert_eq!(
        Shell::new("http://example.com:8082").err(),
        Some(ShellError::NotLoopback)
    );
    assert_eq!(
        Shell::new("http://127.0.0.1.evil:8082").err(),
        Some(ShellError::NotLoopback)
    );
}

#[test]
fn stopped_on_demand_capability_is_idle_not_down() {
    let health: NodeHealth = serde_json::from_str(
        r#"{"ok":true,"version":"v1","uptime_seconds":5,"capabilities":{
                "a":{"up":true,"autostart":true},
                "b":{"up":false,"autostart":false},
                "c":{"up":false}
            }}"#,
    )
    .unwrap();
    assert_eq!(health.capabilities["b"].liveness(), Liveness::Idle);
    assert_eq!(health.down(), vec!["c".to_owned()]);
}

#[test]
fn registry_names_reject_paths() {
    assert!(is_registry_name("sjel-status"));
    assert!(!is_registry_name("../x"));
    assert!(!is_registry_name(""));
}

#[test]
fn a_query_encodes_what_a_project_name_can_hold() {
    assert_eq!(
        with_query(
            "/projects/api/project",
            &[("name", "Home Lab"), ("content", "false")]
        ),
        "/projects/api/project?name=Home+Lab&content=false"
    );
    assert_eq!(
        with_query("/projects/api/record", &[("path", "A/B & C.md")]),
        "/projects/api/record?path=A%2FB+%26+C.md"
    );
}

#[test]
fn a_list_read_without_content_still_parses() {
    let record: ProjectRecord = serde_json::from_str(
        r#"{"path":"P/a.md","kind":"note","title":"A","done":false,
                "due":null,"priority":2}"#,
    )
    .unwrap();
    assert_eq!(record.content, "");
    assert_eq!(record.title, "A");
}

#[test]
fn a_task_reply_carries_the_path_as_its_id() {
    let task: GlobalTask = serde_json::from_str(
        r#"{"id":"P/t.md","kind":"task","title":"T","done":true,
                "due":"2026-10-24","priority":1,"project":"P"}"#,
    )
    .unwrap();
    assert_eq!(task.id, "P/t.md");
    assert!(task.done);
}

#[test]
fn cloud_ask_rejects_history_before_any_request() {
    let shell = Shell::new("http://127.0.0.1:1").unwrap();
    let history = [AskTurn {
        role: "user".into(),
        content: "local conversation".into(),
    }];

    let error = futures::executor::block_on(shell.ask_stream_with_provider(
        "public prompt",
        &history,
        Some("cloud_assistant_demo"),
        true,
    ))
    .unwrap_err();

    assert!(
        matches!(error, ShellError::Refused(409, Some(reason)) if reason.contains("does not send local conversation history"))
    );
}

#[test]
fn frames_come_out_whole_and_leave_the_rest() {
    let mut buffer =
        b"event: snapshot\ndata: {\"agent\":1}\n\n: keep-alive\n\nevent: snapshot\ndata: {\"hea"
            .to_vec();
    assert_eq!(next_frame(&mut buffer).as_deref(), Some("{\"agent\":1}"));
    // A comment carries no data and is not a snapshot.
    assert_eq!(next_frame(&mut buffer).as_deref(), Some(""));
    // The third event is still split across chunks.
    assert_eq!(next_frame(&mut buffer), None);
    buffer.extend_from_slice(b"lth\":2}\n\n");
    assert_eq!(next_frame(&mut buffer).as_deref(), Some("{\"health\":2}"));
}

#[test]
fn a_frame_carries_both_answers_or_why_not() {
    let both: Snapshot = serde_json::from_str(
        r#"{"health":{"version":"v","uptime_seconds":1,"capabilities":{}},
                "agent":{"pending":[],"calls":[]}}"#,
    )
    .unwrap();
    assert!(both.health.is_some() && both.agent.is_some());
    assert!(both.error.is_none());

    let failed: Snapshot = serde_json::from_str(r#"{"error":"agent policy: x"}"#).unwrap();
    assert_eq!(failed.error.as_deref(), Some("agent policy: x"));
    assert!(failed.health.is_none());
}

#[test]
fn an_ask_event_preserves_live_tool_activity() {
    let event: AskEvent = serde_json::from_str(
        r#"{"type":"tool_call","name":"calendar_entries","arguments":"{\"from\":\"2026-10-21\"}"}"#,
    )
    .unwrap();
    assert_eq!(event.kind, "tool_call");
    assert_eq!(event.name, "calendar_entries");
    assert!(event.arguments.contains("2026-10-21"));
}
