//! Owner-side message loading for repeatable, body-free evaluation artifacts.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    config::Config,
    google,
    mail_evidence::Stage,
    mail_evidence_eval::{self as eval, Corpus, Label, Report, Review, ReviewedCase, Source},
    mail_evidence_scan as scan,
    store::Store,
};

/// Frozen source representation; session pseudonyms are never treated as native text.
pub const REPRESENTATION: &str = "native-mail-message-v1";

/// An independently reviewed message reference and expected claims.
#[derive(Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeCase {
    pub id: String,
    pub source_id: String,
    pub stage: Stage,
    pub expected: Vec<Label>,
}

/// Labels supplied before prediction; the capability fetches and classifies source text.
#[derive(Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeReview {
    pub reviewed_by: String,
    pub reviewed_at: String,
    pub cases: Vec<NativeCase>,
}

fn default_role() -> String {
    "rules".into()
}

/// Evaluate native references from a frozen corpus; rules are the model-free default.
#[derive(Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeRun {
    pub corpus: Corpus,
    #[serde(default = "default_role")]
    pub role: String,
}

#[derive(Clone, PartialEq, Eq)]
struct Context {
    class: String,
    stream: String,
}

trait Mailbox {
    fn context(&mut self, thread: &str) -> Result<Context, &'static str>;
    fn messages(&mut self, thread: &str) -> Result<Vec<google::ThreadMessageBody>, &'static str>;
    fn projects(&mut self) -> Result<Vec<String>, &'static str>;
}

struct LiveMailbox<'a> {
    cfg: &'a Config,
    store: Store,
    token: Option<String>,
}

impl<'a> LiveMailbox<'a> {
    fn new(cfg: &'a Config) -> Result<Self, &'static str> {
        Ok(Self {
            cfg,
            store: Store::open(&cfg.database_path).map_err(|_| "source_store")?,
            token: None,
        })
    }
}

impl Mailbox for LiveMailbox<'_> {
    fn context(&mut self, thread: &str) -> Result<Context, &'static str> {
        let item = self
            .store
            .get_triage(thread)
            .map_err(|_| "source_store")?
            .ok_or("source_missing")?;
        Ok(Context {
            class: item.data_class,
            stream: item.stream,
        })
    }
    fn messages(&mut self, thread: &str) -> Result<Vec<google::ThreadMessageBody>, &'static str> {
        if self.token.is_none() {
            self.token =
                Some(google::access_token(&self.cfg.google_env_path).map_err(|_| "mail_auth")?);
        }
        let token = self.token.as_deref().ok_or("mail_auth")?;
        google::thread_message_bodies(token, thread)
            .map_err(|_| "source_transport")?
            .ok_or("source_missing")
    }
    fn projects(&mut self) -> Result<Vec<String>, &'static str> {
        let base =
            scan::local_base(&self.cfg.projects_context.base_url).ok_or("project_context")?;
        let client =
            scan::client(self.cfg.projects_context.timeout_ms).map_err(|_| "project_context")?;
        scan::project_names(&client, &base).map_err(|_| "project_context")
    }
}

struct Reference {
    id: String,
    source_id: String,
    stage: Stage,
}

fn message_ref(source: &str) -> Result<(&str, &str), &'static str> {
    let (thread, message) = source
        .strip_prefix("mail:v1:")
        .and_then(|s| s.split_once(':'))
        .ok_or("invalid_message_reference")?;
    if !scan::valid_provider_id(thread) || !scan::valid_provider_id(message) {
        return Err("invalid_message_reference");
    }
    Ok((thread, message))
}

fn validate(refs: &[Reference]) -> Result<(), &'static str> {
    if refs.is_empty() || refs.len() > 50 {
        return Err("invalid_case_count");
    }
    let mut ids = HashSet::new();
    let mut sources = HashSet::new();
    for r in refs {
        message_ref(&r.source_id)?;
        if r.id.is_empty()
            || r.id.len() > 64
            || !r
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            || !ids.insert(&r.id)
            || !sources.insert((&r.source_id, r.stage == Stage::Inventory))
        {
            return Err("invalid_case_identity");
        }
    }
    Ok(())
}

