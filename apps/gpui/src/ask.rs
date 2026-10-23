//! Native multi-turn Ask session (`sjel://ask?q=…` and `Page::Ask`).
//!
//! Runs multi-turn capability queries in-session through `POST /assistant/api/ask` (which
//! drives `libs/agent` with read-only Sjel tools and no `bash`), keeping the conversation
//! inside the native GPUI window instead of handing off to a browser tab.

use futures::StreamExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, FontWeight, SharedString,
    Subscription, Window,
};
use zeroize::Zeroizing;

use crate::data::Repo;
use crate::shell::{self, AskProvider, AskTurn};
use crate::theme::Palette;
use crate::ui::{list, note, row, section};

const STARTERS: &[&str] = &[
    "What is on my calendar today and tomorrow?",
    "Which capabilities are up right now?",
    "List my active projects and open tasks",
];
const KEY_PROVIDERS: &[(&str, &str)] = &[
    ("openai", "OpenAI"),
    ("deepseek", "DeepSeek"),
    ("opencode-go", "OpenCode Go"),
    ("nvidia-nim", "NVIDIA NIM"),
    ("gemini", "Google Gemini · AI Studio"),
];

fn excerpt(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let shown: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{shown}…")
    } else {
        shown
    }
}

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
    activity: Vec<ToolActivity>,
}

#[derive(Clone, Debug)]
struct ToolActivity {
    name: String,
    arguments: String,
    result: String,
}

/// One question and its answer as it stands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exchange {
    pub prompt: String,
    pub answer: String,
    pub tools: Vec<String>,
    pub error: Option<String>,
    pub loading: bool,
}

pub struct Ask {
    repo: Repo,
    turns: Vec<Turn>,
    pub loading: bool,
    input: Option<(Entity<InputState>, Subscription)>,
    focus_next: bool,
    active_run: Option<u64>,
    queued_steering: Vec<String>,
    providers: Vec<AskProvider>,
    provider_discovery_started: bool,
    provider_discovery_finished: bool,
    provider_discovery_error: Option<String>,
    selected_provider: Option<String>,
    confirm_public_send: bool,
    active_cloud_run: bool,
    show_keys: bool,
    key_provider: String,
    key_input: Option<(Entity<InputState>, Subscription)>,
    key_input_provider: Option<String>,
    key_status: Option<String>,
    confirm_key_removal: bool,
}

