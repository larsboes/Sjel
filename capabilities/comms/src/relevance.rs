//! TELOS relevance for the general Feed.
//!
//! Comms owns this concern: it ranks any observation against explicitly
//! configured focus lenses without turning Feed into a Scouting view. Profiles
//! and items first pass through one resolved `embedding` role in one batch.
//! Embeddings select at most three lens candidates per item; a resolved
//! `reranking` role then scores those query-document pairs jointly. When the
//! model stages are absent or unavailable, the stored mode truthfully steps
//! down to `semantic` or the deterministic `lexical` control.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::config::RelevanceConfig;
use crate::store::FeedItem;
use sjel_inference::{ResolvedRole, TextRole};

// The selected multilingual E5 model accepts 512 tokens. A conservative
// character cap avoids sending and tokenizing long transcripts that the model
// would discard anyway, while leaving room for multilingual token variation.
const DOCUMENT_CAP: usize = 1_800;
const PROFILE_CAP: usize = 1_800;
const LEXICAL_DIMENSIONS: usize = 512;
const CANDIDATE_PROFILES_PER_ITEM: usize = 3;
const RERANK_BATCH_SIZE: usize = 32;

#[derive(Debug, Clone)]
pub struct InterestProfile {
    pub key: String,
    pub label: String,
    pub focus: String,
    pub text: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct RelevanceMatch {
    pub profile_key: String,
    pub profile_label: String,
    pub score: f64,
    pub rationale: String,
    pub mode: String,
    pub profile_revision: String,
}

#[derive(Debug, Clone)]
pub struct ScoredFeedItem {
    pub feed_id: String,
    pub matches: Vec<RelevanceMatch>,
    /// The class ladder refused this item: it reached neither the embedder nor
    /// the lexical scorer. `matches` is empty by refusal rather than by
    /// absence, and the caller stores that as a refusal instead of a zero.
    pub refused_class: bool,
}

/// What a scoring pass did, beside what it scored.
///
/// `context_revision` records the *configured* embedding producer, not the
/// producer that answered, so a pass that fell back was stamped semantic and
/// never re-scored (#the 525 lexical rows all written in one pass on
/// 2026-08-30). This is the receipt that makes the difference readable.
#[derive(Debug, Clone)]
pub struct ScoringOutcome {
    /// No scores may be persisted when temporary admission refuses the pass.
    pub deferred: Option<String>,
    pub items: Vec<ScoredFeedItem>,
    /// The strongest mode any item actually got: `reranked`, `semantic`,
    /// `lexical`, or `unscored` when there was nothing to score.
    pub mode: &'static str,
    /// A stable class, never a provider message: `embedding-unreachable` or
    /// `embedding-failed`. `None` for a machine that has simply declared no
    /// TELOS lens -- that is a configuration and `profile_count` reports it.
    pub error_class: Option<&'static str>,
    /// Embedding calls attempted and how many of them fell back. One failure
    /// now costs one chunk, not the pass.
    pub chunks: usize,
    pub chunks_failed: usize,
}

impl ScoringOutcome {
    fn deferred(reason: String) -> Self {
        Self {
            deferred: Some(reason),
            items: Vec::new(),
            mode: "policy_deferred",
            error_class: None,
            chunks: 0,
            chunks_failed: 0,
        }
    }
}

/// Admission does not change which producers define the stored vector space.
pub fn runtime_admission(
    embedding_role: Option<&ResolvedRole>,
    reranking_role: Option<&ResolvedRole>,
) -> Result<(), sjel_runtime::Deferred> {
    sjel_runtime::require(sjel_runtime::Category::BulkIndexing)?;
    for role in [embedding_role, reranking_role].into_iter().flatten() {
        role.runtime_admission()?;
    }
    Ok(())
}

pub fn embedding_provider_label(role: Option<&ResolvedRole>) -> &'static str {
    role.map(ResolvedRole::provider_label)
        .unwrap_or("No embedding role configured")
}

pub fn embedding_backend_configured(role: Option<&ResolvedRole>) -> bool {
    role.is_some_and(|role| {
        !role.backend.base_url.trim().is_empty() && !role.model.trim().is_empty()
    })
}