struct Loaded {
    sources: Vec<Source>,
    contexts: BTreeMap<String, Context>,
}

fn unchanged(
    mailbox: &mut impl Mailbox,
    thread: &str,
    expected: &Context,
) -> Result<(), &'static str> {
    if mailbox.context(thread)? != *expected {
        return Err("source_context_changed");
    }
    Ok(())
}

fn load(mailbox: &mut impl Mailbox, refs: &[Reference]) -> Result<Loaded, &'static str> {
    validate(refs)?;
    let projects = if refs.iter().any(|r| r.stage == Stage::Projects) {
        mailbox.projects()?
    } else {
        vec![]
    };
    let mut groups: BTreeMap<&str, Vec<&Reference>> = BTreeMap::new();
    for r in refs {
        groups
            .entry(message_ref(&r.source_id)?.0)
            .or_default()
            .push(r);
    }
    let mut loaded = Loaded {
        sources: Vec::with_capacity(refs.len()),
        contexts: BTreeMap::new(),
    };
    for (thread, group) in groups {
        let context = mailbox.context(thread)?;
        if !matches!(context.class.as_str(), "c0" | "c1" | "c2") {
            return Err("source_classification");
        }
        let messages = mailbox.messages(thread)?;
        if messages.is_empty() || !scan::valid_message_ids(&messages) {
            return Err("message_identity");
        }
        let classes = messages
            .iter()
            .map(|m| scan::message_class(&context.class, &context.stream, m))
            .collect::<Option<Vec<_>>>()
            .ok_or("source_classification")?;
        if classes.contains(&"c3") {
            return Err("source_classification");
        }
        for r in group {
            let message_id = message_ref(&r.source_id)?.1;
            let index = messages
                .iter()
                .position(|m| m.id == message_id)
                .ok_or("source_missing")?;
            let message = &messages[index];
            if message.text.len() > crate::mail_evidence::MAX_MAIL_BYTES {
                return Err("source_too_large");
            }
            loaded.sources.push(Source {
                id: r.id.clone(),
                source_id: r.source_id.clone(),
                data_class: classes[index].into(),
                stage: r.stage,
                projects: if r.stage == Stage::Projects {
                    projects.clone()
                } else {
                    vec![]
                },
                mail: message.text.clone(),
            });
        }
        unchanged(mailbox, thread, &context)?;
        loaded.contexts.insert(thread.into(), context);
    }
    Ok(loaded)
}

fn recheck(mailbox: &mut impl Mailbox, loaded: &Loaded) -> Result<(), &'static str> {
    for (thread, context) in &loaded.contexts {
        unchanged(mailbox, thread, context)?;
    }
    if let Some(source) = loaded.sources.iter().find(|s| s.stage == Stage::Projects) {
        if mailbox.projects()? != source.projects {
            return Err("project_context_changed");
        }
    }
    Ok(())
}

fn freeze_with(mailbox: &mut impl Mailbox, review: NativeReview) -> Result<Corpus, &'static str> {
    let refs = review
        .cases
        .iter()
        .map(|c| Reference {
            id: c.id.clone(),
            source_id: c.source_id.clone(),
            stage: c.stage,
        })
        .collect::<Vec<_>>();
    let loaded = load(mailbox, &refs)?;
    let mut cases = Vec::with_capacity(refs.len());
    for case in review.cases {
        let source = loaded
            .sources
            .iter()
            .find(|s| s.id == case.id)
            .cloned()
            .ok_or("source_missing")?;
        cases.push(ReviewedCase {
            source,
            expected: case.expected,
        });
    }
    let corpus = eval::freeze(Review {
        reviewed_by: review.reviewed_by,
        reviewed_at: review.reviewed_at,
        representation: REPRESENTATION.into(),
        cases,
    })?;
    recheck(mailbox, &loaded)?;
    Ok(corpus)
}

