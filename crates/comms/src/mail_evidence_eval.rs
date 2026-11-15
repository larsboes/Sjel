//! Read-only evaluation of frozen labels against transient source text.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::mail_evidence::{self, Parsed, Stage};

/// An independently reviewed claim, with equivalent source spellings of its item mention.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Label {
    pub event: String,
    pub titles: Vec<String>,
    pub project: Option<String>,
    pub evidence: String,
}

/// Mail text supplied on stdin; never serialized into a corpus or report.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub source_id: String,
    pub data_class: String,
    pub stage: Stage,
    #[serde(default)]
    pub projects: Vec<String>,
    pub mail: String,
}

/// Labels and sources supplied before any model output is inspected.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub reviewed_by: String,
    pub reviewed_at: String,
    pub representation: String,
    pub cases: Vec<ReviewedCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
/// One reviewed source with its expected claims; an empty list is a negative case.
pub struct ReviewedCase {
    pub source: Source,
    pub expected: Vec<Label>,
}

/// Frozen labels and input fingerprints, without message bodies.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Corpus {
    pub schema_version: String,
    pub reviewed_by: String,
    pub reviewed_at: String,
    pub representation: String,
    pub cases: Vec<Case>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
/// A retained source reference, fingerprint and reviewed labels.
pub struct Case {
    pub id: String,
    pub source_id: String,
    pub stage: Stage,
    pub input_hash: String,
    pub expected: Vec<Label>,
}