/// Read the configured TELOS lenses, or say why there are none.
///
/// Fallible, and narrowly so. Declaring no profile path is the ordinary state
/// of a fresh install, of CI and of every worktree — `RelevanceConfig::default`
/// has an empty list and two tests pin it (config.rs) — so zero declared paths
/// is `Ok(vec![])` and `/feed/evaluation/status` keeps answering 200 with
/// `profile_count: 0`. What used to fail open and now does not is the real
/// defect: a *declared* directory that has moved silently produced zero
/// profiles, and the pass then scored 372 good rows against nothing and wrote
/// them all `unscored`.
pub fn load_profiles(config: &RelevanceConfig) -> Result<Vec<InterestProfile>, String> {
    let mut files = Vec::new();
    for configured in &config.profile_paths {
        let path = PathBuf::from(configured);
        if path.is_file() {
            files.push(path);
        } else if path.is_dir() {
            let entries = fs::read_dir(&path).map_err(|error| {
                format!("a declared TELOS profile directory cannot be read: {error}")
            })?;
            let mut markdown = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|entry| entry.extension().and_then(|ext| ext.to_str()) == Some("md"))
                .collect::<Vec<_>>();
            markdown.sort();
            files.extend(markdown);
        } else {
            // Neither a file nor a directory. The operator declared it, so a
            // silent skip is the answer that loses the lens.
            return Err("a declared TELOS profile path does not exist".into());
        }
    }

    // No declared path is not an error. It is the default, and erroring here
    // would turn /feed and /feed/library into a 500 on load for a machine that
    // has simply not written a lens.
    if config.profile_paths.is_empty() {
        return Ok(Vec::new());
    }

    let profiles = files
        .into_iter()
        .filter_map(|path| parse_profile(&path))
        .collect::<Vec<_>>();
    if profiles.is_empty() {
        return Err("declared TELOS profile paths yielded no readable profiles".into());
    }
    Ok(profiles)
}

fn parse_profile(path: &Path) -> Option<InterestProfile> {
    let label = path.file_stem()?.to_str()?.trim().to_string();
    if label.eq_ignore_ascii_case("focus") || label.eq_ignore_ascii_case("readme") {
        return None;
    }
    let body = fs::read_to_string(path).ok()?;
    let summary = frontmatter_value(&body, "summary").unwrap_or_default();
    let current_focus = frontmatter_value(&body, "current_focus").unwrap_or_default();
    let affinity = frontmatter_value(&body, "category_affinity").unwrap_or_default();
    let focus = [summary.as_str(), current_focus.as_str(), affinity.as_str()]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let text = profile_embedding_text(&label, &summary, &current_focus, &affinity, &body);
    let fingerprint = sha256_hex(&text);
    Some(InterestProfile {
        key: sha256_hex(&path.to_string_lossy()),
        label,
        focus,
        text,
        fingerprint,
    })
}

fn profile_embedding_text(
    label: &str,
    summary: &str,
    current_focus: &str,
    affinity: &str,
    body: &str,
) -> String {
    let text = frontmatter_value(body, "relevance_query")
        .filter(|query| !query.trim().is_empty())
        .unwrap_or_else(|| format!("{label}\n{summary}\n{current_focus}\n{affinity}\n{body}"));
    cap_chars(&text, PROFILE_CAP)
}

