use std::time::Duration;

use serde_json::{json, Value};

use crate::{
    classification::{
        AbstentionReason, ClassificationConfig, ClassificationProposal, Evidence, EvidenceSource,
        InvalidProposalReason, ProposalOutcome, ProposalSource,
    },
    Document, LibraryError,
};

const ROLE: &str = "document_classification";
const MAX_EXCERPT_BYTES: usize = 2400;

pub(crate) fn propose(
    document: &Document,
    config: &ClassificationConfig,
) -> Result<ClassificationProposal, LibraryError> {
    let role = sjel_inference::InferenceConfig::load(sjel_config::overlay_config)
        .role(ROLE)
        .filter(|role| role.is_loopback())
        .ok_or_else(|| LibraryError::Model("local role is unavailable".into()))?;
    if role.uses_responses_api() {
        return Err(LibraryError::Model(
            "document classification requires a local chat-completions role".into(),
        ));
    }

    let text = document.extracted_text.as_deref().unwrap_or_default();
    let excerpt = bounded_excerpt(text, MAX_EXCERPT_BYTES);
    if excerpt.trim().is_empty() {
        return Ok(proposal(
            document,
            &role.model,
            ProposalOutcome::Abstained {
                reason: AbstentionReason::MissingEvidence,
            },
        ));
    }

    let category_schema = json!({"type":"string", "enum": config.categories()});
    let system = system_prompt(config);
    let request = json!({
        "model": role.model,
        "messages": [
            {"role":"system", "content":system},
            {"role":"user", "content":format!("Document text:\n{}", excerpt)}
        ],
        "max_tokens": 160,
        "temperature": 0,
        "response_format": {"type":"json_schema", "json_schema": {
            "name":"document_classification",
            "strict":true,
            "schema":{"type":"object", "properties":{"category":category_schema, "evidence":{"type":"string"}}, "required":["category", "evidence"], "additionalProperties":false}
        }}
    });
    role.admit_chat_request(&request)
        .map_err(|error| LibraryError::Model(format!("local role declined request: {error}")))?;
    let client = sjel_http::client(
        sjel_http::Purpose::new("documents-classification"),
        Duration::from_secs(45),
    )
    .map_err(|error| LibraryError::Model(error.to_string()))?;
    let mut call = client.post(role.chat_completions_endpoint()).json(&request);
    if let Some(token) = role.bearer_key() {
        call = call.bearer_auth(token);
    }
    let response = call
        .send()
        .map_err(|error| LibraryError::Model(error.to_string()))?;
    if !response.status().is_success() {
        let safety_refused = response
            .json::<Value>()
            .ok()
            .and_then(|body| {
                body.pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .is_some_and(|message| message.to_lowercase().contains("safety guardrails"));
        return Ok(proposal(
            document,
            &role.model,
            ProposalOutcome::Abstained {
                reason: if safety_refused {
                    AbstentionReason::SafetyRefused
                } else {
                    AbstentionReason::ModelUnavailable
                },
            },
        ));
    }
    let body: Value = response
        .json()
        .map_err(|error| LibraryError::Model(error.to_string()))?;
    let content = body
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str);
    if body
        .pointer("/choices/0/message/refusal")
        .and_then(Value::as_str)
        .is_some_and(|refusal| !refusal.is_empty())
    {
        return Ok(proposal(
            document,
            &role.model,
            ProposalOutcome::Abstained {
                reason: AbstentionReason::SafetyRefused,
            },
        ));
    }
    let outcome = match content.and_then(|content| serde_json::from_str::<Value>(content).ok()) {
        Some(value) => {
            let category = value.get("category").and_then(Value::as_str);
            let evidence = value.get("evidence").and_then(Value::as_str);
            match (category, evidence) {
                (Some(_), Some(evidence)) if evidence.trim().is_empty() => {
                    ProposalOutcome::Abstained {
                        reason: AbstentionReason::MissingEvidence,
                    }
                }
                (Some(category), Some(evidence)) => {
                    let candidate = proposal(
                        document,
                        &role.model,
                        ProposalOutcome::Proposed {
                            category: category.to_owned(),
                            evidence: Evidence {
                                source: EvidenceSource::ExtractedText,
                                quote: evidence.to_owned(),
                            },
                        },
                    );
                    match config.validate_proposal(&candidate, &document.filename, Some(&excerpt)) {
                        Ok(()) => candidate.outcome,
                        Err(error) if error.contains("category") => ProposalOutcome::Invalid {
                            reason: InvalidProposalReason::InvalidCategory,
                        },
                        Err(error) if error.contains("at most") => ProposalOutcome::Invalid {
                            reason: InvalidProposalReason::EvidenceTooLong,
                        },
                        Err(_) => ProposalOutcome::Invalid {
                            reason: InvalidProposalReason::EvidenceNotExact,
                        },
                    }
                }
                _ => ProposalOutcome::Invalid {
                    reason: InvalidProposalReason::MalformedOutput,
                },
            }
        }
        None => ProposalOutcome::Invalid {
            reason: InvalidProposalReason::MalformedOutput,
        },
    };
    Ok(proposal(document, &role.model, outcome))
}

fn proposal(document: &Document, model: &str, outcome: ProposalOutcome) -> ClassificationProposal {
    ClassificationProposal {
        document_id: document.id.clone(),
        source: ProposalSource::Model {
            role: ROLE.to_owned(),
            model: model.to_owned(),
            revision: None,
        },
        outcome,
        confidence: None,
        review: None,
    }
}

fn bounded_excerpt(text: &str, max_bytes: usize) -> String {
    let end = text
        .char_indices()
        .map(|(index, character)| index + character.len_utf8())
        .take_while(|end| *end <= max_bytes)
        .last()
        .unwrap_or(0);
    text[..end].to_owned()
}

fn system_prompt(config: &ClassificationConfig) -> String {
    const GUIDANCE: [(&str, &str); 3] = [
        (
            "Finance",
            "invoices, bills, receipts, payments, account statements, or reimbursements",
        ),
        (
            "Events",
            "invitations, schedules, programs, tickets, or registration for gatherings and activities",
        ),
        (
            "Reference",
            "manuals, instructions, technical guides, API documentation, or procedures",
        ),
    ];
    let descriptions = config
        .categories()
        .iter()
        .map(|category| {
            let description = GUIDANCE
                .iter()
                .find_map(|(name, guidance)| (*name == category).then_some(*guidance))
                .unwrap_or("documents whose primary purpose matches this category");
            format!("{category}: {description}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Classify the supplied document by its primary purpose. Choose exactly one allowed category. Use the category descriptions. Return the shortest useful evidence as an exact, contiguous quote copied from the document text. The quote must have no more than 20 whitespace-separated words and no more than 160 characters. Do not paraphrase or add words.\n{descriptions}"
    )
}
