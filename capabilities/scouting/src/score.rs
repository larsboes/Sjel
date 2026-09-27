use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::embed::{embedding_role, read_cache, try_embed_profiles};
use crate::opportunity::{Opportunity, OpportunityType};
use crate::sources::SourceManifest;
use serde::Deserialize;

const EMBED_DIM: usize = 768;

const EMBED_PREFIX: &str = "query: ";
const PASSAGE_PREFIX: &str = "passage: ";

#[derive(Debug, Deserialize)]
struct EmbeddedOpportunity {
    id: String,
    embedding: Vec<f32>,
    #[serde(flatten)]
    #[allow(dead_code)]
    fields: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ScoredOpportunity {
    pub opportunity: Opportunity,
    pub score: f64,
    pub rationale: String,
    pub matched_focus: Option<String>,
}

/// One profile = one interest/focus area to score opportunities against
/// (e.g. "Career", "Polymath", "Travel"). Built from markdown files in the
/// configured `interest_profile_dir` (see config.rs) -- the convention
/// (`summary:`/`current_focus:` frontmatter + a `> [!quote] Charter`
/// blockquote) originated from an Obsidian TELOS vault but is generic:
/// any directory of markdown files following that shape works.
#[derive(Debug, Clone)]
pub struct TelosProfile {
    pub focus_name: String,
    pub vector: Vec<f32>,
    pub source: String,
    pub category_affinity: Vec<String>,
    /// What this profile is a predicate *for*, taken from the source manifest
    /// that declared it. `None` competes for everything, which is what the
    /// legacy profile directory gets since nothing declares a type for it.
    ///
    /// Without this every profile competed for every opportunity, and the
    /// scholarship profile won the top of an events sweep: its embedded text
    /// is about funding candidates and applications, which is a close match
    /// for anything with "Legal", "Essentials" or "Foundation" in the title.
    /// The types were in `scouting.json` the whole time; the scorer just never
    /// read them.
    pub opportunity_type: Option<OpportunityType>,
}

impl TelosProfile {
    /// A profile competes for an opportunity when it declares that type, or
    /// when it declares none at all.
    pub fn competes_for(&self, opportunity_type: OpportunityType) -> bool {
        match self.opportunity_type {
            None => true,
            Some(declared) => declared == opportunity_type,
        }
    }
}

/// Loads interest profiles from the legacy `interest_profile_dir` AND from all
/// configured source manifests that declare a `profiles_glob`. Profiles from both
/// sources are merged into a single list (sources take precedence on name collision).
///
/// If a pre-computed embeddings file exists at `<interest_profile_dir>/telos_vectors.json`
/// and names the producer this machine would use, those real vectors are used;
/// otherwise the `embedding` role computes them, and failing that it falls back to
/// deterministic hash-embedding so the pipeline always runs with zero ML dependencies.
pub fn load_telos_profiles(
    interest_profile_dir: &str,
    sources: &[SourceManifest],
) -> Vec<TelosProfile> {
    let mut profiles = Vec::new();
    let mut seen_names = HashSet::new();
    let mut unembedded: Vec<(String, String, Vec<String>, Option<OpportunityType>)> = Vec::new(); // (name, text, affinity, declared type)

    // Resolved once: every cache read below has to prove it was written by
    // this same model, and no role at all means there is nothing to prove
    // against, so no cache is trusted and everything hashes.
    let role = embedding_role();
    let producer = role.as_ref().map(|role| role.cache_key());

    // Helper: read profiles from a directory or one exact file, separating
    // cached vs. unembedded.
    let mut collect_from_path =
        |path: &Path, vectors_cache: Option<&Path>, declared: Option<OpportunityType>| {
            if !path.is_dir() && !path.is_file() {
                return;
            }

            // Try pre-computed vectors from cache, but only this model's
            if let (Some(cache_path), Some(producer)) = (vectors_cache, producer.as_deref()) {
                if cache_path.exists() {
                    for mut p in load_real_e5_vectors(cache_path, producer) {
                        // The cache is written per directory, so a cached profile
                        // takes the type of the source asking for it now.
                        p.opportunity_type = p.opportunity_type.or(declared);
                        if seen_names.insert(p.focus_name.clone()) {
                            profiles.push(p);
                        }
                    }
                    return; // cache covered everything in this directory
                }
            }

            // No cache — hash-fallback profiles, but collect texts for embedding
            let paths: Vec<PathBuf> = if path.is_file() {
                vec![path.to_path_buf()]
            } else {
                match fs::read_dir(path) {
                    Ok(entries) => entries.flatten().map(|entry| entry.path()).collect(),
                    Err(_) => return,
                }
            };
            for path in paths {
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                if name == "Focus" {
                    continue;
                } // MOC hub, skip
                if seen_names.contains(name) {
                    continue;
                }
                seen_names.insert(name.to_string());

                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                let memo = extract_memo(&text);
                let affinity = parse_category_affinity(&text);
                unembedded.push((name.to_string(), memo, affinity, declared));
            }
        };

    // 1. Legacy interest_profile_dir
    let legacy_dir = Path::new(interest_profile_dir);
    let legacy_cache = legacy_dir.join("telos_vectors.json");
    collect_from_path(legacy_dir, Some(&legacy_cache), None);

    // 2. Source-declared profiles. The root is the source's own `profile_path`
    //    when it declares one and its opportunity `path` otherwise, so a
    //    matching predicate no longer has to live in the knowledge store the
    //    opportunities happen to come from.
    let legacy_cache_real = std::fs::canonicalize(&legacy_cache).ok();
    for src in sources {
        if !src.enabled {
            continue;
        }
        let (root, glob) = match src.profile_location() {
            Ok(Some(found)) => found,
            Ok(None) => continue,
            // Loudly, because a profile that stops being applied changes every
            // score in the run and nothing else downstream would mention it.
            Err(detail) => {
                eprintln!("warning: {detail} -- profile not loaded");
                continue;
            }
        };
        let profiles_path = match root.locate(glob) {
            Ok(path) => path,
            Err(detail) => {
                eprintln!(
                    "warning: source '{}' profiles_glob '{glob}': {detail}",
                    src.id
                );
                continue;
            }
        };

        let src_cache = profiles_path.join("telos_vectors.json");
        // Only pass a cache path when it is not the legacy one (avoids
        // re-reading the same file when legacy and source point at one dir).
        // Compared through the filesystem rather than as strings: the source
        // side is canonical now and the legacy side is whatever config said.
        // Exact profile files never consume a directory-wide cache — doing so
        // would quietly load sibling profiles and defeat the exact-file contract.
        let is_legacy_cache = std::fs::canonicalize(&src_cache)
            .ok()
            .is_some_and(|real| Some(&real) == legacy_cache_real.as_ref());
        let cache = if profiles_path.is_file() || is_legacy_cache {
            None
        } else {
            Some(src_cache.as_path())
        };
        collect_from_path(&profiles_path, cache, Some(src.opportunity_type));
    }

    // 3. Try to embed any unembedded profiles via the configured backend
    if !unembedded.is_empty() {
        let cache_path = if legacy_dir.is_dir() {
            Some(legacy_cache.as_path())
        } else {
            None
        };
        let texts: Vec<(String, String, Vec<String>)> = unembedded
            .iter()
            .map(|(name, text, affinity, _)| (name.clone(), text.clone(), affinity.clone()))
            .collect();
        if let Some(mut embedded) = try_embed_profiles(&texts, role.as_ref(), cache_path) {
            for profile in &mut embedded {
                profile.opportunity_type = unembedded
                    .iter()
                    .find(|(name, ..)| name == &profile.focus_name)
                    .and_then(|(.., declared)| *declared);
            }
            // Replace hash-fallback placeholders with real vectors
            for e in embedded {
                if let Some(existing) = profiles
                    .iter_mut()
                    .find(|p: &&mut TelosProfile| p.focus_name == e.focus_name)
                {
                    existing.vector = e.vector;
                    existing.category_affinity = e.category_affinity;
                } else {
                    profiles.push(e);
                }
            }
            return profiles;
        }

        // Embedding unavailable — use hash vectors for unembedded profiles
        for (name, text, affinity, declared) in &unembedded {
            let vector = hash_embed(text);
            profiles.push(TelosProfile {
                focus_name: name.clone(),
                vector,
                source: text.clone(),
                category_affinity: affinity.clone(),
                opportunity_type: *declared,
            });
        }
    }

    profiles
}

fn parse_category_affinity(md: &str) -> Vec<String> {
    md.lines()
        .find_map(|l| {
            l.strip_prefix("category_affinity:").map(|s| {
                s.split(',')
                    .map(|c| c.trim().trim_matches('"').to_lowercase())
                    .filter(|c| !c.is_empty())
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_default()
}

/// Cached vectors, but only the ones this run's model produced.
///
/// `expected_producer` is `ResolvedRole::cache_key()`. An empty result means
/// "recompute", which is what a producer mismatch has to mean: serving e5
/// vectors to a nomic run is wrong in a way nothing downstream can detect.
fn load_real_e5_vectors(path: &std::path::Path, expected_producer: &str) -> Vec<TelosProfile> {
    let Some(cache) = read_cache(path, expected_producer) else {
        return Vec::new();
    };
    cache
        .profiles
        .into_iter()
        .map(|(name, entry)| TelosProfile {
            focus_name: name,
            vector: entry.vector,
            source: entry.text,
            // The cache predates types; the caller stamps the source's own.
            opportunity_type: None,
            category_affinity: entry.category_affinity,
        })
        .collect()
}

#[allow(dead_code)]
fn load_hash_fallback_profiles(interest_profile_dir: &str) -> Vec<TelosProfile> {
    let mut profiles = Vec::new();
    let Ok(entries) = fs::read_dir(interest_profile_dir) else {
        return profiles;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        // "Focus" is a hub/lens file in the original TELOS convention, not an
        // interest profile itself -- skip it if present so it doesn't dilute
        // scoring (see LifeOS-mono scouting ISA.md, ISC-45).
        if name == "Focus" {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let memo = extract_memo(&text);
        let vector = hash_embed(&memo);
        profiles.push(TelosProfile {
            focus_name: name.into(),
            vector,
            source: memo,
            opportunity_type: None,
            category_affinity: Vec::new(),
        });
    }
    profiles
}

fn extract_memo(md: &str) -> String {
    let lines: Vec<&str> = md.lines().collect();
    let summary = lines
        .iter()
        .find_map(|l| {
            l.strip_prefix("summary:")
                .map(|s| s.trim().trim_matches('"').to_string())
        })
        .unwrap_or_default();
    let focus = lines
        .iter()
        .find_map(|l| {
            l.strip_prefix("current_focus:")
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_default();

    let mut charter_parts: Vec<String> = Vec::new();
    let mut in_charter = false;

    for line in &lines {
        if line.starts_with("> [!quote] Charter") {
            in_charter = true;
            continue;
        } else if in_charter {
            if let Some(content) = line.strip_prefix("> ") {
                charter_parts.push(content.trim().to_string());
            } else if !line.starts_with(">") {
                in_charter = false;
            }
        }
    }

    let charter = charter_parts.join(" ");

    let parts: Vec<String> = vec![summary, focus, charter]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect();
    parts.join(" ")
}

pub fn hash_embed(text: &str) -> Vec<f32> {
    let mut vec = vec![0.0f32; EMBED_DIM];
    for token in text.split(|c: char| !c.is_alphanumeric()) {
        if token.is_empty() {
            continue;
        }
        let mut h = 0u64;
        for b in token.to_lowercase().as_bytes() {
            h = h.wrapping_mul(31).wrapping_add(*b as u64);
        }
        let idx = (h % EMBED_DIM as u64) as usize;
        vec[idx] += 1.0;
    }
    let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut vec {
            *v /= norm;
        }
    }
    vec
}

/// Cosine similarity, which this was named after but did not compute: it was
/// a bare dot product. That approximated cosine only because embedding
/// backends hand back roughly unit-length vectors, and it stopped
/// approximating anything once `batch_centre` subtracted the shared component
/// and left vectors of wildly different magnitude. Then a long text scored
/// high for being long.
///
/// Zero-length input yields 0.0 rather than NaN: an entry with no signal is
/// not similar to anything, and a NaN would sort unpredictably.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b = b.iter().map(|y| y * y).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// The text an opportunity is matched on. Public so the batch embedder and
/// the scorer cannot drift apart on what "the opportunity" means.
pub fn opportunity_text(opp: &Opportunity) -> String {
    let field = |key: &str| {
        opp.raw
            .get(key)
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string()
    };
    format!(
        "{} {} {} {} {} {}",
        opp.title,
        field("category"),
        field("description"),
        opp.location.as_deref().unwrap_or(""),
        opp.city.as_deref().unwrap_or(""),
        opp.url
    )
}

/// Embeds every opportunity in one request, as the *document* side of the
/// retrieval pair.
///
/// Until this existed only the interest profile could ever carry a real
/// vector: the opportunity half was hashed unless someone passed a
/// pre-computed file, so every cosine was half hash no matter which backend
/// was configured. `None` means the caller keeps hashing, which is a
/// degradation, not a failure.
pub fn embed_opportunities(
    opportunities: &[Opportunity],
    role: &sjel_inference::ResolvedRole,
) -> Option<std::collections::HashMap<String, Vec<f32>>> {
    if opportunities.is_empty() {
        return Some(std::collections::HashMap::new());
    }
    let texts: Vec<String> = opportunities.iter().map(opportunity_text).collect();
    match role.embed(&texts, sjel_inference::TextRole::Document) {
        Ok(vectors) => Some(
            opportunities
                .iter()
                .map(|opp| opp.id.clone())
                .zip(vectors)
                .collect(),
        ),
        Err(error) => {
            eprintln!(
                "  embed: opportunity side unreachable on '{}' ({error}) — hashing instead",
                role.backend_name
            );
            None
        }
    }
}

pub fn score(
    opportunities: &[Opportunity],
    telos: &[TelosProfile],
    opp_embeddings: Option<&std::collections::HashMap<String, Vec<f32>>>,
) -> Vec<ScoredOpportunity> {
    score_labelled(opportunities, telos, opp_embeddings, "e5")
}

/// `producer_label` names what produced the opportunity vectors, so the
/// rationale can say `omlx:multilingual-e5-base-mlx` rather than a hardcoded
/// "e5" that was right exactly once.
/// Below this a mean is noise rather than a common component, so nothing is
/// subtracted and the raw cosine stands.
const MIN_BATCH_FOR_CENTRING: usize = 8;

/// The batch's common component, or `None` when there is too little to average.
///
/// Retrieval embeddings carry a large direction shared by every text in the
/// corpus. With `multilingual-e5` it dominates: measured on a live Luma sweep,
/// a Berlin community run and an agentic-payments talk scored 0.822 and 0.821
/// against the same profile, and swapping that profile for a completely
/// different one barely moved the order. Almost all of the cosine was the
/// shared direction, and the part that carries the actual signal was in the
/// third decimal.
///
/// Subtracting the mean removes it, and what is left is how each opportunity
/// differs *from the others*, which is the question a ranking asks. Scores can
/// go negative afterwards; below the batch average is a real answer.
fn batch_centre(vectors: &[Vec<f32>]) -> Option<Vec<f32>> {
    if vectors.len() < MIN_BATCH_FOR_CENTRING {
        return None;
    }
    let dim = vectors.iter().map(Vec::len).min()?;
    if dim == 0 {
        return None;
    }
    let mut centre = vec![0.0f32; dim];
    for vector in vectors {
        for (slot, value) in centre.iter_mut().zip(&vector[..dim]) {
            *slot += value;
        }
    }
    let n = vectors.len() as f32;
    for slot in &mut centre {
        *slot /= n;
    }
    Some(centre)
}

fn centred(vector: &[f32], centre: Option<&[f32]>) -> Vec<f32> {
    match centre {
        None => vector.to_vec(),
        Some(centre) => {
            let dim = vector.len().min(centre.len());
            vector[..dim]
                .iter()
                .zip(&centre[..dim])
                .map(|(value, mean)| value - mean)
                .collect()
        }
    }
}

pub fn score_labelled(
    opportunities: &[Opportunity],
    telos: &[TelosProfile],
    opp_embeddings: Option<&std::collections::HashMap<String, Vec<f32>>>,
    producer_label: &str,
) -> Vec<ScoredOpportunity> {
    if telos.is_empty() {
        return opportunities
            .iter()
            .map(|o| ScoredOpportunity {
                opportunity: o.clone(),
                score: 0.0,
                rationale: "no TELOS profile loaded: uncategorized".into(),
                matched_focus: None,
            })
            .collect();
    }

    // Every vector the batch produced, so the common component can be found
    // before any comparison happens.
    let all_vectors: Vec<Vec<f32>> = opportunities
        .iter()
        .map(|opp| {
            let text = opportunity_text(opp);
            opp_embeddings
                .and_then(|map| map.get(&opp.id).cloned())
                .unwrap_or_else(|| hash_embed(&text))
        })
        .collect();
    let centre = batch_centre(&all_vectors);

    let mut scored = Vec::with_capacity(opportunities.len());
    for (index, opp) in opportunities.iter().enumerate() {
        let category = opp
            .raw
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let description = opp
            .raw
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let _ = (category, description);
        let opp_vec: Vec<f32> = centred(&all_vectors[index], centre.as_deref());

        let category = opp
            .raw
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // Only the profiles that are a predicate for *this kind of thing*. A
        // scholarship profile has no business winning an events sweep, and
        // before this filter it did: it took the top three slots because its
        // text is about applications and eligibility, which reads close to any
        // title with "Legal", "Essentials" or "Foundation" in it.
        let mut best: Option<(&TelosProfile, f32)> = None;
        for profile in telos
            .iter()
            .filter(|profile| profile.competes_for(opp.opportunity_type))
        {
            // The profile is centred against the same batch, so both sides sit
            // in the same recentred space.
            let profile_vec = centred(&profile.vector, centre.as_deref());
            let dim = profile_vec.len().min(opp_vec.len());
            let mut s = cosine(&profile_vec[..dim], &opp_vec[..dim]);
            if !profile.category_affinity.is_empty() && !category.is_empty() {
                let cat_lower = category.to_lowercase();
                if !profile.category_affinity.iter().any(|a| a == &cat_lower) {
                    // Has to push down whichever side of zero it is on.
                    // Multiplying alone only shrinks positives; on a centred
                    // score a negative would be moved *up* toward the mean.
                    s = if s > 0.0 { s * 0.1 } else { s * 10.0 };
                }
            }
            match best {
                None => best = Some((profile, s)),
                Some((_, bs)) if s > bs => best = Some((profile, s)),
                _ => {}
            }
        }

        let Some((profile, sim)) = best else {
            // No profile declares this type. Saying so is better than scoring
            // it against a predicate written for something else.
            scored.push(ScoredOpportunity {
                opportunity: opp.clone(),
                score: 0.0,
                rationale: format!(
                    "no interest profile declared for {}: unscored",
                    opp.opportunity_type.as_str()
                ),
                matched_focus: None,
            });
            continue;
        };
        let score = f64::from(sim);
        let embedding_mode = if opp_embeddings.is_some() {
            producer_label
        } else {
            "hash-fallback"
        };
        let rationale = if score > 0.01 {
            format!(
                "matched focus '{}' (cosine={:.3}, {embedding_mode}): event text overlaps with interest profile",
                profile.focus_name, score
            )
        } else {
            format!(
                "low fit against '{}' (cosine={:.3}, {embedding_mode}): little topical overlap",
                profile.focus_name, score
            )
        };

        scored.push(ScoredOpportunity {
            opportunity: opp.clone(),
            score,
            rationale,
            matched_focus: Some(profile.focus_name.clone()),
        });
    }

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored
}

pub fn load_opp_embeddings(path: &str) -> std::collections::HashMap<String, Vec<f32>> {
    let mut map = std::collections::HashMap::new();
    let Ok(body) = fs::read_to_string(path) else {
        return map;
    };
    let Ok(embedded): Result<Vec<EmbeddedOpportunity>, _> = serde_json::from_str(&body) else {
        return map;
    };
    for e in embedded {
        map.insert(e.id, e.embedding);
    }
    map
}

#[allow(dead_code)]
pub fn _embed_prefix() -> &'static str {
    EMBED_PREFIX
}

#[allow(dead_code)]
pub fn _passage_prefix() -> &'static str {
    PASSAGE_PREFIX
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measured problem: with multilingual-e5 a batch of unrelated texts
    /// all score ~0.82 against the same profile, because most of the cosine is
    /// a direction every embedding shares. Centring has to pull the
    /// differences out of the third decimal.
    ///
    /// It reorders, and that is the point rather than a side effect: an
    /// ordering dominated by a component every candidate carries equally is
    /// not an ordering. This fixture reproduces the shape — a large shared
    /// axis, two small signal axes — and the numbers below are its actual
    /// behaviour, not a target invented for the test.
    #[test]
    fn centring_pulls_the_signal_out_of_the_shared_component() {
        let items: Vec<Vec<f32>> = (0..10)
            .map(|i| vec![10.0, 0.30 + 0.02 * i as f32, 0.30 - 0.015 * i as f32])
            .collect();
        let profile = vec![10.0, 0.45, 0.20];
        let centre = batch_centre(&items).expect("ten is enough to average");

        let spread = |xs: &[f32]| {
            xs.iter().cloned().fold(f32::MIN, f32::max)
                - xs.iter().cloned().fold(f32::MAX, f32::min)
        };
        let before = spread(
            &items
                .iter()
                .map(|v| cosine(&profile, v))
                .collect::<Vec<_>>(),
        );
        let after = spread(
            &items
                .iter()
                .map(|v| {
                    cosine(
                        &centred(&profile, Some(&centre)),
                        &centred(v, Some(&centre)),
                    )
                })
                .collect::<Vec<_>>(),
        );

        assert!(
            before < 0.001,
            "the shared axis should flatten everything: {before}"
        );
        assert!(
            after > 0.5,
            "centring should expose the difference: {after}"
        );
    }

    /// A mean over three points is noise, not a common component.
    #[test]
    fn a_small_batch_is_left_alone() {
        let few = vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![1.0, 1.0]];
        assert!(batch_centre(&few).is_none());
        let v = vec![1.0, 2.0];
        assert_eq!(
            centred(&v, None),
            v,
            "no centre means the raw vector stands"
        );
    }

    /// The defect this filter exists for: the scholarship profile took the
    /// top three slots of an events sweep, because its embedded text is about
    /// funding candidates and eligibility, which sits close to any title with
    /// "Legal", "Essentials" or "Foundation" in it.
    #[test]
    fn a_profile_only_competes_for_the_type_it_declares() {
        let scholarship = TelosProfile {
            focus_name: "Scholarship Profile".into(),
            vector: vec![1.0, 0.0],
            source: "funding candidate".into(),
            category_affinity: vec![],
            opportunity_type: Some(OpportunityType::Scholarship),
        };
        assert!(!scholarship.competes_for(OpportunityType::Event));
        assert!(scholarship.competes_for(OpportunityType::Scholarship));
    }

    /// The legacy profile directory declares nothing, so it must keep working
    /// for everything rather than silently scoring nothing.
    #[test]
    fn an_undeclared_profile_still_competes_for_everything() {
        let anything = TelosProfile {
            focus_name: "legacy".into(),
            vector: vec![1.0, 0.0],
            source: "whatever".into(),
            category_affinity: vec![],
            opportunity_type: None,
        };
        assert!(anything.competes_for(OpportunityType::Event));
        assert!(anything.competes_for(OpportunityType::Trip));
    }

    #[test]
    fn hash_embed_is_deterministic() {
        assert_eq!(
            hash_embed("AI hackathon Berlin"),
            hash_embed("AI hackathon Berlin")
        );
        assert_ne!(
            hash_embed("AI hackathon Berlin"),
            hash_embed("cooking class Munich")
        );
    }

    #[test]
    fn empty_telos_returns_uncategorized() {
        let opp = Opportunity {
            id: "x".into(),
            opportunity_type: crate::opportunity::OpportunityType::Event,
            source: "s".into(),
            source_kind: crate::opportunity::SourceKind::Api,
            url: "u".into(),
            title: "Test".into(),
            starts_at: None,
            ends_at: None,
            location: None,
            city: None,
            country_code: None,
            latitude: None,
            longitude: None,
            raw: serde_json::Value::Null,
            fetched_at: "t".into(),
        };
        let scored = score(&[opp], &[], None);
        assert_eq!(
            scored[0].rationale,
            "no TELOS profile loaded: uncategorized"
        );
    }

    #[test]
    fn cosine_bounds() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine(&a, &b) - 1.0).abs() < 1e-6);
        let c = vec![0.0, 1.0, 0.0];
        assert!(cosine(&a, &c).abs() < 1e-6);

        // The case the old dot-product version got wrong: length must not be
        // mistaken for similarity. These fixtures were all unit vectors, which
        // is why the bug survived its own test.
        let long = vec![5.0, 0.0, 0.0];
        assert!(
            (cosine(&a, &long) - 1.0).abs() < 1e-6,
            "same direction is the same similarity whatever the magnitude"
        );
        let opposite = vec![-1.0, 0.0, 0.0];
        assert!((cosine(&a, &opposite) + 1.0).abs() < 1e-6);
        assert_eq!(
            cosine(&a, &[0.0, 0.0, 0.0]),
            0.0,
            "no signal, no similarity"
        );
    }

    // -----------------------------------------------------------------------
    // Where a profile is allowed to live (#230). The predicate a source scores
    // against is a consumer input, not a required resident of whichever
    // knowledge store the opportunity notes came from.
    // -----------------------------------------------------------------------

    use crate::opportunity::OpportunityType;
    use crate::sources::{SourceEntry, SourceManifest};

    /// A throwaway tree under the OS temp dir, removed on drop.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("axon-scouting-profile-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp tree");
            Tree(dir)
        }

