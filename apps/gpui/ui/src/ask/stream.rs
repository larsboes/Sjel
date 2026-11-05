//! Streaming interaction and steering management for Ask sessions.

use futures::StreamExt;
use gpui_kit::Context;
use sjel_gpui_core::shell::{self, AskTurn};

use super::types::{Role, Turn};
use super::Ask;

impl Ask {
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

    pub(crate) fn steer(&mut self, message: String, cx: &mut Context<Self>) {
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
}
