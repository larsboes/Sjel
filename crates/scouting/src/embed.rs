//! Interest-profile vectors, via whichever backend this machine runs.
//!
//! This module used to own an Ollama client and a hardcoded
//! `localhost:11434`/`nomic-embed-text` default. It owns neither now: the
//! backend, the model and that model's input prefixes come from the overlay's
//! `inference.json` through `libs/inference`, and what stays here is the part
//! that knows what an *interest profile* is.
//!
//! Resolution order:
//!   1. A `telos_vectors.json` cache written **by the same model** → use it.
//!   2. The `embedding` role, if this machine declares one → compute, cache.
//!   3. Otherwise → `None`, and `score.rs` falls back to `hash_embed()`.
//!
//! Step 1's "by the same model" is why [`VectorCache`] carries a producer. A
//! cache keyed on the profile alone will happily serve `multilingual-e5`
//! vectors to a `nomic-embed-text` run after a backend switch: every score
//! wrong, nothing logged, no error anywhere. So the cache names its producer
//! and a mismatch recomputes.
//!
//! See `capabilities/scouting/README.md`.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::score::TelosProfile;
use sjel_config::overlay_config;
use sjel_inference::{InferenceConfig, ResolvedRole, TextRole};

/// The role name this capability asks for. Declared in `inference.json`.
pub const EMBEDDING_ROLE: &str = "embedding";

/// Resolves the embedding role for this machine, or `None` when none is
/// declared — a normal state, not an error. The caller degrades to hashing.
pub fn embedding_role() -> Option<ResolvedRole> {
    InferenceConfig::load(overlay_config).role(EMBEDDING_ROLE)
}