        fn file(&self, relative: &str, body: &str) -> PathBuf {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("parent");
            }
            std::fs::write(&path, body).expect("write");
            path
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn entry(
        path: Option<&Path>,
        profile_path: Option<&Path>,
        glob: Option<&str>,
    ) -> SourceManifest {
        let json = serde_json::json!({
            "id": "events-radar",
            "adapter": "obsidian-markdown",
            "path": path.map(|p| p.to_string_lossy().into_owned()),
            "profile_path": profile_path.map(|p| p.to_string_lossy().into_owned()),
            "profiles_glob": glob,
        });
        serde_json::from_value::<SourceEntry>(json)
            .expect("source entry")
            .resolve()
    }

    /// The compatible form: everything under one root, which is every source
    /// entry written before `profile_path` existed.
    #[test]
    fn one_root_still_resolves_the_profile_the_way_it_always_did() {
        let vault = Tree::new("same-root");
        vault.file("TELOS/Personal/Events Profile.md", "# focus");
        let manifest = entry(
            Some(&vault.0),
            None,
            Some("TELOS/Personal/Events Profile.md"),
        );

        let (root, glob) = manifest
            .profile_location()
            .expect("resolvable")
            .expect("a profile is declared");
        let located = root.locate(glob).expect("located");
        assert!(located.is_file());
        assert!(located.ends_with("Events Profile.md"));
    }

