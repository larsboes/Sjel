//! Which rung an unattended pass is allowed to wake up.
//!
//! On 2026-08-13 the enrichment drain picked up a backlog of 182 unsummarized
//! feed items and began feeding them, one after another, through a 9B model on
//! this laptop's GPU. The machine went loud and hot and the operator stopped the
//! model server by hand. Nothing was broken: every part did what it was built to
//! do, and the sum of them was a bulk local inference job nobody had asked for.
//!
//! So the rung is no longer a property of the request alone. It is a property of
//! **who is asking**:
//!
//! - An unattended pass — either drain, the prefill behind `POST /ingest`, the
//!   bounded refresh endpoint — may use the light local role and nothing else.
//!   That is Apple's on-device model here: about two seconds an item, at zero
//!   Metal cost, sharing no memory pool with anything.
//! - An explicit press keeps the whole ladder, strong local rung included. The
//!   operator pressing Regenerate is looking at the item and has decided the wait
//!   and the fan are worth it. That path does not come through this module.
//!
//! ## Over the window is a verdict, not a failure
//!
//! A source the light role cannot hold is [`Rung::OverWindow`]. No request is
//! made — not a truncated one, not a hopeful one. `afm-server` refuses an
//! over-window prompt outright rather than truncating it, so attempting anyway
//! would buy a guaranteed error, and then an error the caller has to remember
//! not to count. Deciding it here, before the call, is what keeps the
//! capacity-alert streak meaning what it says: the local server is failing
//! requests it accepted.
//!
//! ## One home for the arithmetic, one home for the role
//!
//! The window question belongs to `libs/summarize` — [`summarize::fits_window`] —
//! because it is the same input-plus-reply arithmetic the digest ladder already
//! uses. The *role* question lives here, and `Config::light_summarization_role`
//! reads it from here, so there is exactly one definition of what "the light
//! rung" resolves to on a machine.

use sjel_inference::{InferenceConfig, ResolvedRole};

use crate::summarize;

/// The role name an unattended pass runs on.
pub const LIGHT_ROLE: &str = "summarization_light";

/// A smaller, faster local model for the cheap rungs, if this machine has one.
///
/// Optional by design: a machine with only `summarization` keeps working, its
/// unattended passes simply have nothing to run on and say so.
///
/// The role must declare `max_input_tokens`. Without it there is no way to tell
/// whether a source fits, and guessing is how you get a context error instead of
/// a digest, so an undeclared window means the role is skipped.
pub fn light_role(inference: &InferenceConfig) -> Option<ResolvedRole> {
    inference
        .role(LIGHT_ROLE)
        .filter(|role| role.max_input_tokens.is_some())
}

/// What an unattended pass may do with one source of a given size.
#[derive(Debug, Clone)]
pub enum Rung {
    /// The light local role can hold this source, prompt and reply together.
    Light(Box<ResolvedRole>),
    /// It cannot, and no unattended pass may reach past it. Left for a press —
    /// or, for a `public` feed item, for the cloud door (`crate::cloud_run`).
    OverWindow,
    /// This machine has no light role, so unattended passes have nothing to run
    /// on at all. Distinct from [`Rung::OverWindow`]: one is a fact about the
    /// item, the other about the machine.
    Unconfigured,
}

/// Compared by which rung it is and, for [`Rung::Light`], which model — the two
/// things a caller can act on. `ResolvedRole` carries a resolved backend and is
/// not `PartialEq` in its own crate; making it so would pin fields this
/// comparison has no opinion about.
impl PartialEq for Rung {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Light(left), Self::Light(right)) => left.cache_key() == right.cache_key(),
            (Self::OverWindow, Self::OverWindow) | (Self::Unconfigured, Self::Unconfigured) => true,
            _ => false,
        }
    }
}

/// The rung an unattended pass gets for a source of `source_chars`, answering in
/// at most `reply_tokens`.
///
/// Deliberately does not fall through to the `summarization` role. That
/// fallthrough is what `digest::role_for` does for a press, and reproducing it
/// here would put the strong local model back on the drain's path by the exact
/// route that made the machine hot — a source too big for the small model,
/// silently handed to the big one.
pub fn rung(inference: &InferenceConfig, source_chars: usize, reply_tokens: u32) -> Rung {
    let Some(light) = light_role(inference) else {
        return Rung::Unconfigured;
    };
    // `light_role` already refuses a role with no declared window, so this
    // `unwrap_or` is unreachable rather than lenient.
    let window = light.max_input_tokens.unwrap_or_default();
    if summarize::fits_window(source_chars, reply_tokens, window) {
        Rung::Light(Box::new(light))
    } else {
        Rung::OverWindow
    }
}