fn run_with(
    mailbox: &mut impl Mailbox,
    request: NativeRun,
    cfg: &Config,
    runtime: &sjel_runtime::Status,
) -> Result<Report, &'static str> {
    if request.corpus.representation != REPRESENTATION {
        return Err("native_review_required");
    }
    if request.role != "rules"
        && cfg
            .inference
            .role(&request.role)
            .filter(|r| r.is_afm() && r.max_input_tokens.is_some())
            .is_none()
    {
        return Err("afm_config");
    }
    let refs = request
        .corpus
        .cases
        .iter()
        .map(|c| Reference {
            id: c.id.clone(),
            source_id: c.source_id.clone(),
            stage: c.stage,
        })
        .collect::<Vec<_>>();
    let loaded = load(mailbox, &refs)?;
    recheck(mailbox, &loaded)?;
    let report = eval::run_with_guard(
        &request.corpus,
        &loaded.sources,
        cfg,
        runtime,
        &request.role,
        |source| {
            let thread = message_ref(&source.source_id)?.0;
            unchanged(
                mailbox,
                thread,
                loaded.contexts.get(thread).ok_or("source_missing")?,
            )?;
            if source.stage == Stage::Projects && mailbox.projects()? != source.projects {
                return Err("project_context_changed");
            }
            Ok(())
        },
    )?;
    recheck(mailbox, &loaded)?;
    Ok(report)
}

/// Freeze reviewed labels against freshly classified native messages without storing bodies.
///
/// # Errors
/// Refuses invalid references, unavailable sources, classification changes and ungrounded labels.
pub fn freeze(cfg: &Config, review: NativeReview) -> Result<Corpus, &'static str> {
    validate(
        &review
            .cases
            .iter()
            .map(|c| Reference {
                id: c.id.clone(),
                source_id: c.source_id.clone(),
                stage: c.stage,
            })
            .collect::<Vec<_>>(),
    )?;
    freeze_with(&mut LiveMailbox::new(cfg)?, review)
}