    /// The point of the issue: the profile lives in one system, the
    /// opportunities in another, and neither has to know about the other.
    #[test]
    fn a_profile_resolves_under_its_own_root_when_one_is_declared() {
        let vault = Tree::new("split-opportunities");
        vault.file("Applications/a.md", "note");
        let profiles = Tree::new("split-profiles");
        profiles.file("Events Profile.md", "# focus");

        let manifest = entry(Some(&vault.0), Some(&profiles.0), Some("Events Profile.md"));
        let (root, glob) = manifest
            .profile_location()
            .expect("resolvable")
            .expect("a profile is declared");

        assert_eq!(root.path(), std::fs::canonicalize(&profiles.0).unwrap());
        let located = root.locate(glob).expect("located");
        assert!(
            located.starts_with(root.path()),
            "the profile resolves under the profile root, not the vault"
        );
    }

    /// Moving a profile changes nothing about the source it belongs to.
    #[test]
    fn moving_a_profile_leaves_source_identity_and_type_untouched() {
        let vault = Tree::new("identity-vault");
        let profiles = Tree::new("identity-profiles");
        profiles.file("Events Profile.md", "# focus");

        let together = entry(Some(&vault.0), None, Some("Events Profile.md"));
        let apart = entry(Some(&vault.0), Some(&profiles.0), Some("Events Profile.md"));

        assert_eq!(together.id, apart.id);
        assert_eq!(together.opportunity_type, apart.opportunity_type);
        assert_eq!(together.root_path, apart.root_path);
    }