/// Try to compute real embedding vectors for a set of profile texts.
///
/// `Some` means real vectors; `None` tells the caller to use the hash
/// fallback. An interest profile is the *query* side of the retrieval pair —
/// the opportunity text it gets compared against is the document side — so the
/// role's query prefix is the one that goes on here.
pub fn try_embed_profiles(
    profiles: &[(String, String, Vec<String>)], // (focus_name, text, category_affinity)
    role: Option<&ResolvedRole>,
    cache_path: Option<&Path>,
) -> Option<Vec<TelosProfile>> {
    if profiles.is_empty() {
        return Some(Vec::new());
    }
    let role = match role {
        Some(role) => role,
        None => {
            eprintln!(
                "  embed: no '{EMBEDDING_ROLE}' role declared for this machine \
                 — falling back to hash embedding"
            );
            return None;
        }
    };

    let texts: Vec<String> = profiles.iter().map(|(_, text, _)| text.clone()).collect();
    match role.embed(&texts, TextRole::Query) {
        Ok(vectors) => {
            let telos: Vec<TelosProfile> = profiles
                .iter()
                .zip(vectors)
                .map(|((name, text, affinity), vector)| TelosProfile {
                    focus_name: name.clone(),
                    vector,
                    source: text.clone(),
                    category_affinity: affinity.clone(),
                    // Stamped by the caller, which knows which source asked.
                    opportunity_type: None,
                })
                .collect();
            if let Some(cache) = cache_path {
                write_cache(cache, &telos, &role.cache_key());
            }
            Some(telos)
        }
        Err(error) => {
            eprintln!(
                "  embed: backend '{}' unreachable ({error}) — falling back to hash embedding",
                role.backend_name
            );
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Cache I/O
// ---------------------------------------------------------------------------

/// Bumped when the on-disk shape changes. Format 1 was a bare map with no
/// producer, so it cannot prove which model wrote it and is never trusted.
pub const CACHE_FORMAT: u32 = 2;

#[derive(Serialize, Deserialize)]
pub struct CacheEntry {
    pub vector: Vec<f32>,
    pub text: String,
    #[serde(default)]
    pub category_affinity: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct VectorCache {
    pub format: u32,
    /// `backend:model` of whatever produced these vectors, from
    /// `ResolvedRole::cache_key`.
    pub producer: String,
    pub profiles: HashMap<String, CacheEntry>,
}

/// Reads a cache only when the same producer wrote it.
///
/// `None` on a missing file, an unreadable one, a format-1 file, or a producer
/// mismatch. Every rejection costs one recompute; accepting a mismatch costs
/// silently wrong scores until a human notices, which is the trade this
/// function exists to make in the safe direction.
pub fn read_cache(path: &Path, expected_producer: &str) -> Option<VectorCache> {
    let text = std::fs::read_to_string(path).ok()?;
    let cache: VectorCache = match serde_json::from_str(&text) {
        Ok(cache) => cache,
        Err(_) => {
            eprintln!(
                "  embed: {} predates the inference config and cannot name the model \
                 that wrote it — recomputing",
                path.display()
            );
            return None;
        }
    };
    if cache.format != CACHE_FORMAT || cache.producer != expected_producer {
        eprintln!(
            "  embed: {} was written by '{}', this run uses '{expected_producer}' — recomputing",
            path.display(),
            cache.producer
        );
        return None;
    }
    Some(cache)
}

fn write_cache(path: &Path, telos: &[TelosProfile], producer: &str) {
    let cache = VectorCache {
        format: CACHE_FORMAT,
        producer: producer.to_string(),
        profiles: telos
            .iter()
            .map(|profile| {
                (
                    profile.focus_name.clone(),
                    CacheEntry {
                        vector: profile.vector.clone(),
                        text: profile.source.clone(),
                        category_affinity: profile.category_affinity.clone(),
                    },
                )
            })
            .collect(),
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(&cache) {
        Ok(json) => {
            if let Err(error) = std::fs::write(path, &json) {
                eprintln!(
                    "  embed: could not write cache to {}: {error}",
                    path.display()
                );
            }
        }
        Err(error) => eprintln!("  embed: could not serialize cache: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("axon-embed-test-{name}.json"))
    }

    fn profile() -> TelosProfile {
        TelosProfile {
            focus_name: "Reisen".into(),
            vector: vec![0.1, 0.2, 0.3],
            source: "AI-Meetups und Kurztrips".into(),
            category_affinity: vec![],
            opportunity_type: None,
        }
    }

    #[test]
    fn a_cache_round_trips_under_its_own_producer() {
        let path = temp_file("roundtrip");
        write_cache(&path, &[profile()], "omlx:multilingual-e5-base-mlx");
        let cache = read_cache(&path, "omlx:multilingual-e5-base-mlx").expect("same producer");
        assert_eq!(cache.profiles["Reisen"].vector, vec![0.1, 0.2, 0.3]);
        let _ = std::fs::remove_file(path);
    }

    /// The silent-corruption case. Without this, an oMLX to Ollama switch
    /// re-uses e5 vectors under nomic-embed-text and every score is wrong with
    /// nothing in the logs.
    #[test]
    fn a_cache_from_another_model_is_refused() {
        let path = temp_file("mismatch");
        write_cache(&path, &[profile()], "omlx:multilingual-e5-base-mlx");
        assert!(
            read_cache(&path, "ollama:nomic-embed-text").is_none(),
            "vectors from one model must never be served to another"
        );
        let _ = std::fs::remove_file(path);
    }

    /// A cache written before the producer existed cannot prove anything.
    #[test]
    fn a_legacy_cache_without_a_producer_is_refused() {
        let path = temp_file("legacy");
        std::fs::write(
            &path,
            r#"{"Reisen":{"vector":[0.1],"text":"x","category_affinity":[]}}"#,
        )
        .unwrap();
        assert!(read_cache(&path, "omlx:multilingual-e5-base-mlx").is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn no_role_means_hash_fallback_not_a_panic() {
        let profiles = vec![("Reisen".to_string(), "text".to_string(), vec![])];
        assert!(try_embed_profiles(&profiles, None, None).is_none());
    }

    #[test]
    fn an_empty_profile_set_needs_no_backend_at_all() {
        assert!(try_embed_profiles(&[], None, None).unwrap().is_empty());
    }
}
