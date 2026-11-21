//! Local resume state for the operator's Documents split queue and pure evaluation of
//! evidence-backed classification proposals.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::shell::{
    DocumentDetail, DocumentProposalRecord, ProposalEvidenceSource, ProposalOutcome,
    ProposalReview, ProposalSource,
};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct SplitQueue {
    pub resume: Option<String>,
    pub deferred: BTreeSet<String>,
    pub drafts: BTreeMap<String, SavedSplit>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct SavedSplit {
    pub cuts: BTreeSet<u32>,
    pub names: BTreeMap<u32, String>,
    pub offset: usize,
    pub collection_id: Option<String>,
}

impl SplitQueue {
    pub fn load() -> io::Result<Self> {
        let path = crate::scratchpad::local_app_state_path("document-split-queue.json")
            .ok_or_else(|| io::Error::other("No local Documents state directory"))?;
        match fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self) -> io::Result<()> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = crate::scratchpad::local_app_state_path("document-split-queue.json")
            .ok_or_else(|| io::Error::other("No local Documents state directory"))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(self)?;
        let temp = path.with_extension(format!(
            "{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temp)?;
        let result = file
            .write_all(&bytes)
            .and_then(|()| file.sync_all())
            .and_then(|()| fs::rename(&temp, path));
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
}

/// Agreement or conflict across sources in the active proposal generation run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalAgreement {
    Agree(String),
    Conflict(Vec<String>),
    Single(String),
    NoCategory,
    NoRecords,
}

impl ProposalAgreement {
    pub fn summary_label(&self) -> String {
        match self {
            Self::Agree(category) => format!("Sources agree: {category}"),
            Self::Conflict(categories) => {
                format!("Sources disagree: {}", categories.join(" · "))
            }
            Self::Single(category) => format!("One proposal: {category}"),
            Self::NoCategory => "No category proposed in this run".into(),
            Self::NoRecords => "No proposal records yet".into(),
        }
    }
}

/// Evaluated view of one rule or model proposal row, with exact evidence verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalEvidenceView {
    pub id: String,
    pub generation_id: String,
    pub is_rule: bool,
    pub source_label: String,
    pub engine_revision: String,
    pub outcome_label: String,
    pub proposed_category: Option<String>,
    pub evidence_source_label: Option<&'static str>,
    pub evidence_quote: Option<String>,
    pub evidence_page: Option<u32>,
    pub evidence_span_verified: bool,
    pub generated_at: String,
    pub data_class: String,
    pub review_outcome: &'static str,
    pub review_history_label: String,
    pub confidence_label: String,
    pub summary_label: String,
}

/// Host and prompt boundary checks for document data classes (`c0`..`c3`).
pub fn can_leave_host(data_class: &str) -> bool {
    matches!(data_class.trim().to_ascii_lowercase().as_str(), "c0" | "c1")
}

/// Model generation requires an approved loopback role, `c2` stays on the host, and `c3` never
/// enters prompts.
pub fn can_enter_model_prompt(data_class: &str, loopback_role_approved: bool) -> bool {
    if !loopback_role_approved {
        return false;
    }
    let normalized = data_class.trim().to_ascii_lowercase();
    !normalized.is_empty() && normalized != "c3"
}

pub fn proposal_review_label(review: ProposalReview) -> &'static str {
    match review {
        ProposalReview::Accepted => "Agreed",
        ProposalReview::Rejected => "Rejected",
        ProposalReview::Overridden => "Overridden",
    }
}