    /// A glob with nothing to resolve against used to be a silent skip.
    #[test]
    fn a_glob_with_no_root_at_all_is_a_named_config_error() {
        let manifest = entry(None, None, Some("Events Profile.md"));
        let detail = manifest
            .profile_location()
            .expect_err("neither root is declared");
        assert!(
            detail.contains("events-radar"),
            "the error names the source"
        );
        assert!(detail.contains("profile_path"), "and the way out");
    }

    #[test]
    fn a_profile_root_that_is_not_there_says_so_rather_than_scoring_blind() {
        let missing = std::env::temp_dir().join("axon-scouting-profile-gone-9f8e7d");
        let _ = std::fs::remove_dir_all(&missing);
        let vault = Tree::new("missing-profile-root");

        let detail = entry(Some(&vault.0), Some(&missing), Some("Events Profile.md"))
            .profile_location()
            .expect_err("a declared root that does not exist");
        assert!(detail.contains("events-radar"));
    }

    /// An absent profile file inside a real root stays ordinary: the caller
    /// decides that means "no profile", which is what the network adapters do.
    #[test]
    fn an_absent_profile_file_inside_a_real_root_is_not_an_error() {
        let profiles = Tree::new("absent-profile-file");
        let manifest = entry(None, Some(&profiles.0), Some("Nowhere Profile.md"));
        let (root, glob) = manifest.profile_location().expect("root is fine").unwrap();
        let located = root.locate(glob).expect("located");
        assert!(!located.exists());
    }

    #[test]
    fn a_profile_glob_cannot_climb_out_of_its_declared_root() {
        let profiles = Tree::new("profile-traversal");
        let manifest = entry(None, Some(&profiles.0), Some("../../*.md"));
        let (root, glob) = manifest.profile_location().expect("root is fine").unwrap();
        assert!(
            root.locate(glob).is_err(),
            "a profile glob is bounded by the root that declared it"
        );
    }

    #[test]
    fn a_source_declaring_no_profile_is_not_an_error() {
        let vault = Tree::new("no-profile");
        assert!(entry(Some(&vault.0), None, None)
            .profile_location()
            .expect("no profile is ordinary")
            .is_none());
    }
}