fn frontmatter_value(body: &str, key: &str) -> Option<String> {
    let mut lines = body.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let line = line.trim();
        if line == "---" {
            break;
        }
        if let Some(value) = line.strip_prefix(&format!("{key}:")) {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// One embedding call per this many items, plus one for the profile set.
///
/// The old shape sent every item and every profile in ONE call and fell back to
/// lexical for the whole batch when it failed. That is how 525 rows were
/// written lexical in a single pass. A chunk now costs a chunk.
const EMBED_CHUNK_SIZE: usize = 32;

/// Score items against the configured lenses.
///
/// Two things happen before any text leaves this process. Items are partitioned
/// by `content_item::local_prompt_allowed`, and a refused item is excluded from
/// the embedding input AND from the lexical fallback — one rule instead of two,
/// because a lexical score is still a content-derived number rendered in a
/// rationale on a surface. An embedding is a model call and `item_document`
/// joins title, author and content, which for a mail is the sender address.
pub fn score_items(
    items: &[FeedItem],
    profiles: &[InterestProfile],
    embedding_role: Option<&ResolvedRole>,
    reranking_role: Option<&ResolvedRole>,
) -> ScoringOutcome {
    if let Err(reason) = runtime_admission(embedding_role, reranking_role) {
        return ScoringOutcome::deferred(reason.to_string());
    }
    let allowed = items
        .iter()
        .enumerate()
        .filter(|(_, item)| crate::content_item::local_prompt_allowed(&item.data_class))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let mut scored = items
        .iter()
        .map(|item| ScoredFeedItem {
            feed_id: item.id.clone(),
            matches: Vec::new(),
            refused_class: !crate::content_item::local_prompt_allowed(&item.data_class),
        })
        .collect::<Vec<_>>();

    // No declared lens is a configuration, not a failure, and `load_profiles`
    // says so in the same words: it is the ordinary state of a fresh install, of
    // CI and of every worktree. Reporting it as an `error_class` made
    // `record_relevance_pass` grow `consecutive_failures` on every pass forever
    // and made the dashboard's dot red beside the words "lexical fallback" on a
    // machine that never attempted an embedding. The count is reported instead:
    // `profile_count` is already in every receipt and in the status endpoint.
    if profiles.is_empty() {
        return ScoringOutcome {
            deferred: None,
            items: scored,
            mode: "unscored",
            error_class: None,
            chunks: 0,
            chunks_failed: 0,
        };
    }
    if allowed.is_empty() {
        return ScoringOutcome {
            deferred: None,
            items: scored,
            mode: "unscored",
            error_class: None,
            chunks: 0,
            chunks_failed: 0,
        };
    }

    let item_documents = allowed
        .iter()
        .map(|index| item_document(&items[*index]))
        .collect::<Vec<_>>();
    // Always computed: they are the fallback space for any chunk that does not
    // get an embedding, and comparing a lexical item vector against a semantic
    // profile vector would be a number with no meaning.
    let lexical_profile_vectors = profiles
        .iter()
        .map(|profile| lexical_vector(&profile.text))
        .collect::<Vec<_>>();

    let mut chunks = 0usize;
    let mut chunks_failed = 0usize;
    let mut error_class: Option<&'static str> = None;

    let mut deferred = None;
    let semantic_profile_vectors = embedding_role.and_then(|role| {
        chunks += 1;
        let inputs = profiles
            .iter()
            .map(|profile| (profile.text.clone(), TextRole::Query))
            .collect::<Vec<_>>();
        match embed(role, &inputs) {
            Ok(vectors) if vectors.len() == profiles.len() => Some(vectors),
            Err(error) if sjel_runtime::is_deferred(&error) => {
                deferred = Some(error);
                None
            }
            outcome => {
                chunks_failed += 1;
                error_class = Some(report_embed_failure("the profile set", role, outcome.err()));
                None
            }
        }
    });

    if let Some(reason) = deferred.take() {
        return ScoringOutcome::deferred(reason);
    }
    let mut item_vectors: Vec<Vec<f64>> = Vec::with_capacity(item_documents.len());
    let mut item_modes: Vec<&'static str> = Vec::with_capacity(item_documents.len());
    for chunk in item_documents.chunks(EMBED_CHUNK_SIZE) {
        let embedded = match (embedding_role, semantic_profile_vectors.as_ref()) {
            (Some(role), Some(_)) => {
                chunks += 1;
                let inputs = chunk
                    .iter()
                    .cloned()
                    .map(|document| (document, TextRole::Document))
                    .collect::<Vec<_>>();
                match embed(role, &inputs) {
                    Ok(vectors) if vectors.len() == chunk.len() => Some(vectors),
                    Err(error) if sjel_runtime::is_deferred(&error) => {
                        return ScoringOutcome::deferred(error);
                    }
                    outcome => {
                        chunks_failed += 1;
                        error_class = error_class.or(Some(report_embed_failure(
                            "a chunk",
                            role,
                            outcome.err(),
                        )));
                        None
                    }
                }
            }
            // No embedding role, or its profile call failed. Lexical is the
            // declared control, not a degradation, when no role is configured.
            _ => None,
        };
        match embedded {
            Some(vectors) => {
                item_vectors.extend(vectors);
                item_modes.extend(std::iter::repeat_n("semantic", chunk.len()));
            }
            None => {
                item_vectors.extend(chunk.iter().map(|document| lexical_vector(document)));
                item_modes.extend(std::iter::repeat_n("lexical", chunk.len()));
            }
        }
    }

    let profile_vectors_for = |mode: &str| -> &[Vec<f64>] {
        match (mode, semantic_profile_vectors.as_ref()) {
            ("semantic", Some(vectors)) => vectors.as_slice(),
            _ => lexical_profile_vectors.as_slice(),
        }
    };

    let candidate_profiles = item_vectors
        .iter()
        .enumerate()
        .map(|(position, item_vector)| {
            // A lexical item is never a rerank candidate: the reranker is a
            // cross-encoder over the semantic stage, and feeding it a fallback
            // row would report `reranked` for a pass that never embedded.
            if item_modes[position] != "semantic" {
                return Vec::new();
            }
            let mut candidates = profile_vectors_for("semantic")
                .iter()
                .enumerate()
                .map(|(index, profile_vector)| (index, cosine(profile_vector, item_vector)))
                .collect::<Vec<_>>();
            candidates.sort_by(|left, right| {
                right
                    .1
                    .partial_cmp(&left.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            candidates.truncate(CANDIDATE_PROFILES_PER_ITEM);
            candidates
        })
        .collect::<Vec<_>>();

    let reranked = if candidate_profiles.iter().any(|row| !row.is_empty()) {
        reranking_role.and_then(|role| {
            match rerank_candidate_scores(role, profiles, &item_documents, &candidate_profiles) {
                Ok(scores) => Some(scores),
                Err(error) if sjel_runtime::is_deferred(&error) => {
                    deferred = Some(error);
                    None
                }
                Err(error) => {
                    eprintln!("  comms: reranking unavailable ({error}) - keeping semantic scores");
                    None
                }
            }
        })
    } else {
        None
    };

    if let Some(reason) = deferred {
        return ScoringOutcome::deferred(reason);
    }
    if let Err(reason) = runtime_admission(embedding_role, reranking_role) {
        return ScoringOutcome::deferred(reason.to_string());
    }
    for (position, item_index) in allowed.iter().enumerate() {
        let mode = item_modes[position];
        let rerank_scores = reranked
            .as_ref()
            .filter(|_| !candidate_profiles[position].is_empty())
            .map(|scores| &scores[position]);
        let profile_scores = if let Some(scores) = rerank_scores {
            candidate_profiles[position]
                .iter()
                .map(|(profile_index, _)| (*profile_index, scores[*profile_index].unwrap()))
                .collect::<Vec<_>>()
        } else {
            let profile_vectors = profile_vectors_for(mode);
            (0..profiles.len())
                .map(|profile_index| {
                    (
                        profile_index,
                        cosine(&profile_vectors[profile_index], &item_vectors[position]),
                    )
                })
                .collect::<Vec<_>>()
        };
        let scoring_mode = if rerank_scores.is_some() {
            "reranked"
        } else {
            mode
        };
        let mut matches = profile_scores
            .into_iter()
            .map(|(profile_index, score)| {
                let profile = &profiles[profile_index];
                let method = match scoring_mode {
                    "reranked" => "Reranked relevance",
                    "semantic" => "Semantic similarity",
                    _ => "Lexical similarity",
                };
                let rationale = if profile.focus.is_empty() {
                    format!("{method} for the TELOS lens {}", profile.label)
                } else {
                    format!("{method} for {} · {}", profile.label, profile.focus)
                };
                RelevanceMatch {
                    profile_key: profile.key.clone(),
                    profile_label: profile.label.clone(),
                    score,
                    rationale,
                    mode: scoring_mode.to_string(),
                    profile_revision: profile.fingerprint.clone(),
                }
            })
            .collect::<Vec<_>>();
        matches.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        matches.truncate(3);
        scored[*item_index].matches = matches;
    }

    let mode = if scored
        .iter()
        .any(|item| item.matches.first().is_some_and(|m| m.mode == "reranked"))
    {
        "reranked"
    } else if item_modes.contains(&"semantic") {
        "semantic"
    } else {
        "lexical"
    };
    ScoringOutcome {
        deferred: None,
        items: scored,
        mode,
        error_class,
        chunks,
        chunks_failed,
    }
}

/// Say why an embedding call did not answer, and put it in a class.
///
/// The reason is printed the way the rerank path prints its own — the previous
/// `.ok()` discarded it, so a degraded pass looked exactly like a configured
/// lexical one. `model_reachable` is probed only on a failure, which is what
/// separates "the server is down" from "the server answered wrongly".
fn report_embed_failure(what: &str, role: &ResolvedRole, error: Option<String>) -> &'static str {
    let reachable = role.model_reachable();
    let class = if reachable {
        "embedding-failed"
    } else {
        "embedding-unreachable"
    };
    let reason = error.unwrap_or_else(|| "the model returned the wrong number of vectors".into());
    eprintln!("  comms: embedding {what} failed ({class}: {reason}) - scoring it lexically");
    class
}

fn rerank_candidate_scores(
    role: &ResolvedRole,
    profiles: &[InterestProfile],
    item_documents: &[String],
    candidate_profiles: &[Vec<(usize, f64)>],
) -> Result<Vec<Vec<Option<f64>>>, String> {
    let mut scores = vec![vec![None; profiles.len()]; item_documents.len()];
    for (profile_index, profile) in profiles.iter().enumerate() {
        let item_indices = candidate_profiles
            .iter()
            .enumerate()
            .filter_map(|(item_index, candidates)| {
                candidates
                    .iter()
                    .any(|(candidate, _)| *candidate == profile_index)
                    .then_some(item_index)
            })
            .collect::<Vec<_>>();
        for item_chunk in item_indices.chunks(RERANK_BATCH_SIZE) {
            let documents = item_chunk
                .iter()
                .map(|index| item_documents[*index].clone())
                .collect::<Vec<_>>();
            sjel_runtime::require(sjel_runtime::Category::BulkIndexing)
                .map_err(|e| e.to_string())?;
            let reranked = role.rerank(&profile.text, &documents)?;
            for (item_index, score) in item_chunk.iter().zip(reranked) {
                scores[*item_index][profile_index] = Some(f64::from(score));
            }
        }
    }
    if scores.iter().enumerate().any(|(item_index, row)| {
        candidate_profiles[item_index]
            .iter()
            .any(|(profile_index, _)| row[*profile_index].is_none())
    }) {
        return Err("reranker omitted a selected candidate".into());
    }
    Ok(scores)
}

/// Cheap provider-specific readiness probe. Listing installed models avoids
/// running an embedding just to paint a status indicator in the dashboard.
pub fn embedding_backend_reachable(role: Option<&ResolvedRole>) -> bool {
    role.is_some_and(ResolvedRole::model_reachable)
}

fn item_document(item: &FeedItem) -> String {
    // Once a summary exists it is the distilled relevance input. Re-appending
    // the full transcript duplicates its information and wastes the small
    // embedding model's fixed context window.
    let content = item
        .summary
        .as_deref()
        .or(item.transcript.as_deref())
        .unwrap_or_default();
    cap_chars(
        &[
            item.title.as_deref().unwrap_or_default(),
            item.author.as_deref().unwrap_or_default(),
            content,
        ]
        .join("\n"),
        DOCUMENT_CAP,
    )
}

/// One embedding call. The error is returned rather than swallowed: `.ok()`
/// here was the reason a pass could degrade to lexical with nothing on stderr
/// and nothing in the store to say so.
fn embed(role: &ResolvedRole, inputs: &[(String, TextRole)]) -> Result<Vec<Vec<f64>>, String> {
    sjel_runtime::require(sjel_runtime::Category::BulkIndexing).map_err(|e| e.to_string())?;
    role.embed_mixed(inputs).map(|vectors| {
        vectors
            .into_iter()
            .map(|vector| vector.into_iter().map(f64::from).collect())
            .collect()
    })
}

fn lexical_vector(text: &str) -> Vec<f64> {
    let mut vector = vec![0.0; LEXICAL_DIMENSIONS];
    for token in tokens(text) {
        let mut hasher = DefaultHasher::new();
        token.hash(&mut hasher);
        let hash = hasher.finish();
        let index = (hash as usize) % LEXICAL_DIMENSIONS;
        let sign = if hash & 1 == 0 { 1.0 } else { -1.0 };
        vector[index] += sign;
    }
    normalize(vector)
}

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| token.chars().count() >= 3)
        .map(str::to_string)
        .collect()
}

fn normalize(mut vector: Vec<f64>) -> Vec<f64> {
    let norm = vector.iter().map(|value| value * value).sum::<f64>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn cosine(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    a.iter().zip(b).map(|(left, right)| left * right).sum()
}

fn cap_chars(text: &str, cap: usize) -> String {
    text.chars().take(cap).collect()
}

fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_fields_are_read() {
        let body = "---\nsummary: \"Technical systems\"\ncurrent_focus: AI, RAG\n---\n# Lens";
        assert_eq!(
            frontmatter_value(body, "current_focus").as_deref(),
            Some("AI, RAG")
        );
    }

    #[test]
    fn explicit_relevance_query_excludes_note_scaffolding() {
        let body = "---\nsummary: Technical systems\ncurrent_focus: AI, RAG\nrelevance_query: LLM agents, retrieval, and software engineering\n---\n[[Private Link]]";
        assert_eq!(
            profile_embedding_text(
                "Polymath",
                "Technical systems",
                "AI, RAG",
                "conference",
                body
            ),
            "LLM agents, retrieval, and software engineering"
        );
    }

    #[test]
    fn lexical_fallback_uses_one_space_for_both_sides() {
        let profile = lexical_vector("AI systems architecture and open source");
        let close = lexical_vector("open source AI system architecture");
        let far = lexical_vector("cooking pottery relationship");
        assert!(cosine(&profile, &close) > cosine(&profile, &far));
    }

    #[test]
    fn item_text_is_bounded() {
        let mut item = FeedItem::new("https://example.com", "news", "article");
        item.transcript = Some("a".repeat(DOCUMENT_CAP + 100));
        assert_eq!(item_document(&item).chars().count(), DOCUMENT_CAP);
    }

    #[test]
    fn summary_replaces_transcript_for_embedding_input() {
        let mut item = FeedItem::new("https://example.com", "news", "article");
        item.summary = Some("distilled".into());
        item.transcript = Some("long raw transcript".into());
        let document = item_document(&item);
        assert!(document.contains("distilled"));
        assert!(!document.contains("long raw transcript"));
    }

    #[test]
    fn lexical_rationale_uses_the_english_surface() {
        let item = FeedItem::new("https://example.com", "news", "article");
        let profile = InterestProfile {
            key: "profile".into(),
            label: "Systems".into(),
            focus: "software architecture".into(),
            text: "software architecture".into(),
            fingerprint: "revision".into(),
        };
        let scored = score_items(&[item], &[profile], None, None);
        assert!(scored.items[0].matches[0]
            .rationale
            .starts_with("Lexical similarity for Systems"));
    }

    #[test]
    fn a_missing_embedding_role_is_an_explicit_lexical_fallback() {
        assert!(!embedding_backend_configured(None));
        assert_eq!(
            embedding_provider_label(None),
            "No embedding role configured"
        );
    }
}

/// A loopback embedding server that answers a scripted number of calls and then
/// fails, so the chunking repair can be tested without a model.
#[cfg(test)]
mod stub_embedding {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// Bodies of every embedding request the stub received, in order.
    pub(super) type Recorded = Arc<Mutex<Vec<String>>>;

    /// Serve on 127.0.0.1. `succeed_calls` embedding calls answer 200 with one
    /// vector per input; every later call answers 500, which is what a
    /// half-available embedding server looks like from here.
    pub(super) fn start(succeed_calls: usize) -> (String, Recorded) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free loopback port");
        let port = listener.local_addr().expect("a bound address").port();
        let recorded: Recorded = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&recorded);
        std::thread::spawn(move || {
            let mut embed_calls = 0usize;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().expect("a clone"));
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
                let mut length = 0usize;
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
                        break;
                    }
                    if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:")
                    {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; length];
                let _ = reader.read_exact(&mut body);
                let body = String::from_utf8_lossy(&body).to_string();
                let is_embedding = request_line.contains("/embeddings");
                let response = if !is_embedding {
                    // The reachability probe. Refusing it is honest: this stub
                    // is not a model server.
                    "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n".to_string()
                } else {
                    sink.lock().expect("the record").push(body.clone());
                    embed_calls += 1;
                    if embed_calls > succeed_calls {
                        "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n"
                            .to_string()
                    } else {
                        let parsed: serde_json::Value =
                            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
                        let count = parsed["input"].as_array().map(Vec::len).unwrap_or(0);
                        let data = (0..count)
                            .map(|index| {
                                serde_json::json!({
                                    "index": index,
                                    "embedding": [1.0, 0.5, 0.25, 0.125],
                                })
                            })
                            .collect::<Vec<_>>();
                        let payload = serde_json::json!({ "data": data }).to_string();
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
                            payload.len()
                        )
                    }
                };
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        (format!("http://127.0.0.1:{port}/v1"), recorded)
    }
}

