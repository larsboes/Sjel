//! Rung 3 of the ladder: the `ocr` inference role, and no engine.
//!
//! PRD Q63 -> B30 gives the ladder a third rung because rung 2 does not fail on
//! a page of notation — it succeeds and returns something wrong
//! (`upstreams.toml [auge]`, 2026-08-31). This module is the slot that failure
//! earns. It deliberately holds NO engine.
//!
//! ## The gate an engine has to clear to fill this slot
//!
//! The same one `multilingual-e5-base-mlx` and `bge-reranker-v2-m3-mlx` cleared,
//! and the reason both entered while `multilingual-e5-small-mlx` and both Apple
//! native variants did not: a frozen corpus with judgements written before any
//! score was seen, fixed acceptance thresholds that do not move between
//! candidates, and an append-only record — including the failures.
//!
//! For this rung that corpus is `libs/extraction/eval/`. An engine enters when,
//! and only when:
//!
//! 1. it has a passing record under `eval/results/`, on the unchanged corpus,
//!    reported on BOTH verdict lines (prose recall and notation fidelity) —
//!    only the second one earns this rung, because prose alone is what rung 2
//!    already does;
//! 2. its `upstreams.toml` entry quotes those numbers and points at the record;
//! 3. `ocr` is then declared in the deployment's `inference.json`.
//!
//! Two candidates are held as direction only and named here so the next reader
//! finds the gate before a README claim: `upstreams.toml [dolphin]` and
//! `[ocrs]`. Neither has a German measurement on this machine. No engine enters
//! this ladder on a project's own description of itself.
//!
//! ## Why this is a role and not a dependency
//!
//! Every candidate for this rung is a model, not a library: PyTorch weights
//! behind a server. `libs/inference` is where "which model answers this job on
//! this machine" already lives, and a second answer to that question is the
//! defect that library was created to end. So rung 3 takes a
//! [`ResolvedRole`] the caller looked up, and never reads configuration itself
//! — this crate owns no config, no store and no HTTP client, which is what
//! makes it a `libs/` member at all.

use sjel_inference::ResolvedRole;

use crate::{Document, Extraction, ExtractionError, Result};

/// What this rung calls itself in an error, and the role name it asks for.
pub const ENGINE: &str = "ocr-role";
/// The role id in `inference.json`. Undeclared on purpose; see this module's
/// doc comment for what declaring it requires first.
pub const ROLE: &str = "ocr";

/// Read a page with the `ocr` role.
///
/// Always [`ExtractionError::Unavailable`] today, with or without a resolved
/// role, and the two cases say different things on purpose:
///
/// * `None` — this deployment declares no `ocr` role. The ordinary state, and
///   the one `libs/inference` documents as a degrade rather than a failure.
/// * `Some(role)` — a role IS declared, and this rung still refuses, because no
///   engine has a passing record against the frozen corpus. That refusal is the
///   gate doing its job: a role pointed at an unmeasured model would put wrong
///   notation into the store under a `producer` claiming it was read.
///
/// The `doc` argument is taken now rather than added later so that the
/// signature a future engine implements is the one every caller already writes
/// against.
pub fn read(role: Option<&ResolvedRole>, doc: &Document<'_>) -> Result<Extraction> {
    let _ = doc;
    let why = match role {
        None => format!(
            "this deployment declares no {ROLE:?} inference role, so there is no rung above \
             Apple Vision here"
        ),
        Some(role) => format!(
            "the {ROLE:?} role resolves to {} on {}, and no engine has cleared the frozen \
             DE/EN corpus gate at libs/extraction/eval/. See that directory's README for \
             what a passing record requires.",
            role.model, role.backend_name
        ),
    };
    Err(ExtractionError::Unavailable {
        engine: ENGINE,
        why,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_undeclared_ocr_role_says_the_rung_is_absent_not_that_the_page_failed() {
        let error = read(None, &Document::image(b"pixels")).expect_err("no engine is adopted");
        assert!(
            matches!(error, ExtractionError::Unavailable { .. }),
            "a missing rung is not a document failure: {error:?}"
        );
        assert!(
            error.to_string().contains("no \"ocr\" inference role"),
            "{error}"
        );
    }

    #[test]
    fn a_declared_role_still_refuses_until_an_engine_clears_the_corpus_gate() {
        // The whole point of the slot. Declaring a role in inference.json must
        // not be enough to put an unmeasured model's output into a store, and
        // the error names where the measurement has to happen.
        let role = ResolvedRole {
            backend_name: "ollama".into(),
            backend: sjel_inference::Backend {
                api: sjel_inference::Api::Ollama,
                base_url: "http://127.0.0.1:11434".into(),
                api_key_file: None,
                provided_by: None,
            },
            model: "some-unmeasured-ocr-model".into(),
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
        };
        let error = read(Some(&role), &Document::image(b"pixels")).expect_err("gate not cleared");
        assert!(
            error.to_string().contains("libs/extraction/eval/"),
            "{error}"
        );
        assert!(
            error.to_string().contains("some-unmeasured-ocr-model"),
            "{error}"
        );
    }
}