impl Ask {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            turns: Vec::new(),
            loading: false,
            input: None,
            focus_next: true,
            active_run: None,
            queued_steering: Vec::new(),
            providers: Vec::new(),
            provider_discovery_started: false,
            provider_discovery_finished: false,
            provider_discovery_error: None,
            selected_provider: None,
            confirm_public_send: false,
            active_cloud_run: false,
            show_keys: false,
            key_provider: "openai".into(),
            key_input: None,
            key_input_provider: None,
            key_status: None,
            confirm_key_removal: false,
        }
    }

    /// The newest question and what has come back so far, for the finder's inline answer. The
    /// conversation stays here: the finder shows one exchange, and ⌘2 shows all of them.
    pub fn latest(&self) -> Option<Exchange> {
        let prompt = self
            .turns
            .iter()
            .rposition(|turn| turn.role == Role::User)?;
        let after = &self.turns[prompt + 1..];
        let answer = after.iter().find(|turn| turn.role == Role::Assistant);
        Some(Exchange {
            prompt: self.turns[prompt].content.clone(),
            answer: answer.map(|turn| turn.content.clone()).unwrap_or_default(),
            tools: answer.map(|turn| turn.tools.clone()).unwrap_or_default(),
            error: after
                .iter()
                .rev()
                .find(|turn| turn.role == Role::Error)
                .map(|turn| turn.content.clone()),
            loading: self.loading,
        })
    }

    pub fn focus_input(&mut self, cx: &mut Context<Self>) {
        self.focus_next = true;
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.turns.clear();
        self.loading = false;
        self.active_run = None;
        self.queued_steering.clear();
        self.confirm_public_send = false;
        self.active_cloud_run = false;
        self.focus_next = true;
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
                    activity: Vec::new(),
                });
                self.turns.push(Turn {
                    role: Role::Error,
                    content: error.to_string(),
                    meta: None,
                    tools: Vec::new(),
                    activity: Vec::new(),
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
                activity: Vec::new(),
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
                activity: Vec::new(),
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
            activity: Vec::new(),
        });
        self.turns.push(Turn {
            role: Role::Assistant,
            content: String::new(),
            meta: provider_meta.map(|meta| format!("Sending current prompt only · {meta}")),
            tools: Vec::new(),
            activity: Vec::new(),
        });
        self.loading = true;
        self.focus_next = true;
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
                            activity: Vec::new(),
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
                            activity: Vec::new(),
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
                            activity: Vec::new(),
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
                            activity: Vec::new(),
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
                                    turn.tools.push(event.name.clone());
                                }
                                turn.activity.push(ToolActivity {
                                    name: event.name,
                                    arguments: event.arguments,
                                    result: String::new(),
                                });
                            }
                        }
                        "tool_result" => {
                            if let Some(turn) = assistant {
                                if let Some(activity) =
                                    turn.activity.iter_mut().rev().find(|activity| {
                                        activity.name == event.name && activity.result.is_empty()
                                    })
                                {
                                    activity.result = event.content;
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
                                activity: Vec::new(),
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
                        activity: Vec::new(),
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
                activity: Vec::new(),
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
            activity: Vec::new(),
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
                            activity: Vec::new(),
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

    fn stop(&mut self, cx: &mut Context<Self>) {
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
                        activity: Vec::new(),
                    });
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn ensure_input(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        let input = if let Some((input, _)) = &self.input {
            input.clone()
        } else {
            let input = cx.new(|cx| {
                InputState::new(window, cx).placeholder(
                    "Ask Sjel anything about your calendar, projects, trips or capabilities…",
                )
            });
            let subscription = cx.subscribe_in(&input, window, |view, input, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    let text = input.read(cx).value().trim().to_owned();
                    if text.is_empty() {
                        return;
                    }
                    input.update(cx, |input, cx| input.set_value("", window, cx));
                    view.submit(text, cx);
                }
            });
            self.input = Some((input.clone(), subscription));
            input
        };
        if self.focus_next {
            self.focus_next = false;
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        input
    }

    fn ensure_key_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if self.key_input_provider.as_deref() != Some(&self.key_provider) {
            self.key_input = None;
            self.key_input_provider = Some(self.key_provider.clone());
        }
        if let Some((input, _)) = &self.key_input {
            return input.clone();
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("API key (hidden while typing)")
        });
        let subscription = cx.subscribe_in(&input, window, |view, input, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                let value = input.read(cx).value().to_string();
                input.update(cx, |input, cx| input.set_value("", window, cx));
                view.save_provider_key(value, cx);
            }
        });
        self.key_input = Some((input.clone(), subscription));
        input
    }

    fn save_provider_key(&mut self, value: String, cx: &mut Context<Self>) {
        if value.trim().is_empty() {
            self.key_status = Some("Enter an API key first.".into());
            cx.notify();
            return;
        }
        let key = Zeroizing::new(value);
        let provider = self.key_provider.clone();
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(_) => {
                self.key_status = Some("Sjel is not connected.".into());
                cx.notify();
                return;
            }
        };
        self.key_status = Some("Saving to the OS keychain…".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result =
                shell::run(async move { shell.store_provider_key(&provider, key).await }).await;
            let _ = this.update(cx, |view, cx| {
                view.key_status = Some(match result {
                    Some(Ok(())) => "API key saved in the OS keychain.".into(),
                    _ => {
                        "Could not save the API key. Check Keychain access and the private overlay."
                            .into()
                    }
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn remove_provider_key(&mut self, cx: &mut Context<Self>) {
        let provider = self.key_provider.clone();
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(_) => {
                self.key_status = Some("Sjel is not connected.".into());
                cx.notify();
                return;
            }
        };
        self.key_status = Some("Removing key from the OS keychain…".into());
        self.confirm_key_removal = false;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result =
                shell::run(async move { shell.remove_provider_key(&provider).await }).await;
            let _ = this.update(cx, |view, cx| {
                view.key_status = Some(match result {
                    Some(Ok(())) => "API key removed from the OS keychain.".into(),
                    _ => "Could not remove this provider key from the OS keychain.".into(),
                });
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for Ask {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.provider_discovery_started {
            self.provider_discovery_started = true;
            if let Ok(shell_client) = self.repo.shell() {
                cx.spawn(async move |this, cx| {
                    let result =
                        shell::run(async move { shell_client.ask_providers().await }).await;
                    let _ = this.update(cx, |view, cx| {
                        match result {
                            Some(Ok(providers)) => {
                                view.providers = providers;
                                view.provider_discovery_error = None;
                            }
                            Some(Err(error)) => {
                                view.provider_discovery_error = Some(error.to_string());
                            }
                            None => {
                                view.provider_discovery_error =
                                    Some("the provider request did not complete".into());
                            }
                        }
                        view.provider_discovery_finished = true;
                        cx.notify();
                    });
                })
                .detach();
            } else {
                self.provider_discovery_finished = true;
                self.provider_discovery_error = Some("Sjel is not connected".into());
            }
        }
        let palette = Palette::for_appearance(window.appearance());
        let mono = cx.theme().mono_font_family.clone();
        let input = self.ensure_input(window, cx);
        let input_for_send = input.clone();
        let key_input = self.show_keys.then(|| self.ensure_key_input(window, cx));

        let provider_bar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                Button::new("ask-model-local")
                    .small()
                    .disabled(self.loading)
                    .when(self.selected_provider.is_none(), |button| button.primary())
                    .when(self.selected_provider.is_some(), |button| button.ghost())
                    .label("Local")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.selected_provider = None;
                        view.confirm_public_send = false;
                        cx.notify();
                    })),
            )
            .children(self.providers.iter().map(|provider| {
                let role = provider.role.clone();
                let selected = self.selected_provider.as_deref() == Some(&provider.role);
                let label = if provider.available {
                    format!("{} · {}", provider.provider, provider.model)
                } else {
                    format!("{} · {} · unavailable", provider.provider, provider.model)
                };
                Button::new(format!("ask-model-{}", provider.role))
                    .small()
                    .disabled(self.loading || !provider.available)
                    .when(selected, |button| button.primary())
                    .when(!selected, |button| button.ghost())
                    .label(label)
                    .tooltip(provider.reason.clone().unwrap_or_else(|| {
                        "This provider receives only the current prompt after public-data confirmation. No local history or tools are sent.".into()
                    }))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.selected_provider = if view.selected_provider.as_deref() == Some(&role) {
                            None
                        } else {
                            Some(role.clone())
                        };
                        view.confirm_public_send = false;
                        cx.notify();
                    }))
            }))
            .child(
                Button::new("ask-provider-keys")
                    .small()
                    .ghost()
                    .label(if self.show_keys { "Close API keys" } else { "API keys" })
                    .on_click(cx.listener(|view, _, window, cx| {
                        if view.show_keys {
                            if let Some((input, _)) = &view.key_input {
                                input.update(cx, |input, cx| input.set_value("", window, cx));
                            }
                            view.key_input = None;
                            view.key_input_provider = None;
                        }
                        view.show_keys = !view.show_keys;
                        view.key_status = None;
                        view.confirm_key_removal = false;
                        cx.notify();
                    })),
            )
            .when(self.selected_provider.is_some(), |bar| {
                bar.child(
                    Button::new("ask-confirm-public")
                        .small()
                        .disabled(self.loading)
                        .when(self.confirm_public_send, |button| button.primary())
                        .when(!self.confirm_public_send, |button| button.ghost())
                        .label(if self.confirm_public_send {
                            "Public c0 confirmed"
                        } else {
                            "Confirm public c0"
                        })
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.confirm_public_send = !view.confirm_public_send;
                            cx.notify();
                        })),
                )
            })
            .when(self.provider_discovery_error.is_some(), |bar| {
                bar.child(
                    Button::new("ask-provider-retry")
                        .small()
                        .ghost()
                        .label("Retry providers")
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.provider_discovery_started = false;
                            view.provider_discovery_finished = false;
                            view.provider_discovery_error = None;
                            cx.notify();
                        })),
                )
            });

        let key_panel: Option<AnyElement> = key_input.as_ref().map(|key_input| {
            let input_for_save = key_input.clone();
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded(px(12.))
                .bg(rgba(palette.surface))
                .border_1()
                .border_color(rgba(palette.border))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.text))
                        .child("Manage provider API keys · stored in the OS keychain"),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .children(KEY_PROVIDERS.iter().map(|(slug, label)| {
                            let selected = self.key_provider == *slug;
                            let slug = (*slug).to_owned();
                            Button::new(format!("ask-key-{slug}"))
                                .small()
                                .when(selected, |button| button.primary())
                                .when(!selected, |button| button.ghost())
                                .label(*label)
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    if let Some((input, _)) = &view.key_input {
                                        input.update(cx, |input, cx| {
                                            input.set_value("", window, cx)
                                        });
                                    }
                                    view.key_provider = slug.clone();
                                    view.key_status = None;
                                    view.confirm_key_removal = false;
                                    view.key_input = None;
                                    view.key_input_provider = None;
                                    cx.notify();
                                }))
                        })),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().flex_1().child(Input::new(key_input)))
                        .child(
                            Button::new("ask-provider-key-save")
                                .small()
                                .primary()
                                .label("Save key")
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    let value = input_for_save.read(cx).value().to_string();
                                    input_for_save
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                    view.save_provider_key(value, cx);
                                })),
                        )
                        .child(if self.confirm_key_removal {
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child("Remove this saved key?"),
                                )
                                .child(
                                    Button::new("ask-provider-key-remove-confirm")
                                        .small()
                                        .label("Confirm remove")
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.remove_provider_key(cx);
                                        })),
                                )
                                .child(
                                    Button::new("ask-provider-key-remove-cancel")
                                        .small()
                                        .ghost()
                                        .label("Cancel")
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.confirm_key_removal = false;
                                            cx.notify();
                                        })),
                                )
                        } else {
                            div().child(
                                Button::new("ask-provider-key-remove")
                                    .small()
                                    .ghost()
                                    .label("Remove…")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.confirm_key_removal = true;
                                        cx.notify();
                                    })),
                            )
                        }),
                )
                .children(self.key_status.as_ref().map(|status| {
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(status.clone())
                }))
                .into_any_element()
        });

        let provider_notice = if self.selected_provider.is_some() {
            note(
                "Cloud sends the current prompt only. It excludes conversation history, Sjel tools, and extensions. Confirmation resets after each send.",
                palette.muted,
            )
        } else if !self.provider_discovery_finished {
            note("Checking configured cloud provider roles…", palette.muted)
        } else if let Some(error) = self.provider_discovery_error.as_ref() {
            note(
                format!("Could not load provider roles: {error}"),
                palette.muted,
            )
        } else if self.providers.is_empty() {
            note(
                "No cloud model roles are available. Ask stays on the configured local model.",
                palette.muted,
            )
        } else if !self.providers.iter().any(|provider| provider.available) {
            note(
                "No cloud role is ready. Unavailable provider buttons explain whether review, billing, or credentials need attention.",
                palette.muted,
            )
        } else {
            note(
                "Choose a configured provider to send a public c0 prompt. Local remains the default.",
                palette.muted,
            )
        };

        let composer = div()
            .flex()
            .items_center()
            .gap_2()
            .p_3()
            .rounded(px(12.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .child(div().flex_1().child(Input::new(&input)))
            .child(
                Button::new("ask-send")
                    .primary()
                    .small()
                    .label(if self.loading { "Steer" } else { "Send" })
                    .tooltip(if self.loading {
                        "Add guidance for the next model step (↩)"
                    } else {
                        "Send in session (↩)"
                    })
                    .on_click(cx.listener(move |view, _, window, cx| {
                        let text = input_for_send.read(cx).value().trim().to_owned();
                        if text.is_empty() {
                            return;
                        }
                        input_for_send.update(cx, |state, cx| state.set_value("", window, cx));
                        view.submit(text, cx);
                    })),
            )
            .when(self.loading && self.active_run.is_some(), |bar| {
                bar.child(
                    Button::new("ask-stop")
                        .ghost()
                        .small()
                        .label("Stop")
                        .tooltip("Stop after the current model request or tool batch")
                        .on_click(cx.listener(|view, _, _, cx| view.stop(cx))),
                )
            })
            .when(!self.turns.is_empty(), |bar| {
                bar.child(
                    Button::new("ask-clear")
                        .ghost()
                        .small()
                        .label("Clear")
                        .disabled(self.loading)
                        .on_click(cx.listener(|view, _, _, cx| view.clear(cx))),
                )
            });

        let starters = section(
            "Suggested prompts",
            list(
                STARTERS
                    .iter()
                    .enumerate()
                    .map(|(idx, &prompt)| {
                        row()
                            .id(("starter", idx))
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgba(palette.raised)))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.submit(prompt.to_owned(), cx);
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .text_sm()
                                    .text_color(rgb(palette.text))
                                    .child(prompt),
                            )
                            .child(
                                div()
                                    .font_family(mono.clone())
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child("↩"),
                            )
                            .into_any_element()
                    })
                    .collect(),
                palette,
            ),
            palette,
        );

        let transcript: AnyElement = if self.turns.is_empty() {
            note(
                "Ask a question above or press ⌥Space from any app. Follow-up questions stay in this native session.",
                palette.muted,
            )
        } else {
            let mut rows: Vec<AnyElement> = self
                .turns
                .iter()
                .map(|turn| {
                    let (label, label_color) = match turn.role {
                        Role::User => ("You", palette.accent),
                        Role::Assistant => ("Sjel", palette.green),
                        Role::Error => ("Error", palette.red),
                    };
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .px_4()
                        .py_3()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(label_color))
                                        .child(label),
                                )
                                .children(turn.meta.as_ref().map(|meta| {
                                    div()
                                        .font_family(mono.clone())
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(SharedString::from(meta.clone()))
                                })),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(if turn.role == Role::Error {
                                    palette.red
                                } else {
                                    palette.text
                                }))
                                .child(SharedString::from(turn.content.clone())),
                        )
                        .when(!turn.tools.is_empty(), |card| {
                            card.child(div().flex().flex_wrap().gap_1().children(
                                turn.tools.iter().enumerate().map(|(index, tool)| {
                                    div()
                                        .id(("ask-tool", index))
                                        .px_2()
                                        .py_1()
                                        .rounded(px(5.))
                                        .bg(rgba(palette.raised))
                                        .font_family(mono.clone())
                                        .text_xs()
                                        .text_color(rgb(palette.accent))
                                        .child(SharedString::from(tool.clone()))
                                }),
                            ))
                        })
                        .when(!turn.activity.is_empty(), |card| {
                            card.children(turn.activity.iter().enumerate().map(|(index, item)| {
                                let arguments = excerpt(&item.arguments, 500);
                                let result = excerpt(&item.result, 1200);
                                div()
                                    .id(("ask-activity", index))
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .mt_2()
                                    .px_3()
                                    .py_2()
                                    .rounded(px(7.))
                                    .bg(rgba(palette.raised))
                                    .child(
                                        div()
                                            .font_family(mono.clone())
                                            .text_xs()
                                            .text_color(rgb(palette.accent))
                                            .child(SharedString::from(item.name.clone())),
                                    )
                                    .when(!arguments.is_empty(), |row| {
                                        row.child(
                                            div()
                                                .font_family(mono.clone())
                                                .text_xs()
                                                .text_color(rgb(palette.muted))
                                                .child(SharedString::from(arguments)),
                                        )
                                    })
                                    .when(!result.is_empty(), |row| {
                                        row.child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(palette.text))
                                                .child(SharedString::from(result)),
                                        )
                                    })
                            }))
                        })
                        .into_any_element()
                })
                .collect();
            if self.loading {
                rows.push(
                    row()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("Reading capabilities and thinking…")
                        .into_any_element(),
                );
            }
            section("Session", list(rows, palette), palette).into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(provider_bar)
            .children(key_panel)
            .child(provider_notice)
            .child(composer)
            .child(transcript)
            .when(self.turns.is_empty(), |page| page.child(starters))
    }
}