/// Runtime admission plus the full-source window check for On-the-go AFM work.
/// No truncation is allowed to turn an oversized source into an admitted request.
pub fn runtime_admission(
    role: &ResolvedRole,
    source_chars: usize,
    reply_tokens: u32,
) -> Result<(), sjel_runtime::Deferred> {
    let status = sjel_runtime::current()?;
    status.admit_model(&role.backend_name, &role.model, role.is_loopback())?;
    if status.effective == sjel_runtime::Selection::OnTheGo
        && role.backend_name == "foundation-models"
        && role.model == "apple-foundationmodel"
        && role.max_input_tokens.is_none_or(|window| {
            source_chars > summarize::window_chars(reply_tokens, window)
                || !summarize::fits_window(source_chars, reply_tokens, window)
        })
    {
        return Err(sjel_runtime::Deferred(
            "deferred by runtime profile: AFM needs a known context window that fits the full source".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apple's on-device model: 4,096 tokens shared between prompt and reply.
    const APPLE_WINDOW: u32 = 4_096;

    fn inference(light: Option<u32>, strong: bool) -> InferenceConfig {
        let mut roles = serde_json::Map::new();
        if strong {
            roles.insert(
                "summarization".into(),
                serde_json::json!({ "backend": "omlx", "model": "big-local-model" }),
            );
        }
        if let Some(window) = light {
            roles.insert(
                LIGHT_ROLE.into(),
                serde_json::json!({
                    "backend": "foundation-models",
                    "model": "apple-on-device",
                    "max_input_tokens": window,
                }),
            );
        }
        serde_json::from_value(serde_json::json!({
            "backends": {
                "omlx": { "api": "openai", "base_url": "http://127.0.0.1:8000/v1" },
                "foundation-models": { "api": "openai", "base_url": "http://127.0.0.1:8091/v1" },
            },
            "roles": roles,
        }))
        .expect("the probe config is well formed")
    }

    /// C20's seam. A source the light role can hold is served by the light role
    /// — named, not merely "some local model" — and one it cannot is a verdict
    /// rather than a handoff to the strong rung.
    #[test]
    fn an_unattended_pass_gets_the_light_role_or_nothing() {
        let inference = inference(Some(APPLE_WINDOW), true);

        match rung(&inference, 2_000, 500) {
            Rung::Light(role) => {
                assert_eq!(role.backend_name, "foundation-models");
                assert_eq!(role.model, "apple-on-device");
            }
            other => panic!("a 2,000-character source must fit the light role, got {other:?}"),
        }

        assert_eq!(
            rung(&inference, 30_000, 1_000),
            Rung::OverWindow,
            "a source over the light window must never reach the strong local role"
        );
    }

    /// The regression this module exists to prevent, stated as the property
    /// rather than as one example: whatever the size, an unattended pass never
    /// resolves the strong local role, even with one configured and healthy.
    #[test]
    fn no_source_length_reaches_the_strong_local_role() {
        let inference = inference(Some(APPLE_WINDOW), true);
        for chars in [0, 599, 600, 2_499, 2_500, 8_999, 9_000, 15_000, 200_000] {
            for reply in [0, 200, 500, 800, 1_000] {
                if let Rung::Light(role) = rung(&inference, chars, reply) {
                    assert_eq!(
                        role.backend_name, "foundation-models",
                        "{chars} chars / {reply} reply tokens resolved a non-light backend"
                    );
                }
            }
        }
    }

    /// A machine with no light role has nothing to run unattended work on. It
    /// must say so rather than quietly meaning "use the big one".
    #[test]
    fn a_machine_without_a_light_role_is_unconfigured_not_escalated() {
        assert_eq!(rung(&inference(None, true), 100, 200), Rung::Unconfigured);
        assert!(light_role(&inference(None, true)).is_none());
    }

    /// A light role with no declared window is skipped rather than guessed at:
    /// offering a model a request it cannot hold produces a context error, not
    /// a digest.
    #[test]
    fn a_light_role_without_a_declared_window_does_not_count() {
        let no_window: InferenceConfig = serde_json::from_value(serde_json::json!({
            "backends": {
                "foundation-models": { "api": "openai", "base_url": "http://127.0.0.1:8091/v1" },
            },
            "roles": {
                LIGHT_ROLE: { "backend": "foundation-models", "model": "apple-on-device" },
            },
        }))
        .expect("the probe config is well formed");
        assert_eq!(rung(&no_window, 10, 10), Rung::Unconfigured);
    }

    /// The boundary is input *and* reply against the shared window, so the same
    /// source flips verdict when more room is asked for on the way out. Pinned
    /// because a check that sized only the prompt would pass both of these.
    #[test]
    fn the_reply_ceiling_counts_against_the_same_window() {
        let inference = inference(Some(APPLE_WINDOW), false);
        // 9,000 chars is 3,000 tokens of input plus 400 of overhead: 3,400.
        assert!(matches!(rung(&inference, 9_000, 500), Rung::Light(_)));
        assert_eq!(rung(&inference, 9_000, 800), Rung::OverWindow);
    }
}