/// Fetch and revalidate a native corpus, then evaluate without staging any owner changes.
///
/// # Errors
/// Refuses non-native reviews, changed or disallowed sources, unavailable AFM and stale contexts.
pub fn run(
    cfg: &Config,
    runtime: &sjel_runtime::Status,
    request: NativeRun,
) -> Result<Report, &'static str> {
    if request.corpus.representation != REPRESENTATION {
        return Err("native_review_required");
    }
    run_with(&mut LiveMailbox::new(cfg)?, request, cfg, runtime)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = "Thank you for your purchase!\nOrder summary\nBlue mug × 1\n$12.00\nSubtotal\nUnreviewed footer sentinel";

    struct MockMailbox {
        class: String,
        messages: Vec<google::ThreadMessageBody>,
        reads: usize,
        context_reads: usize,
        change_at: usize,
        project_reads: usize,
        change_projects_at: usize,
    }

    fn message(id: &str, body: &str) -> google::ThreadMessageBody {
        google::ThreadMessageBody {
            id: id.into(),
            from_addr: Some("shop@example.test".into()),
            subject: Some("Order confirmation".into()),
            snippet: Some("Blue mug".into()),
            text: body.into(),
        }
    }

    fn fixture() -> MockMailbox {
        MockMailbox {
            class: "c1".into(),
            messages: vec![message("message", BODY)],
            reads: 0,
            context_reads: 0,
            change_at: usize::MAX,
            project_reads: 0,
            change_projects_at: usize::MAX,
        }
    }

    impl Mailbox for MockMailbox {
        fn context(&mut self, _: &str) -> Result<Context, &'static str> {
            self.context_reads += 1;
            Ok(Context {
                class: self.class.clone(),
                stream: if self.context_reads >= self.change_at {
                    "changed"
                } else {
                    "sonstiges"
                }
                .into(),
            })
        }
        fn messages(&mut self, _: &str) -> Result<Vec<google::ThreadMessageBody>, &'static str> {
            self.reads += 1;
            Ok(std::mem::take(&mut self.messages))
        }
        fn projects(&mut self) -> Result<Vec<String>, &'static str> {
            self.project_reads += 1;
            Ok(vec![if self.project_reads >= self.change_projects_at {
                "Changed"
            } else {
                "Kitchen"
            }
            .into()])
        }
    }

    fn review() -> NativeReview {
        NativeReview {
            reviewed_by: "synthetic-test".into(),
            reviewed_at: "2026-11-15".into(),
            cases: vec![NativeCase {
                id: "case-1".into(),
                source_id: "mail:v1:thread:message".into(),
                stage: Stage::Inventory,
                expected: vec![Label {
                    event: "order".into(),
                    titles: vec!["Blue mug".into()],
                    project: None,
                    evidence: "Blue mug × 1".into(),
                }],
            }],
        }
    }

    fn evaluate(mailbox: &mut MockMailbox, corpus: Corpus) -> Result<Report, &'static str> {
        let mut cfg = Config::load();
        cfg.inference = Default::default();
        run_with(
            mailbox,
            NativeRun {
                corpus,
                role: "rules".into(),
            },
            &cfg,
            &sjel_runtime::Status::unresolved("no runtime required"),
        )
    }

    #[test]
    fn separate_reads_repeat_without_bodies_models_or_runtime() {
        let corpus = freeze_with(&mut fixture(), review()).unwrap();
        let frozen = serde_json::to_string(&corpus).unwrap();
        assert!(!frozen.contains("Unreviewed footer sentinel"));
        assert!(!frozen.contains("Thank you for your purchase!"));
        let report = evaluate(&mut fixture(), corpus).unwrap();
        assert_eq!(report.model, "none");
        assert!(report.failures.is_empty());
        assert_eq!(report.cases["case-1"].correct_claims, 1);
        assert!(!serde_json::to_string(&report).unwrap().contains("Blue mug"));
        let repeated = freeze_with(&mut fixture(), review()).unwrap();
        assert_eq!(frozen, serde_json::to_string(&repeated).unwrap());
    }

    #[test]
    fn grouped_messages_fetch_thread_once_and_keep_exact_identity() {
        let mut mailbox = fixture();
        mailbox.messages.push(message("second", "No purchase here"));
        let mut review = review();
        review.cases.push(NativeCase {
            id: "case-2".into(),
            source_id: "mail:v1:thread:second".into(),
            stage: Stage::Inventory,
            expected: vec![],
        });
        let corpus = freeze_with(&mut mailbox, review).unwrap();
        assert_eq!(mailbox.reads, 1);
        assert_ne!(corpus.cases[0].input_hash, corpus.cases[1].input_hash);
    }

    #[test]
    fn restricted_or_unknown_thread_never_fetches_bodies() {
        for class in ["c3", "unknown", ""] {
            let mut mailbox = fixture();
            mailbox.class = class.into();
            assert_eq!(
                freeze_with(&mut mailbox, review()).err(),
                Some("source_classification")
            );
            assert_eq!(mailbox.reads, 0);
        }
    }

    #[test]
    fn restricted_unselected_body_and_missing_metadata_refuse_entire_thread() {
        let mut mailbox = fixture();
        mailbox
            .messages
            .push(message("other", "Your verification code is 123456"));
        assert_eq!(
            freeze_with(&mut mailbox, review()).err(),
            Some("source_classification")
        );
        let mut mailbox = fixture();
        mailbox.messages[0].from_addr = None;
        assert_eq!(
            freeze_with(&mut mailbox, review()).err(),
            Some("source_classification")
        );
    }

    #[test]
    fn invalid_missing_and_duplicate_message_identities_are_refused() {
        for source in [
            "mail-thread:thread",
            "mail:v1:thread:message:extra",
            "mail:v1:../thread:message",
        ] {
            let mut mailbox = fixture();
            let mut review = review();
            review.cases[0].source_id = source.into();
            assert_eq!(
                freeze_with(&mut mailbox, review).err(),
                Some("invalid_message_reference")
            );
            assert_eq!(mailbox.reads, 0);
        }
        let mut mailbox = fixture();
        mailbox.messages[0].id = "different".into();
        assert_eq!(
            freeze_with(&mut mailbox, review()).err(),
            Some("source_missing")
        );
        let mut mailbox = fixture();
        mailbox.messages.push(message("message", BODY));
        assert_eq!(
            freeze_with(&mut mailbox, review()).err(),
            Some("message_identity")
        );
    }

    #[test]
    fn source_edits_and_old_representations_cannot_be_scored() {
        let corpus = freeze_with(&mut fixture(), review()).unwrap();
        let mut mailbox = fixture();
        mailbox.messages[0].text.push_str(" changed");
        assert_eq!(
            evaluate(&mut mailbox, corpus).err(),
            Some("source changed since review")
        );
        let mut corpus = freeze_with(&mut fixture(), review()).unwrap();
        corpus.representation = "pseudonymized-thread".into();
        let mut mailbox = fixture();
        assert_eq!(
            evaluate(&mut mailbox, corpus).err(),
            Some("native_review_required")
        );
        assert_eq!(mailbox.reads, 0);
    }

    #[test]
    fn context_drift_during_read_dispatch_or_final_check_refuses_report() {
        for change_at in [2, 3, 4, 5] {
            let corpus = freeze_with(&mut fixture(), review()).unwrap();
            let mut mailbox = fixture();
            mailbox.change_at = change_at;
            assert_eq!(
                evaluate(&mut mailbox, corpus).err(),
                Some("source_context_changed")
            );
        }
    }

    #[test]
    fn project_candidates_are_revalidated_and_rules_still_abstain() {
        let mut review = review();
        review.cases[0].stage = Stage::Projects;
        review.cases[0].expected.clear();
        let frozen = freeze_with(&mut fixture(), review).unwrap();
        let bytes = serde_json::to_vec(&frozen).unwrap();
        for change_at in [2, 3, 4] {
            let mut mailbox = fixture();
            mailbox.change_projects_at = change_at;
            assert_eq!(
                evaluate(&mut mailbox, serde_json::from_slice(&bytes).unwrap()).err(),
                Some("project_context_changed")
            );
        }
        let report = evaluate(&mut fixture(), frozen).unwrap();
        assert!(report.cases["case-1"].abstained);
    }

    #[test]
    fn defaults_to_rules_and_refuses_unavailable_model_before_fetch() {
        let corpus = freeze_with(&mut fixture(), review()).unwrap();
        let mut request: NativeRun =
            serde_json::from_value(serde_json::json!({"corpus":corpus})).unwrap();
        assert_eq!(request.role, "rules");
        request.role = "missing".into();
        let mut mailbox = fixture();
        let mut cfg = Config::load();
        cfg.inference = Default::default();
        assert_eq!(
            run_with(
                &mut mailbox,
                request,
                &cfg,
                &sjel_runtime::Status::normal("test")
            )
            .err(),
            Some("afm_config")
        );
        assert_eq!(mailbox.reads, 0);
    }

    #[test]
    fn oversized_sources_and_duplicate_cases_are_refused() {
        let mut mailbox = fixture();
        mailbox.messages[0].text = "x".repeat(crate::mail_evidence::MAX_MAIL_BYTES + 1);
        assert_eq!(
            freeze_with(&mut mailbox, review()).err(),
            Some("source_too_large")
        );
        let mut review = review();
        review.cases.push(NativeCase {
            id: "case-2".into(),
            source_id: review.cases[0].source_id.clone(),
            stage: Stage::Inventory,
            expected: vec![],
        });
        let mut mailbox = fixture();
        assert_eq!(
            freeze_with(&mut mailbox, review).err(),
            Some("invalid_case_identity")
        );
        assert_eq!(mailbox.reads, 0);
    }
}
