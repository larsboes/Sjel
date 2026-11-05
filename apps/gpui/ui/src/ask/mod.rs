//! The one Ask session: the conversation the finder (⌘K, `sjel://ask?q=…`) draws.
//!
//! Runs multi-turn capability queries through `POST /assistant/api/ask` (which drives
//! `libs/agent` with read-only Sjel tools and no `bash`). This holds the turns and the model
//! choice; it draws nothing. Provider keys live in Settings, under *Accounts & keys*.

pub mod providers;
pub mod stream;
pub mod types;

pub use types::{Exchange, Model};

use crate::data::Repo;
use sjel_gpui_core::shell::AskProvider;

use self::types::{pairs, Role, Turn};

pub struct Ask {
    pub(crate) repo: Repo,
    pub(crate) turns: Vec<Turn>,
    pub loading: bool,
    pub(crate) active_run: Option<u64>,
    pub(crate) queued_steering: Vec<String>,
    pub(crate) providers: Vec<AskProvider>,
    pub(crate) provider_discovery_started: bool,
    pub(crate) provider_discovery_finished: bool,
    pub(crate) provider_discovery_error: Option<String>,
    pub(crate) selected_provider: Option<String>,
    pub(crate) confirm_public_send: bool,
    pub(crate) active_cloud_run: bool,
}

impl Ask {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            turns: Vec::new(),
            loading: false,
            active_run: None,
            queued_steering: Vec::new(),
            providers: Vec::new(),
            provider_discovery_started: false,
            provider_discovery_finished: false,
            provider_discovery_error: None,
            selected_provider: None,
            confirm_public_send: false,
            active_cloud_run: false,
        }
    }

    /// The latest in-memory prompt that was not sent to a cloud provider.
    ///
    /// Settings uses this only to match catalog recommendation topics locally; the prompt itself
    /// is never persisted or sent as recommendation telemetry.
    pub fn latest_local_recommendation_prompt(&self) -> Option<String> {
        self.turns
            .iter()
            .enumerate()
            .rev()
            .find(|(index, turn)| {
                turn.role == Role::User
                    && !self.turns[index + 1..]
                        .iter()
                        .find(|later| later.role == Role::Assistant)
                        .and_then(|assistant| assistant.meta.as_deref())
                        .is_some_and(|meta| meta.starts_with("Sending current prompt only ·"))
            })
            .map(|(_, turn)| turn.content.clone())
    }

    /// The conversation as question-and-answer pairs, oldest first. A refusal that came before
    /// any question (a cloud send not confirmed yet) is a pair with an empty prompt.
    pub fn exchanges(&self) -> Vec<Exchange> {
        pairs(&self.turns, self.loading)
    }

    /// The newest pair, as it stands.
    pub fn latest(&self) -> Option<Exchange> {
        self.exchanges().pop()
    }
}

#[cfg(test)]
mod tests {
    use super::types::{Role, Turn};
    use super::*;

    fn turn(role: Role, content: &str) -> Turn {
        Turn {
            role,
            content: content.into(),
            meta: None,
            tools: Vec::new(),
        }
    }

    #[test]
    fn turns_pair_up_and_only_the_last_pair_is_loading() {
        let turns = [
            turn(Role::Error, "Confirm that this prompt is public (c0)"),
            turn(Role::User, "who is Anna?"),
            turn(Role::Assistant, "A friend in Berlin."),
            turn(Role::User, "and her birthday?"),
            turn(Role::Assistant, ""),
        ];
        let pairs = pairs(&turns, true);
        let prompts: Vec<&str> = pairs.iter().map(|pair| pair.prompt.as_str()).collect();
        assert_eq!(prompts, ["", "who is Anna?", "and her birthday?"]);
        assert!(
            pairs[0].error.is_some(),
            "a refusal before any question still shows"
        );
        assert_eq!(pairs[1].answer, "A friend in Berlin.");
        assert!(!pairs[1].loading && pairs[2].loading);
    }
}
