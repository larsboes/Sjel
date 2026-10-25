//! The one Ask session: the conversation the finder (⌘K, `sjel://ask?q=…`) draws.
//!
//! Runs multi-turn capability queries through `POST /assistant/api/ask` (which drives
//! `libs/agent` with read-only Sjel tools and no `bash`). This holds the turns and the model
//! choice; it draws nothing. Provider keys live in Settings, under *Accounts & keys*.

use futures::StreamExt;
use gpui_kit::Context;

use crate::data::Repo;
use crate::shell::{self, AskProvider, AskTurn};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    User,
    Assistant,
    Error,
}

#[derive(Clone, Debug)]
struct Turn {
    role: Role,
    content: String,
    meta: Option<String>,
    tools: Vec<String>,
}

/// One question and its answer as it stands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exchange {
    pub prompt: String,
    pub answer: String,
    pub tools: Vec<String>,
    /// Which model answered, and from where.
    pub meta: Option<String>,
    pub error: Option<String>,
    pub loading: bool,
}

pub struct Ask {
    repo: Repo,
    turns: Vec<Turn>,
    pub loading: bool,
    active_run: Option<u64>,
    queued_steering: Vec<String>,
    providers: Vec<AskProvider>,
    provider_discovery_started: bool,
    provider_discovery_finished: bool,
    provider_discovery_error: Option<String>,
    selected_provider: Option<String>,
    confirm_public_send: bool,
    active_cloud_run: bool,
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

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.turns.clear();
        self.loading = false;
        self.active_run = None;
        self.queued_steering.clear();
        self.confirm_public_send = false;
        self.active_cloud_run = false;
        cx.notify();
    }

    pub fn submit(&mut self, raw: String, cx: &mut Context<Self>) {
        let prompt = raw.trim().to_owned();
        if prompt.is_empty() {
            return;
        }
        if self.loading {
            self.steer(prompt, cx);
            return;
        }
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                // The question stays on screen above the reason it was not sent, so it can be
                // read back and sent again.
                self.turns.push(Turn {
                    role: Role::User,
                    content: prompt,
                    meta: None,
                    tools: Vec::new(),
                });
                self.turns.push(Turn {
                    role: Role::Error,
                    content: error.to_string(),
                    meta: None,
                    tools: Vec::new(),
                });
                cx.notify();
                return;
            }
        };
        let history: Vec<AskTurn> = self
            .turns
            .iter()
            .filter_map(|t| match t.role {
                Role::User => Some(AskTurn {
                    role: "user".into(),
                    content: t.content.clone(),
                }),
                Role::Assistant => Some(AskTurn {
                    role: "assistant".into(),
                    content: t.content.clone(),
                }),
                Role::Error => None,
            })
            .collect();
        let provider_role = self.selected_provider.clone();
        let confirm_public_send = self.confirm_public_send;
        if provider_role.is_some() && !confirm_public_send {
            self.turns.push(Turn {
                role: Role::Error,
                content:
                    "Confirm that this prompt is public (c0) before sending it to a cloud provider."
                        .into(),
                meta: None,
                tools: Vec::new(),
            });
            cx.notify();
            return;
        }
        if provider_role.is_some() && !history.is_empty() {
            self.turns.push(Turn {
                role: Role::Error,
                content: "Cloud Ask sends one prompt only. Clear the local conversation before using a cloud model.".into(),
                meta: None,
                tools: Vec::new(),
            });
            cx.notify();
            return;
        }
        self.confirm_public_send = false;
        let provider_meta = provider_role.as_ref().and_then(|role| {
            self.providers
                .iter()
                .find(|provider| &provider.role == role)
                .map(|provider| format!("{} · {} · public c0", provider.provider, provider.model))
        });
        self.active_cloud_run = provider_role.is_some();
        self.turns.push(Turn {
            role: Role::User,
            content: prompt.clone(),
            meta: provider_meta.clone(),
            tools: Vec::new(),
        });
        self.turns.push(Turn {
            role: Role::Assistant,
            content: String::new(),
            meta: provider_meta.map(|meta| format!("Sending current prompt only · {meta}")),
            tools: Vec::new(),
        });
        self.loading = true;
        cx.notify();

        let shell_client = shell.clone();
        let send_history = if provider_role.is_some() {
            Vec::new()
        } else {
            history
        };
        let work = shell::run(async move {
            shell
                .ask_stream_with_provider(
                    &prompt,
                    &send_history,
                    provider_role.as_deref(),
                    confirm_public_send,
                )
                .await
        });
        cx.spawn(async move |this, cx| {
            let (run_id, mut events) = match work.await {
                Some(Ok(stream)) => stream,
                Some(Err(error)) => {
                    let _ = this.update(cx, |view, cx| {
                        view.loading = false;
                        view.active_cloud_run = false;
                        view.turns.push(Turn {
                            role: Role::Error,
                            content: error.to_string(),
                            meta: None,
                            tools: Vec::new(),
                        });
                        cx.notify();
                    });
                    return;
                }
                None => {
                    let _ = this.update(cx, |view, cx| {
                        view.loading = false;
                        view.active_cloud_run = false;
                        view.turns.push(Turn {
                            role: Role::Error,
                            content: "Could not start the assistant stream.".into(),
                            meta: None,
                            tools: Vec::new(),
                        });
                        cx.notify();
                    });
                    return;
                }
            };
            let queued = this
                .update(cx, |view, cx| {
                    view.active_run = (run_id != 0).then_some(run_id);
                    let queued = std::mem::take(&mut view.queued_steering);
                    cx.notify();
                    queued
                })
                .unwrap_or_default();
            let mut received_terminal_event = false;
            for message in queued {
                let client = shell_client.clone();
                if run_id == 0 {
                    let _ = this.update(cx, |view, cx| {
                        view.turns.push(Turn {
                            role: Role::Error,
                            content: "Steering is unavailable for the offline summary.".into(),
                            meta: None,
                            tools: Vec::new(),
                        });
                        cx.notify();
                    });
                    continue;
                }
                if let Some(Err(error)) =
                    shell::run(async move { client.steer_ask(run_id, &message).await }).await
                {
                    let _ = this.update(cx, |view, cx| {
                        view.turns.push(Turn {
                            role: Role::Error,
                            content: format!("Steering could not be queued: {error}"),
                            meta: None,
                            tools: Vec::new(),
                        });
                        cx.notify();
                    });
                }
            }

            while let Some(event) = events.next().await {
                received_terminal_event =
                    matches!(event.kind.as_str(), "done" | "error" | "stopped");
                let _ = this.update(cx, |view, cx| {
                    let assistant = view
                        .turns
                        .iter_mut()
                        .rev()
                        .find(|turn| turn.role == Role::Assistant);
                    match event.kind.as_str() {
                        "text" => {
                            if let Some(turn) = assistant {
                                turn.content.push_str(&event.content);
                            }
                        }
                        "tool_call" => {
                            if let Some(turn) = assistant {
                                if !turn.tools.contains(&event.name) {
                                    turn.tools.push(event.name);
                                }
                            }
                        }
                        "done" => {
                            if let Some(turn) = assistant {
                                if turn.content.is_empty() {
                                    turn.content = event.text;
                                }
                                turn.meta = Some(format!("{} · {}", event.model, event.source));
                                turn.tools = event.tools;
                            }
                            view.loading = false;
                            view.active_cloud_run = false;
                            view.active_run = None;
                        }
                        "error" => {
                            view.turns.push(Turn {
                                role: Role::Error,
                                content: event.message,
                                meta: None,
                                tools: Vec::new(),
                            });
                            view.loading = false;
                            view.active_cloud_run = false;
                            view.active_run = None;
                        }
                        "stopped" => {
                            if let Some(turn) = assistant {
                                turn.meta = Some("Stopped".into());
                            }
                            view.loading = false;
                            view.active_cloud_run = false;
                            view.active_run = None;
                        }
                        _ => {}
                    }
                    cx.notify();
                });
            }
            let _ = this.update(cx, |view, cx| {
                if !received_terminal_event {
                    view.turns.push(Turn {
                        role: Role::Error,
                        content: "The assistant stream ended before the run finished.".into(),
                        meta: None,
                        tools: Vec::new(),
                    });
                }
                view.loading = false;
                view.active_cloud_run = false;
                view.active_run = None;
                cx.notify();
            });
        })
        .detach();
    }

    fn steer(&mut self, message: String, cx: &mut Context<Self>) {
        if self.active_cloud_run {
            self.turns.push(Turn {
                role: Role::Error,
                content: "Steering is disabled for cloud Ask so new text cannot leave without its own public-data confirmation.".into(),
                meta: None,
                tools: Vec::new(),
            });
            cx.notify();
            return;
        }
        let Some(shell) = self.repo.shell().ok() else {
            return;
        };
        if self.queued_steering.len() >= 32 {
            return;
        }
        self.turns.push(Turn {
            role: Role::User,
            content: message.clone(),
            meta: Some("steering".into()),
            tools: Vec::new(),
        });
        if let Some(run_id) = self.active_run {
            let shell_client = shell.clone();
            cx.spawn(async move |this, cx| {
                if let Some(Err(error)) =
                    shell::run(async move { shell_client.steer_ask(run_id, &message).await }).await
                {
                    let _ = this.update(cx, |view, cx| {
                        view.turns.push(Turn {
                            role: Role::Error,
                            content: format!("Steering could not be queued: {error}"),
                            meta: None,
                            tools: Vec::new(),
                        });
                        cx.notify();
                    });
                }
            })
            .detach();
        } else {
            self.queued_steering.push(message);
        }
        cx.notify();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        let (Some(run_id), Ok(shell)) = (self.active_run, self.repo.shell()) else {
            return;
        };
        let shell_client = shell.clone();
        cx.spawn(async move |this, cx| {
            if let Some(Err(error)) =
                shell::run(async move { shell_client.stop_ask(run_id).await }).await
            {
                let _ = this.update(cx, |view, cx| {
                    view.turns.push(Turn {
                        role: Role::Error,
                        content: format!("Stop could not be requested: {error}"),
                        meta: None,
                        tools: Vec::new(),
                    });
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Read the configured cloud roles once. Later calls do nothing until [`Self::retry_providers`].
    pub fn discover_providers(&mut self, cx: &mut Context<Self>) {
        if self.provider_discovery_started {
            return;
        }
        self.provider_discovery_started = true;
        let Ok(shell_client) = self.repo.shell() else {
            self.provider_discovery_finished = true;
            self.provider_discovery_error = Some("Sjel is not connected".into());
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = shell::run(async move { shell_client.ask_providers().await }).await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Some(Ok(providers)) => {
                        view.providers = providers;
                        view.provider_discovery_error = None;
                    }
                    Some(Err(error)) => view.provider_discovery_error = Some(error.to_string()),
                    None => {
                        view.provider_discovery_error =
                            Some("the provider request did not complete".into())
                    }
                }
                view.provider_discovery_finished = true;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn retry_providers(&mut self, cx: &mut Context<Self>) {
        self.provider_discovery_started = false;
        self.provider_discovery_finished = false;
        self.provider_discovery_error = None;
        self.discover_providers(cx);
    }

    /// The model the next question goes to: `local`, or a cloud role as `provider · model`.
    pub fn model(&self) -> Model {
        let Some(role) = self.selected_provider.as_deref() else {
            return Model::Local {
                error: self.provider_discovery_error.clone(),
            };
        };
        let label = self
            .providers
            .iter()
            .find(|provider| provider.role == role)
            .map_or_else(
                || role.to_owned(),
                |provider| format!("{} · {}", provider.provider, provider.model),
            );
        Model::Cloud {
            label,
            confirmed: self.confirm_public_send,
        }
    }

    /// Step local → each available cloud role → local. A new choice is never confirmed.
    pub fn next_model(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let available: Vec<&str> = self
            .providers
            .iter()
            .filter(|provider| provider.available)
            .map(|provider| provider.role.as_str())
            .collect();
        let next = match self.selected_provider.as_deref() {
            None => available.first(),
            Some(role) => available
                .iter()
                .position(|candidate| *candidate == role)
                .and_then(|index| available.get(index + 1)),
        };
        self.selected_provider = next.map(|role| (*role).to_owned());
        self.confirm_public_send = false;
        cx.notify();
    }

    pub fn toggle_public(&mut self, cx: &mut Context<Self>) {
        if self.selected_provider.is_some() && !self.loading {
            self.confirm_public_send = !self.confirm_public_send;
            cx.notify();
        }
    }

    /// True while a run is going that [`Self::stop`] can end.
    pub fn can_stop(&self) -> bool {
        self.loading && self.active_run.is_some()
    }
}

fn pairs(turns: &[Turn], loading: bool) -> Vec<Exchange> {
    let mut pairs: Vec<Exchange> = Vec::new();
    for turn in turns {
        if turn.role == Role::User {
            pairs.push(Exchange {
                prompt: turn.content.clone(),
                ..Exchange::default()
            });
            continue;
        }
        if pairs.is_empty() {
            pairs.push(Exchange::default());
        }
        let Some(pair) = pairs.last_mut() else {
            continue;
        };
        match turn.role {
            Role::Assistant => {
                pair.answer = turn.content.clone();
                pair.tools = turn.tools.clone();
                pair.meta = turn.meta.clone();
            }
            _ => pair.error = Some(turn.content.clone()),
        }
    }
    if let Some(last) = pairs.last_mut() {
        last.loading = loading;
    }
    pairs
}

/// Where the next question goes, for the finder's footer.
#[derive(Clone, Debug, PartialEq)]
pub enum Model {
    /// The configured local model, with the reason the cloud roles could not be read, if any.
    Local { error: Option<String> },
    /// A cloud role. It receives the current prompt only, and only once `confirmed` (public c0).
    Cloud { label: String, confirmed: bool },
}

#[cfg(test)]
mod tests {
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