fn key(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn digest(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hash(source: &Source) -> Result<String, &'static str> {
    mail_evidence::prompt(
        source.stage,
        &source.data_class,
        &source.mail,
        &source.projects,
    )
    .map_err(|_| "source violates prompt policy")?;
    let input = serde_json::json!({
        "source_id":source.source_id, "class":source.data_class, "stage":source.stage,
        "projects":source.projects, "mail":source.mail
    });
    Ok(digest(&input.to_string()))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// Freeze reviewed labels before model evaluation.
///
/// # Errors
/// Refuses invalid sources, duplicate cases, ungrounded labels and unsupported events.
pub fn freeze(review: Review) -> Result<Corpus, &'static str> {
    if review.reviewed_by.trim().is_empty()
        || review.reviewed_at.trim().is_empty()
        || review.representation.trim().is_empty()
        || review.cases.is_empty()
        || review.cases.len() > 50
    {
        return Err("review metadata or case count is invalid");
    }
    let mut ids = HashSet::new();
    let mut sources = HashSet::new();
    let mut cases = Vec::with_capacity(review.cases.len());
    for case in review.cases {
        let s = &case.source;
        if s.source_id.is_empty()
            || s.source_id.chars().any(char::is_control)
            || s.source_id.len() > 256
            || case.expected.len() > mail_evidence::MAX_PROPOSALS
            || !valid_id(&s.id)
            || !ids.insert(s.id.clone())
            || !sources.insert((s.source_id.clone(), s.stage as u8))
        {
            return Err("duplicate or invalid case identity");
        }
        let mut labels = HashSet::new();
        for label in &case.expected {
            let valid_event = match s.stage {
                Stage::Inventory => {
                    serde_json::from_value::<inventory_api::mail_evidence::PurchaseEvent>(
                        serde_json::Value::String(label.event.clone()),
                    )
                    .is_ok()
                        && label.project.is_none()
                }
                Stage::Projects => {
                    label.event == "project_evidence"
                        && label
                            .project
                            .as_ref()
                            .is_some_and(|p| s.projects.contains(p))
                }
            };
            if !valid_event
                || label.evidence.trim().is_empty()
                || label.evidence.len() > 1024
                || !key(&s.mail).contains(&key(&label.evidence))
                || label.titles.is_empty()
                || label.titles.len() > 16
                || label.titles.iter().any(|t| {
                    t.trim().is_empty() || t.len() > 256 || !key(&s.mail).contains(&key(t))
                })
                || !labels.insert((
                    label.event.clone(),
                    key(&label.titles[0]),
                    label.project.clone(),
                ))
            {
                return Err("invalid or ungrounded review label");
            }
        }
        cases.push(Case {
            id: s.id.clone(),
            source_id: s.source_id.clone(),
            stage: s.stage,
            input_hash: hash(s)?,
            expected: case.expected,
        });
    }
    Ok(Corpus {
        schema_version: "mail-evidence-corpus-v1".into(),
        reviewed_by: review.reviewed_by,
        reviewed_at: review.reviewed_at,
        representation: review.representation,
        cases,
    })
}

/// Separate error counts; source text and predicted labels never enter the report.
#[derive(Default, Debug, Serialize)]
pub struct Counts {
    pub scored: usize,
    pub expected_claims: usize,
    pub expected_project_claims: usize,
    pub correct_claims: usize,
    pub false_claims: usize,
    pub missed_claims: usize,
    pub incorrect_targets: usize,
    pub missed_item_mentions: usize,
    pub duplicate_predictions: usize,
    pub rejected_rows: BTreeMap<String, usize>,
}

/// Compare accepted claims separately from item mentions and project assignments.
pub fn score(labels: &[Label], parsed: &Parsed) -> Counts {
    let mut result = Counts {
        scored: 1,
        expected_claims: labels.len(),
        expected_project_claims: labels.iter().filter(|l| l.project.is_some()).count(),
        duplicate_predictions: parsed.duplicates,
        ..Counts::default()
    };
    let mut matched = HashSet::new();
    let mut mentions = HashSet::new();
    let mut predictions = HashSet::new();
    for rule in &parsed.dropped {
        *result.rejected_rows.entry((*rule).into()).or_default() += 1;
        if *rule == "unknown_project" {
            result.incorrect_targets += 1;
        }
    }
    for proposal in &parsed.proposals {
        let (event, title, project) = match proposal {
            mail_evidence::Proposal::Inventory(p) => (p.event.as_str(), p.title.as_str(), None),
            mail_evidence::Proposal::Projects(p) => {
                (p.event.as_str(), p.title.as_str(), Some(p.project.as_str()))
            }
        };
        let title = key(title);
        let candidates: Vec<_> = labels
            .iter()
            .enumerate()
            .filter(|(_, l)| l.titles.iter().any(|t| key(t) == title))
            .collect();
        for (i, _) in &candidates {
            mentions.insert(*i);
        }
        let exact = candidates.iter().find(|(i, l)| {
            !matched.contains(i) && l.event == event && l.project.as_deref() == project
        });
        if let Some((i, _)) = exact {
            matched.insert(*i);
            result.correct_claims += 1;
        } else {
            result.false_claims += 1;
            if candidates
                .iter()
                .any(|(_, l)| l.event == event && l.project.as_deref() != project)
            {
                result.incorrect_targets += 1;
            }
        }
        let canonical = candidates
            .first()
            .map(|(i, _)| i.to_string())
            .unwrap_or(title);
        if !predictions.insert((event, canonical, project)) {
            result.duplicate_predictions += 1;
        }
    }
    result.missed_claims = labels.len() - matched.len();
    result.missed_item_mentions = labels.len() - mentions.len();
    result
}

/// Model provenance and per-case counts for one read-only run.
#[derive(Serialize)]
pub struct Report {
    pub started_unix_seconds: u64,
    pub corpus_hash: String,
    pub role: String,
    pub extraction_version: &'static str,
    pub backend: String,
    pub model: String,
    pub conventions_hash: String,
    pub initial_admission_failure: Option<String>,
    pub cases: BTreeMap<String, Counts>,
    pub failures: BTreeMap<String, String>,
}

/// Evaluate a frozen corpus with transient sources using the production inference path.
///
/// # Errors
/// Refuses unreviewed schemas, missing/changed sources or an unavailable local role.
pub fn run(
    corpus: &Corpus,
    sources: &[Source],
    cfg: &crate::config::Config,
    runtime: &sjel_runtime::Status,
    role_name: &str,
) -> Result<Report, &'static str> {
    if corpus.schema_version != "mail-evidence-corpus-v1"
        || corpus.reviewed_by.trim().is_empty()
        || corpus.cases.is_empty()
        || corpus.cases.len() > 50
        || sources.len() != corpus.cases.len()
    {
        return Err("invalid corpus or source count");
    }
    let mut seen = HashSet::new();
    for case in &corpus.cases {
        if !valid_id(&case.id) || !seen.insert(&case.id) {
            return Err("duplicate or invalid case identity");
        }
        let source = sources
            .iter()
            .find(|s| s.id == case.id)
            .ok_or("missing source")?;
        if source.source_id != case.source_id
            || source.stage != case.stage
            || hash(source)? != case.input_hash
        {
            return Err("source changed since review");
        }
    }
    freeze(Review {
        reviewed_by: corpus.reviewed_by.clone(),
        reviewed_at: corpus.reviewed_at.clone(),
        representation: corpus.representation.clone(),
        cases: corpus
            .cases
            .iter()
            .map(|c| {
                sources
                    .iter()
                    .find(|s| s.id == c.id)
                    .cloned()
                    .map(|source| ReviewedCase {
                        source,
                        expected: c.expected.clone(),
                    })
                    .ok_or("missing source")
            })
            .collect::<Result<Vec<_>, _>>()?,
    })?;
    let role = cfg
        .inference
        .role(role_name)
        .filter(|r| r.is_loopback() && r.max_input_tokens.is_some())
        .ok_or("local role unavailable")?;
    let conventions = serde_json::json!({"chat_template_kwargs":role.chat_template_kwargs,"request_overrides":role.request_overrides,"max_input_tokens":role.max_input_tokens});
    let mut report = Report {
        corpus_hash: digest(&serde_json::to_string(corpus).map_err(|_| "invalid corpus")?),
        role: role_name.into(),
        started_unix_seconds: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "clock unavailable")?
            .as_secs(),
        extraction_version: mail_evidence::EXTRACTION_VERSION,
        backend: role.backend_name.clone(),
        model: role.model.clone(),
        conventions_hash: digest(&conventions.to_string()),
        initial_admission_failure: role
            .runtime_admission_with(runtime)
            .err()
            .map(|e| e.to_string()),
        cases: BTreeMap::new(),
        failures: BTreeMap::new(),
    };
    for case in &corpus.cases {
        let source = sources
            .iter()
            .find(|s| s.id == case.id)
            .ok_or("missing source")?;
        let extraction = crate::mail_evidence_scan::Extraction {
            stage: source.stage,
            class: &source.data_class,
            text: &source.mail,
            source: &source.source_id,
            show: false,
        };
        match crate::mail_evidence_scan::model(runtime, cfg, &role, &extraction, &source.projects) {
            Ok(parsed) => {
                report
                    .cases
                    .insert(case.id.clone(), score(&case.expected, &parsed));
            }
            Err(code) => {
                report.failures.insert(case.id.clone(), code.into());
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Source {
        Source {
            id: "case-1".into(),
            source_id: "mail:v1:thread:message".into(),
            data_class: "c1".into(),
            stage: Stage::Inventory,
            projects: vec![],
            mail: "Order 123: Blue mug".into(),
        }
    }

    fn label() -> Label {
        Label {
            event: "order".into(),
            titles: vec!["Blue mug".into()],
            project: None,
            evidence: "Order 123: Blue mug".into(),
        }
    }

    fn review(source: Source, expected: Vec<Label>) -> Review {
        Review {
            reviewed_by: "agent-review".into(),
            reviewed_at: "2026-11-15".into(),
            representation: "synthetic".into(),
            cases: vec![ReviewedCase { source, expected }],
        }
    }

    #[test]
    fn frozen_corpus_retains_labels_but_never_the_mail_body() {
        let mut source = source();
        source.mail.push_str("\nPrivate footer omitted from corpus");
        let corpus = freeze(review(source, vec![label()])).unwrap();
        let json = serde_json::to_string(&corpus).unwrap();
        assert!(!json.contains("Private footer"));
        assert!(!json.contains("\"mail\""));
        assert_eq!(corpus.cases[0].input_hash.len(), 64);
    }

    #[test]
    fn secret_unknown_and_duplicate_sources_are_refused() {
        for class in ["c3", "unknown"] {
            let mut s = source();
            s.data_class = class.into();
            assert!(freeze(review(s, vec![label()])).is_err());
        }
        let mut r = review(source(), vec![label()]);
        let mut s = source();
        s.id = "case-2".into();
        r.cases.push(ReviewedCase {
            source: s,
            expected: vec![],
        });
        assert!(freeze(r).is_err());
    }

    #[test]
    fn empty_labels_are_valid_negatives_but_invented_labels_are_not() {
        assert!(freeze(review(source(), vec![])).is_ok());
        let mut l = label();
        l.titles = vec!["Red plate".into()];
        assert!(freeze(review(source(), vec![l])).is_err());
        let mut l = label();
        l.evidence = "invented evidence".into();
        assert!(freeze(review(source(), vec![l])).is_err());
    }

    #[test]
    fn fingerprints_bind_class_projects_and_source_text() {
        let s = source();
        let original = hash(&s).unwrap();
        let mut edited = s.clone();
        edited.mail.push('!');
        assert_ne!(original, hash(&edited).unwrap());
        edited = s.clone();
        edited.projects.push("Kitchen".into());
        assert_ne!(original, hash(&edited).unwrap());
        edited = s;
        edited.data_class = "c2".into();
        assert_ne!(original, hash(&edited).unwrap());
    }

    #[test]
    fn wrong_event_is_a_false_and_missed_claim_but_the_item_was_found() {
        let parsed = mail_evidence::parse(
            Stage::Inventory,
            "s",
            "c1",
            &source().mail,
            &[],
            r#"[{"event":"shipment","title":"Blue mug","evidence":"Order 123: Blue mug"}]"#,
        )
        .unwrap();
        let counts = score(&[label()], &parsed);
        assert_eq!(
            (
                counts.false_claims,
                counts.missed_claims,
                counts.missed_item_mentions
            ),
            (1, 1, 0)
        );
    }

    #[test]
    fn incorrect_target_is_reported_separately() {
        let projects = vec!["Kitchen".into(), "Move".into()];
        let parsed = mail_evidence::parse(Stage::Projects, "s", "c1", "Blue mug", &projects, r#"[{"event":"project_evidence","project":"Move","title":"Blue mug","evidence":"Blue mug"}]"#).unwrap();
        let mut l = label();
        l.event = "project_evidence".into();
        l.project = Some("Kitchen".into());
        let c = score(&[l], &parsed);
        assert_eq!(
            (
                c.expected_project_claims,
                c.incorrect_targets,
                c.false_claims,
                c.missed_claims
            ),
            (1, 1, 1, 1)
        );
    }

    #[test]
    fn rejected_rows_and_suppressed_duplicates_remain_visible() {
        let parsed = mail_evidence::parse(Stage::Inventory, "s", "c1", &source().mail, &[], r#"[{"event":"order","title":"Blue mug","evidence":"Order 123: Blue mug"},{"event":"order","title":"Blue mug","evidence":"Order 123: Blue mug"},{"event":"order","title":"Red plate","evidence":"made up"}]"#).unwrap();
        let c = score(&[label()], &parsed);
        assert_eq!((c.correct_claims, c.duplicate_predictions), (1, 1));
        assert_eq!(c.rejected_rows["evidence_not_in_mail"], 1);
    }

    #[test]
    fn changed_source_is_refused_before_role_resolution_or_inference() {
        let source = source();
        let corpus = freeze(review(source.clone(), vec![label()])).unwrap();
        let mut changed = source;
        changed.mail.push_str(" modified");
        let mut cfg = crate::config::Config::load();
        cfg.inference = Default::default();
        let result = run(
            &corpus,
            &[changed],
            &cfg,
            &sjel_runtime::Status::normal("test"),
            "missing",
        );
        assert_eq!(result.err(), Some("source changed since review"));
    }

    #[test]
    fn failed_predictions_do_not_turn_into_perfect_recall() {
        let parsed =
            mail_evidence::parse(Stage::Inventory, "s", "c1", &source().mail, &[], "[]").unwrap();
        let c = score(&[label()], &parsed);
        assert_eq!(
            (c.correct_claims, c.missed_claims, c.missed_item_mentions),
            (0, 1, 1)
        );
    }
}