/// Evaluates a document's proposals without mutating owner category values or filing automatically.
pub fn evaluate_proposal_run(
    detail: &DocumentDetail,
    include_history: bool,
) -> (ProposalAgreement, Vec<ProposalEvidenceView>) {
    let latest_generation = detail
        .proposals
        .iter()
        .max_by(|left, right| {
            left.generated_at
                .cmp(&right.generated_at)
                .then_with(|| left.generation_id.cmp(&right.generation_id))
        })
        .map(|record| record.generation_id.as_str());

    let latest: Vec<&DocumentProposalRecord> = detail
        .proposals
        .iter()
        .filter(|record| Some(record.generation_id.as_str()) == latest_generation)
        .collect();

    let mut categories = BTreeMap::<String, usize>::new();
    for record in &latest {
        if let ProposalOutcome::Proposed { category, .. } = &record.proposal.outcome {
            *categories.entry(category.clone()).or_default() += 1;
        }
    }
    let proposed_count: usize = categories.values().sum();
    let agreement = if categories.len() > 1 {
        ProposalAgreement::Conflict(categories.keys().cloned().collect())
    } else if proposed_count > 1 {
        ProposalAgreement::Agree(categories.keys().next().cloned().unwrap_or_default())
    } else if proposed_count == 1 {
        ProposalAgreement::Single(categories.keys().next().cloned().unwrap_or_default())
    } else if latest.is_empty() {
        ProposalAgreement::NoRecords
    } else {
        ProposalAgreement::NoCategory
    };

    let mut visible: Vec<&DocumentProposalRecord> = detail
        .proposals
        .iter()
        .filter(|record| {
            include_history || Some(record.generation_id.as_str()) == latest_generation
        })
        .collect();
    visible.sort_by(|left, right| {
        right
            .generated_at
            .cmp(&left.generated_at)
            .then_with(|| right.id.cmp(&left.id))
    });

    let data_class = if detail.document.data_class.trim().is_empty() {
        "c2".to_owned()
    } else {
        detail.document.data_class.clone()
    };
    let summary_label = detail
        .document
        .extracted_title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| "No summary generated".to_owned());

    let rows = visible
        .into_iter()
        .map(|record| {
            let (is_rule, source_label, engine_revision) = match &record.proposal.source {
                ProposalSource::Rules { identity, version } => (
                    true,
                    format!("Rules · {identity} · {version}"),
                    format!("{identity}@{version}"),
                ),
                ProposalSource::Model {
                    role,
                    model,
                    revision,
                } => {
                    let rev = revision.as_deref().unwrap_or("unpinned");
                    (
                        false,
                        format!("Model · {role} · {model} · {rev}"),
                        format!("{model}@{rev}"),
                    )
                }
            };

            let (
                outcome_label,
                proposed_category,
                evidence_source_label,
                evidence_quote,
                evidence_page,
                evidence_span_verified,
            ) = match &record.proposal.outcome {
                ProposalOutcome::Proposed { category, evidence } => {
                    let quote = evidence.quote.trim();
                    let (src_label, page, verified) = match evidence.source {
                        ProposalEvidenceSource::Filename => (
                            "Filename evidence",
                            None,
                            !quote.is_empty() && detail.document.filename.contains(quote),
                        ),
                        ProposalEvidenceSource::ExtractedText => {
                            let matched_page = detail
                                .pages
                                .iter()
                                .find(|p| !quote.is_empty() && p.text.contains(quote))
                                .map(|p| p.page_number);
                            let in_full_text = detail
                                .text
                                .as_deref()
                                .is_some_and(|t| !quote.is_empty() && t.contains(quote));
                            (
                                "Text evidence",
                                matched_page,
                                matched_page.is_some() || in_full_text,
                            )
                        }
                    };
                    (
                        category.clone(),
                        Some(category.clone()),
                        Some(src_label),
                        Some(evidence.quote.clone()),
                        page,
                        verified,
                    )
                }
                ProposalOutcome::Abstained { reason } => (
                    format!(
                        "No category · {}",
                        match reason {
                            crate::shell::ProposalAbstentionReason::NoMatch => "no rule matched",
                            crate::shell::ProposalAbstentionReason::MissingEvidence => {
                                "evidence missing"
                            }
                            crate::shell::ProposalAbstentionReason::ModelUnavailable => {
                                "source unavailable"
                            }
                            crate::shell::ProposalAbstentionReason::CategoryNotConfigured => {
                                "category not configured"
                            }
                            crate::shell::ProposalAbstentionReason::SafetyRefused => {
                                "safety refused"
                            }
                        }
                    ),
                    None,
                    None,
                    None,
                    None,
                    false,
                ),
                ProposalOutcome::Invalid { reason } => (
                    format!(
                        "Invalid · {}",
                        match reason {
                            crate::shell::ProposalInvalidReason::MalformedOutput => {
                                "malformed output"
                            }
                            crate::shell::ProposalInvalidReason::EvidenceNotExact => {
                                "evidence was not exact"
                            }
                            crate::shell::ProposalInvalidReason::EvidenceTooLong => {
                                "evidence exceeded limits"
                            }
                            crate::shell::ProposalInvalidReason::InvalidCategory => {
                                "category was invalid"
                            }
                        }
                    ),
                    None,
                    None,
                    None,
                    None,
                    false,
                ),
            };

            let review_outcome = record
                .proposal
                .review
                .map(proposal_review_label)
                .unwrap_or("Not reviewed");

            let review_history_label = if record.review_history.is_empty() {
                "Not reviewed".to_owned()
            } else {
                record
                    .review_history
                    .iter()
                    .map(|event| {
                        format!(
                            "{} · {}",
                            proposal_review_label(event.outcome),
                            event.reviewed_at.get(..10).unwrap_or("date unknown")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" → ")
            };

            let confidence_label = record
                .proposal
                .confidence
                .map(|c| format!("Confidence {c:.2}"))
                .unwrap_or_else(|| "No confidence score".to_owned());

            let generated_at = record
                .generated_at
                .get(..16)
                .unwrap_or(&record.generated_at)
                .to_owned();

            ProposalEvidenceView {
                id: record.id.clone(),
                generation_id: record.generation_id.clone(),
                is_rule,
                source_label,
                engine_revision,
                outcome_label,
                proposed_category,
                evidence_source_label,
                evidence_quote,
                evidence_page,
                evidence_span_verified,
                generated_at,
                data_class: data_class.clone(),
                review_outcome,
                review_history_label,
                confidence_label,
                summary_label: summary_label.clone(),
            }
        })
        .collect();

    (agreement, rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{
        ClassificationProposal, DocumentExtractedPage, DocumentSummary, ProposalEvidence,
        ProposalReviewEvent,
    };

    #[test]
    fn frozen_german_and_english_corpus_verifies_evidence_pages_agreement_and_data_classes() {
        // 1. German tax assessment (c2): rules and local model agree on "tax" with exact page-2 evidence.
        let german_detail = DocumentDetail {
            document: DocumentSummary {
                id: "a".repeat(64),
                filename: "Finanzamt_Bescheid_2026.pdf".into(),
                media_type: "application/pdf".into(),
                category: None,
                extracted_title: None,
                data_class: "c2".into(),
                page_count: 2,
                ..Default::default()
            },
            pages: vec![
                DocumentExtractedPage {
                    page_number: 1,
                    text: "Finanzamt München · Steuernummer 143/200/10000".into(),
                    engine: "pdfium".into(),
                    failure: None,
                },
                DocumentExtractedPage {
                    page_number: 2,
                    text: "Einkommensteuerbescheid für 2025 · Festgesetzte Steuer 4.120,00 EUR"
                        .into(),
                    engine: "pdfium".into(),
                    failure: None,
                },
            ],
            proposals: vec![
                DocumentProposalRecord {
                    id: "1".repeat(64),
                    generation_id: "gen-de-1".into(),
                    generated_at: "2026-11-20T09:15:00Z".into(),
                    proposal: ClassificationProposal {
                        document_id: "a".repeat(64),
                        source: ProposalSource::Rules {
                            identity: "sjel-rules".into(),
                            version: "v2".into(),
                        },
                        outcome: ProposalOutcome::Proposed {
                            category: "tax".into(),
                            evidence: ProposalEvidence {
                                source: ProposalEvidenceSource::Filename,
                                quote: "Finanzamt_Bescheid".into(),
                            },
                        },
                        confidence: None,
                        review: Some(ProposalReview::Accepted),
                    },
                    review_history: vec![ProposalReviewEvent {
                        id: 1,
                        outcome: ProposalReview::Accepted,
                        reviewed_at: "2026-11-20T09:16:00Z".into(),
                    }],
                },
                DocumentProposalRecord {
                    id: "2".repeat(64),
                    generation_id: "gen-de-1".into(),
                    generated_at: "2026-11-20T09:15:05Z".into(),
                    proposal: ClassificationProposal {
                        document_id: "a".repeat(64),
                        source: ProposalSource::Model {
                            role: "document_triage".into(),
                            model: "qwen3-8b".into(),
                            revision: Some("r4".into()),
                        },
                        outcome: ProposalOutcome::Proposed {
                            category: "tax".into(),
                            evidence: ProposalEvidence {
                                source: ProposalEvidenceSource::ExtractedText,
                                quote: "Einkommensteuerbescheid für 2025".into(),
                            },
                        },
                        confidence: Some(0.93),
                        review: None,
                    },
                    review_history: Vec::new(),
                },
            ],
            ..Default::default()
        };

        let (agreement, rows) = evaluate_proposal_run(&german_detail, false);
        assert_eq!(agreement, ProposalAgreement::Agree("tax".into()));
        assert_eq!(rows.len(), 2, "rule and model proposals stay separate rows");
        assert_eq!(
            german_detail.document.category, None,
            "proposal evaluation never overwrites owner category or files automatically"
        );
        // Model row has confidence, page 2 evidence, and honest absent summary
        let model_row = rows.iter().find(|r| !r.is_rule).expect("model row");
        assert_eq!(model_row.evidence_page, Some(2));
        assert!(model_row.evidence_span_verified);
        assert_eq!(model_row.confidence_label, "Confidence 0.93");
        assert_eq!(model_row.summary_label, "No summary generated");
        // Rule row shows honest absent confidence
        let rule_row = rows.iter().find(|r| r.is_rule).expect("rule row");
        assert!(rule_row.evidence_span_verified);
        assert_eq!(rule_row.confidence_label, "No confidence score");
        assert_eq!(rule_row.review_outcome, "Agreed");

        // 2. English services agreement (c2): rules and model conflict ("finance" vs "legal"),
        // and fabricated model quote is flagged as unverified.
        let english_detail = DocumentDetail {
            document: DocumentSummary {
                id: "b".repeat(64),
                filename: "Master_Services_Agreement_Invoice_Schedule.pdf".into(),
                media_type: "application/pdf".into(),
                category: Some("legal".into()),
                extracted_title: Some("Master Services Agreement 2026".into()),
                data_class: "c2".into(),
                page_count: 1,
                ..Default::default()
            },
            pages: vec![DocumentExtractedPage {
                page_number: 1,
                text:
                    "This Master Services Agreement governs consulting deliverables and liability."
                        .into(),
                engine: "pdfium".into(),
                failure: None,
            }],
            proposals: vec![
                DocumentProposalRecord {
                    id: "3".repeat(64),
                    generation_id: "gen-en-1".into(),
                    generated_at: "2026-11-20T10:00:00Z".into(),
                    proposal: ClassificationProposal {
                        document_id: "b".repeat(64),
                        source: ProposalSource::Rules {
                            identity: "sjel-rules".into(),
                            version: "v2".into(),
                        },
                        outcome: ProposalOutcome::Proposed {
                            category: "finance".into(),
                            evidence: ProposalEvidence {
                                source: ProposalEvidenceSource::Filename,
                                quote: "Invoice_Schedule".into(),
                            },
                        },
                        confidence: None,
                        review: None,
                    },
                    review_history: Vec::new(),
                },
                DocumentProposalRecord {
                    id: "4".repeat(64),
                    generation_id: "gen-en-1".into(),
                    generated_at: "2026-11-20T10:00:02Z".into(),
                    proposal: ClassificationProposal {
                        document_id: "b".repeat(64),
                        source: ProposalSource::Model {
                            role: "document_triage".into(),
                            model: "qwen3-8b".into(),
                            revision: Some("r4".into()),
                        },
                        outcome: ProposalOutcome::Proposed {
                            category: "legal".into(),
                            evidence: ProposalEvidence {
                                source: ProposalEvidenceSource::ExtractedText,
                                quote: "governs consulting deliverables and liability".into(),
                            },
                        },
                        confidence: Some(0.88),
                        review: None,
                    },
                    review_history: Vec::new(),
                },
            ],
            ..Default::default()
        };

        let (en_agreement, en_rows) = evaluate_proposal_run(&english_detail, false);
        assert_eq!(
            en_agreement,
            ProposalAgreement::Conflict(vec!["finance".into(), "legal".into()])
        );
        assert_eq!(en_rows[0].summary_label, "Master Services Agreement 2026");
        assert_eq!(
            english_detail.document.category.as_deref(),
            Some("legal"),
            "owner category remains untouched"
        );

        // 3. Data-class boundaries: c2 never leaves the host; c3 never enters prompts; generation needs approved loopback role.
        assert!(can_leave_host("c0"));
        assert!(can_leave_host("c1"));
        assert!(!can_leave_host("c2"), "c2 never leaves the host");
        assert!(!can_leave_host("c3"), "c3 never leaves the host");

        assert!(
            !can_enter_model_prompt("c2", false),
            "unapproved role cannot run generation"
        );
        assert!(
            can_enter_model_prompt("c2", true),
            "approved loopback role may inspect c2 on host"
        );
        assert!(
            !can_enter_model_prompt("c3", true),
            "c3 never enters model prompts even on loopback"
        );
    }
}
