//! Private category/rule configuration and the proposal contract.
//!
//! This module does not execute inference or change filing metadata. A configured rule set
//! replaces the built-in defaults as a whole; filed categories remain free-form.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

const CONFIG_NAME: &str = "documents-classification.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassificationConfig {
    version: String,
    categories: Vec<String>,
    rules: Vec<KeywordRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeywordRule {
    id: String,
    category: String,
    keywords: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassificationProposal {
    pub document_id: String,
    pub source: ProposalSource,
    pub outcome: ProposalOutcome,
    /// `None` is serialized as JSON `null` when a source gives no confidence value.
    pub confidence: Option<f32>,
    pub review: Option<ProposalReview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProposalSource {
    Rules {
        identity: String,
        version: String,
    },
    Model {
        role: String,
        model: String,
        revision: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProposalOutcome {
    Proposed {
        category: String,
        evidence: Evidence,
    },
    Abstained {
        reason: AbstentionReason,
    },
    Invalid {
        reason: InvalidProposalReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub source: EvidenceSource,
    /// Exact contiguous quote from the named source; validation belongs to the producer boundary.
    pub quote: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    Filename,
    ExtractedText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbstentionReason {
    NoMatch,
    MissingEvidence,
    ModelUnavailable,
    CategoryNotConfigured,
    SafetyRefused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvalidProposalReason {
    MalformedOutput,
    EvidenceNotExact,
    EvidenceTooLong,
    InvalidCategory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalReview {
    Accepted,
    Rejected,
    Overridden,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposalReviewRequest {
    pub outcome: ProposalReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalReviewEvent {
    pub id: i64,
    pub outcome: ProposalReview,
    pub reviewed_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredProposal {
    pub id: String,
    pub generation_id: String,
    pub generated_at: String,
    pub proposal: ClassificationProposal,
    pub review_history: Vec<ProposalReviewEvent>,
}

impl KeywordRule {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn category(&self) -> &str {
        &self.category
    }

    pub fn keywords(&self) -> &[String] {
        &self.keywords
    }
}

impl Default for ClassificationConfig {
    fn default() -> Self {
        Self {
            version: "builtin-v1".into(),
            categories: vec!["Finance".into(), "Events".into(), "Reference".into()],
            rules: vec![
                KeywordRule {
                    id: "finance-keywords".into(),
                    category: "Finance".into(),
                    keywords: vec![
                        "invoice".into(),
                        "rechnung".into(),
                        "receipt".into(),
                        "rechnungs".into(),
                    ],
                },
                KeywordRule {
                    id: "events-keywords".into(),
                    category: "Events".into(),
                    keywords: vec![
                        "summit".into(),
                        "conference".into(),
                        "congress".into(),
                        "konferenz".into(),
                    ],
                },
                KeywordRule {
                    id: "reference-keywords".into(),
                    category: "Reference".into(),
                    keywords: vec![
                        "manual".into(),
                        "handbuch".into(),
                        "documentation".into(),
                        "guide".into(),
                    ],
                },
            ],
        }
    }
}

impl ClassificationConfig {
    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn categories(&self) -> &[String] {
        &self.categories
    }

    pub fn rules(&self) -> &[KeywordRule] {
        &self.rules
    }

    /// Generate one evidence-bearing proposal for each matching configured rule.
    pub fn proposals_for(
        &self,
        document_id: &str,
        filename: &str,
        extracted_text: Option<&str>,
    ) -> Vec<ClassificationProposal> {
        let mut proposals = Vec::with_capacity(self.rules.len().max(1));
        for rule in &self.rules {
            let evidence = rule.keywords.iter().find_map(|keyword| {
                find_quote(filename, keyword)
                    .map(|quote| Evidence {
                        source: EvidenceSource::Filename,
                        quote: quote.to_string(),
                    })
                    .or_else(|| {
                        extracted_text.and_then(|text| {
                            find_quote(text, keyword).map(|quote| Evidence {
                                source: EvidenceSource::ExtractedText,
                                quote: quote.to_string(),
                            })
                        })
                    })
            });
            if let Some(evidence) = evidence {
                proposals.push(ClassificationProposal {
                    document_id: document_id.to_string(),
                    source: ProposalSource::Rules {
                        identity: rule.id.clone(),
                        version: self.version.clone(),
                    },
                    outcome: ProposalOutcome::Proposed {
                        category: rule.category.clone(),
                        evidence,
                    },
                    confidence: None,
                    review: None,
                });
            }
        }
        if proposals.is_empty() {
            proposals.push(ClassificationProposal {
                document_id: document_id.to_string(),
                source: ProposalSource::Rules {
                    identity: "keyword-rules".into(),
                    version: self.version.clone(),
                },
                outcome: ProposalOutcome::Abstained {
                    reason: AbstentionReason::NoMatch,
                },
                confidence: None,
                review: None,
            });
        }
        proposals
    }

    /// Load the private overlay config. An absent file uses the replaceable built-in defaults;
    /// an unreadable or invalid file is an error and must not silently restore the defaults.
    pub fn load() -> Result<Self, String> {
        let Some(path) = sjel_config::overlay_config(CONFIG_NAME) else {
            return Ok(Self::default());
        };
        let body = match std::fs::read_to_string(&path) {
            Ok(body) => body,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(format!("could not read {}: {error}", path.display()));
            }
        };
        Self::from_json(&body).map_err(|error| format!("invalid {}: {error}", path.display()))
    }

    /// Parse and validate a complete replacement rule set.
    pub fn from_json(body: &str) -> Result<Self, String> {
        let config: Self = serde_json::from_str(body).map_err(|error| error.to_string())?;
        config.validate()?;
        Ok(config)
    }

    /// Check the configured label and bounded exact evidence for a proposal.
    pub fn validate_proposal(
        &self,
        proposal: &ClassificationProposal,
        filename: &str,
        extracted_text: Option<&str>,
    ) -> Result<(), String> {
        if proposal.confidence.is_some_and(|score| !score.is_finite()) {
            return Err("proposal confidence must be finite".into());
        }
        let ProposalOutcome::Proposed { category, evidence } = &proposal.outcome else {
            return Ok(());
        };
        if !self.categories.contains(category) {
            return Err("proposal category is not configured".into());
        }
        let source = match evidence.source {
            EvidenceSource::Filename => filename,
            EvidenceSource::ExtractedText => extracted_text.unwrap_or_default(),
        };
        let word_count = evidence.quote.split_whitespace().count();
        if evidence.quote.is_empty()
            || evidence.quote.chars().count() > 160
            || word_count > 20
            || !source.contains(&evidence.quote)
        {
            return Err(
                "proposal evidence must be an exact source quote of at most 20 words and 160 characters"
                    .into(),
            );
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), String> {
        if self.version.trim().is_empty()
            || self.version.len() > 120
            || self.version.chars().any(char::is_control)
        {
            return Err("rule-set version must be a non-empty label of at most 120 bytes".into());
        }
        if self.categories.is_empty() {
            return Err("categories must contain at least one label".into());
        }
        for (index, category) in self.categories.iter().enumerate() {
            validate_label(category)?;
            if self.categories[..index].contains(category) {
                return Err(format!("duplicate category {category:?}"));
            }
        }
        for (index, rule) in self.rules.iter().enumerate() {
            if rule.id.is_empty()
                || rule.id.len() > 80
                || !rule
                    .id
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character))
                || self.rules[..index].iter().any(|prior| prior.id == rule.id)
            {
                return Err("rule ids must be unique ASCII labels of at most 80 characters".into());
            }
            validate_label(&rule.category)?;
            if !self.categories.contains(&rule.category) {
                return Err(format!(
                    "rule category {:?} is not in categories",
                    rule.category
                ));
            }
            if rule.keywords.is_empty() {
                return Err(format!("rule for {:?} has no keywords", rule.category));
            }
            for keyword in &rule.keywords {
                if keyword.trim().is_empty()
                    || keyword.trim() != keyword
                    || keyword.chars().count() > 160
                    || keyword.split_whitespace().count() > 20
                    || keyword.chars().any(char::is_control)
                {
                    return Err(format!(
                        "rule for {:?} has an invalid keyword",
                        rule.category
                    ));
                }
            }
        }
        Ok(())
    }
}

fn find_quote<'a>(source: &'a str, keyword: &str) -> Option<&'a str> {
    let target: Vec<char> = keyword.chars().flat_map(char::to_lowercase).collect();
    if target.is_empty() {
        return None;
    }
    let mut prefix = vec![0; target.len()];
    for index in 1..target.len() {
        let mut matched = prefix[index - 1];
        while matched > 0 && target[index] != target[matched] {
            matched = prefix[matched - 1];
        }
        if target[index] == target[matched] {
            matched += 1;
        }
        prefix[index] = matched;
    }

    let mut matched = 0;
    let mut starts = VecDeque::with_capacity(target.len());
    for (start, character) in source.char_indices() {
        let end = start + character.len_utf8();
        for folded in character.to_lowercase() {
            starts.push_back(start);
            if starts.len() > target.len() {
                starts.pop_front();
            }
            while matched > 0 && target[matched] != folded {
                matched = prefix[matched - 1];
            }
            if target[matched] == folded {
                matched += 1;
            }
            if matched == target.len() {
                return source.get(*starts.front()?..end);
            }
        }
    }
    None
}

fn validate_label(label: &str) -> Result<(), String> {
    if label.is_empty()
        || label.len() > 120
        || label == "."
        || label == ".."
        || label.contains('/')
        || label.contains('\\')
        || label.chars().any(char::is_control)
    {
        return Err("category must be a single safe label (max 120 bytes)".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_replaces_defaults() {
        let config = ClassificationConfig::from_json(
            r#"{
                "version": "travel-v1",
                "categories": ["Travel"],
                "rules": [{"id": "travel-keywords", "category": "Travel", "keywords": ["ticket", "hotel"]}]
            }"#,
        )
        .unwrap();

        assert_eq!(config.categories, ["Travel"]);
        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules[0].keywords, ["ticket", "hotel"]);
        assert!(!config.categories.contains(&"Finance".to_string()));
    }

    #[test]
    fn rule_category_must_be_configured() {
        let error = ClassificationConfig::from_json(
            r#"{
                "version": "travel-v1",
                "categories": ["Travel"],
                "rules": [{"id": "finance-keywords", "category": "Finance", "keywords": ["invoice"]}]
            }"#,
        )
        .unwrap_err();

        assert!(error.contains("is not in categories"));
    }

    #[test]
    fn dev_proposals_keep_evidence_valid() {
        #[derive(Deserialize)]
        struct Sample {
            id: String,
            text: String,
        }

        let samples: Vec<Sample> =
            serde_json::from_str(include_str!("../eval/clm-corpus-v2.json")).unwrap();
        let config = ClassificationConfig::default();
        assert_eq!(samples.len(), 48);
        for sample in samples {
            let proposals = config.proposals_for(&sample.id, "synthetic.txt", Some(&sample.text));
            for proposal in proposals {
                assert!(config
                    .validate_proposal(&proposal, "synthetic.txt", Some(&sample.text))
                    .is_ok());
            }
        }
    }

    #[test]
    fn rules_return_separate_exact_proposals() {
        let config = ClassificationConfig::default();
        let proposals = config.proposals_for(
            "document-id",
            "Rechnung.pdf",
            Some("Schedule for the annual conference"),
        );

        assert_eq!(proposals.len(), 2);
        assert!(matches!(
            &proposals[0].outcome,
            ProposalOutcome::Proposed { category, evidence }
                if category == "Finance"
                    && evidence.source == EvidenceSource::Filename
                    && evidence.quote == "Rechnung"
        ));
        assert!(matches!(
            &proposals[1].outcome,
            ProposalOutcome::Proposed { category, evidence }
                if category == "Events"
                    && evidence.source == EvidenceSource::ExtractedText
                    && evidence.quote == "conference"
        ));
        assert_eq!(proposals[0].confidence, None);
        assert_eq!(
            proposals[0].source,
            ProposalSource::Rules {
                identity: "finance-keywords".into(),
                version: "builtin-v1".into(),
            }
        );
    }

    #[test]
    fn no_match_abstains() {
        let proposals = ClassificationConfig::default().proposals_for(
            "document-id",
            "notes.txt",
            Some("Synthetic content without category terms"),
        );
        assert_eq!(proposals.len(), 1);
        assert_eq!(
            proposals[0].outcome,
            ProposalOutcome::Abstained {
                reason: AbstentionReason::NoMatch
            }
        );
    }

    #[test]
    fn proposal_round_trip_keeps_evidence() {
        let proposal = ClassificationProposal {
            document_id: "document-id".into(),
            source: ProposalSource::Rules {
                identity: "keyword-rules".into(),
                version: "v1".into(),
            },
            outcome: ProposalOutcome::Proposed {
                category: "Events".into(),
                evidence: Evidence {
                    source: EvidenceSource::ExtractedText,
                    quote: "Annual conference".into(),
                },
            },
            confidence: None,
            review: None,
        };

        let encoded = serde_json::to_string(&proposal).unwrap();
        let decoded: ClassificationProposal = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, proposal);
        assert!(encoded.contains("\"confidence\":null"));
        assert!(ClassificationConfig::default()
            .validate_proposal(&proposal, "agenda.pdf", Some("Annual conference"))
            .is_ok());
    }

    #[test]
    fn proposal_validation() {
        let mut proposal = ClassificationProposal {
            document_id: "document-id".into(),
            source: ProposalSource::Rules {
                identity: "keyword-rules".into(),
                version: "v1".into(),
            },
            outcome: ProposalOutcome::Proposed {
                category: "Travel".into(),
                evidence: Evidence {
                    source: EvidenceSource::ExtractedText,
                    quote: "Annual conference".into(),
                },
            },
            confidence: None,
            review: None,
        };
        let config = ClassificationConfig::default();
        assert!(config
            .validate_proposal(&proposal, "agenda.pdf", Some("Annual conference"))
            .is_err());

        proposal.outcome = ProposalOutcome::Proposed {
            category: "Events".into(),
            evidence: Evidence {
                source: EvidenceSource::ExtractedText,
                quote: "Conference".into(),
            },
        };
        assert!(config
            .validate_proposal(&proposal, "agenda.pdf", Some("Annual conference"))
            .is_err());

        proposal.confidence = Some(f32::NAN);
        assert!(config
            .validate_proposal(&proposal, "agenda.pdf", Some("Annual conference"))
            .is_err());
    }

    #[test]
    fn invalid_config_is_an_error() {
        assert!(ClassificationConfig::from_json("not json").is_err());
    }
}