#[cfg(test)]
mod repair_tests {
    use super::*;
    use crate::config::RelevanceConfig;
    use sjel_inference::{Api, Backend};

    fn role(base_url: &str) -> ResolvedRole {
        ResolvedRole {
            backend_name: "stub".into(),
            backend: Backend {
                api: Api::OpenAi,
                base_url: base_url.into(),
                api_key_file: None,
                provided_by: None,
            },
            model: "stub-embed".into(),
            provider_name: None,
            cloud_data_tier: None,
            billing_mode: None,
            failover_priority: None,
            max_requests_per_day: None,
            max_input_tokens: None,
            credit_expires_on: None,
            query_prefix: String::new(),
            document_prefix: String::new(),
            chat_template_kwargs: None,
            request_overrides: None,
            trusted_peer: false,
        }
    }

    fn profile() -> InterestProfile {
        InterestProfile {
            key: "lens".into(),
            label: "Systems".into(),
            focus: "software architecture".into(),
            text: "software architecture".into(),
            fingerprint: "revision".into(),
        }
    }

    fn item(index: usize, class: &str) -> FeedItem {
        let mut item = FeedItem::new(&format!("https://example.com/{index}"), "news", "article");
        item.title = Some(format!("Item {index} about architecture"));
        item.summary = Some("A bounded summary".into());
        item.data_class = class.into();
        item
    }

    #[test]
    fn zero_declared_paths_is_not_an_error() {
        // Every worktree, every CI run and every fresh install is this state.
        assert_eq!(
            load_profiles(&RelevanceConfig::default()).map(|profiles| profiles.len()),
            Ok(0),
            "a machine that declares no TELOS lens must not 500 its feed"
        );

        let declared = RelevanceConfig {
            profile_paths: vec!["/nonexistent/telos/lenses".into()],
        };
        assert!(
            load_profiles(&declared).is_err(),
            "a declared path that has moved is a fault, not zero lenses"
        );

        let empty = std::env::temp_dir().join(format!("comms-lenses-{}", std::process::id()));
        std::fs::create_dir_all(&empty).expect("a writable temp directory");
        let declared_empty = RelevanceConfig {
            profile_paths: vec![empty.to_string_lossy().to_string()],
        };
        assert!(
            load_profiles(&declared_empty).is_err(),
            "a declared directory holding no lens yields no scores and must say so"
        );
    }

    #[test]
    fn one_failed_chunk_does_not_downgrade_the_pass() {
        // Two calls answer: the profile set and the first chunk of 32. The
        // second chunk fails. Before the repair one failure took the pass.
        let (base_url, _recorded) = stub_embedding::start(2);
        let role = role(&base_url);
        let items = (0..40).map(|index| item(index, "c0")).collect::<Vec<_>>();
        let outcome = score_items(&items, &[profile()], Some(&role), None);

        assert_eq!(outcome.mode, "semantic");
        assert_eq!(outcome.chunks, 3, "one profile call plus two item chunks");
        assert_eq!(outcome.chunks_failed, 1);
        assert!(
            outcome.error_class.is_some_and(|class| !class.is_empty()),
            "a degraded pass must carry an error class"
        );
        for scored in outcome.items.iter().take(EMBED_CHUNK_SIZE) {
            assert_eq!(scored.matches[0].mode, "semantic");
        }
        for scored in outcome.items.iter().skip(EMBED_CHUNK_SIZE) {
            assert_eq!(scored.matches[0].mode, "lexical");
        }
    }

    #[test]
    fn a_c3_item_reaches_neither_the_embedder_nor_the_lexical_scorer() {
        let (base_url, recorded) = stub_embedding::start(10);
        let role = role(&base_url);
        let mut refused = item(1, "c3");
        refused.title = Some("Refused private correspondence".into());
        refused.summary = Some("Refused private body".into());
        let allowed = item(2, "c0");
        let allowed_title = allowed.title.clone().expect("a title");
        let outcome = score_items(&[refused, allowed], &[profile()], Some(&role), None);

        assert!(outcome.items[0].refused_class);
        assert!(
            outcome.items[0].matches.is_empty(),
            "a refused item gets no score from either path"
        );
        assert!(!outcome.items[1].refused_class);
        assert!(!outcome.items[1].matches.is_empty());

        let sent = recorded.lock().expect("the record").join("\n");
        assert!(
            !sent.contains("Refused private"),
            "a refused item must not reach the embedder at all"
        );
        assert!(
            sent.contains(&allowed_title),
            "the allowed item is what the embedder saw"
        );
    }
}
