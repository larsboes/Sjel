//! Settings › Accounts: each credential a capability declares, where this Mac holds it, the
//! steps to obtain it, and a drop field for a Google client JSON.
//!
//! The rows come from sjel-status (`crate::accounts::CredentialRow`, presence only). Connect
//! runs the consent flow in this process (`sjel_gpui_core::accounts::connect_google`) and
//! writes the grant to the OS keychain; nothing secret is sent to the shell.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui::ExternalPaths;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, SharedString, Subscription, Window,
};
use zeroize::Zeroizing;

use crate::data::Repo;
use crate::shell;
use crate::theme::Palette;
use crate::ui::{list, note, section};
use sjel_gpui_core::accounts::CredentialRow;

const KEY_PROVIDERS: &[(&str, &str)] = &[
    ("openai", "OpenAI"),
    ("deepseek", "DeepSeek"),
    ("opencode-go", "OpenCode Go"),
    ("nvidia-nim", "NVIDIA NIM"),
    ("gemini", "Google Gemini · AI Studio"),
];

pub struct Accounts {
    repo: Repo,
    rows: Option<Result<Vec<CredentialRow>, String>>,
    loading: bool,
    /// The client JSON dropped for each credential ID, until Connect reads it.
    dropped: HashMap<String, PathBuf>,
    /// The credential ID whose consent flow is running.
    busy: Option<String>,
    /// The last outcome per credential ID: success, and the line shown.
    outcome: HashMap<String, (bool, String)>,
    key_provider: String,
    key_input: Option<(Entity<InputState>, Subscription)>,
    key_status: Option<String>,
    confirm_key_removal: bool,
    key_presence: Option<Result<HashMap<String, bool>, String>>,
}

impl Accounts {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            rows: None,
            loading: false,
            dropped: HashMap::new(),
            busy: None,
            outcome: HashMap::new(),
            key_provider: "openai".into(),
            key_input: None,
            key_status: None,
            confirm_key_removal: false,
            key_presence: None,
        }
    }

    pub fn select_key_provider(
        &mut self,
        provider: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !KEY_PROVIDERS.iter().any(|(slug, _)| *slug == provider) {
            return;
        }
        if self.key_provider == provider {
            return;
        }
        if let Some((input, _)) = &self.key_input {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.key_provider = provider.to_owned();
        self.key_status = None;
        self.confirm_key_removal = false;
        cx.notify();
    }

    pub fn refresh_if_missing(&mut self, cx: &mut Context<Self>) {
        if self.rows.is_none() || self.key_presence.is_none() {
            self.refresh(cx);
        }
    }

    fn key_input(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((input, _)) = &self.key_input {
            return input.clone();
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("API key (hidden while typing)")
        });
        let subscription = cx.subscribe_in(&input, window, |view, input, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let value = input.read(cx).value().to_string();
                input.update(cx, |input, cx| input.set_value("", window, cx));
                view.save_key(value, cx);
            }
        });
        self.key_input = Some((input.clone(), subscription));
        input
    }

    fn save_key(&mut self, value: String, cx: &mut Context<Self>) {
        if value.trim().is_empty() {
            self.key_status = Some("Enter an API key first.".into());
            cx.notify();
            return;
        }
        let Ok(client) = self.repo.shell() else {
            self.key_status = Some("Sjel is not connected.".into());
            cx.notify();
            return;
        };
        let provider = self.key_provider.clone();
        let key = Zeroizing::new(value);
        self.key_status = Some("Saving to the OS keychain…".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result =
                shell::run(async move { client.store_provider_key(&provider, key).await }).await;
            let _ = this.update(cx, |view, cx| {
                view.key_status = Some(
                    if matches!(result, Some(Ok(()))) {
                        "API key saved in the OS keychain."
                    } else {
                        "Could not save the API key."
                    }
                    .into(),
                );
                view.refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn remove_key(&mut self, cx: &mut Context<Self>) {
        let Ok(client) = self.repo.shell() else {
            self.key_status = Some("Sjel is not connected.".into());
            cx.notify();
            return;
        };
        let provider = self.key_provider.clone();
        self.confirm_key_removal = false;
        self.key_status = Some("Removing key from the OS keychain…".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result =
                shell::run(async move { client.remove_provider_key(&provider).await }).await;
            let _ = this.update(cx, |view, cx| {
                view.key_status = Some(
                    if matches!(result, Some(Ok(()))) {
                        "API key removed from the OS keychain."
                    } else {
                        "Could not remove this provider key."
                    }
                    .into(),
                );
                view.refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn provider_keys(
        &self,
        palette: Palette,
        key_input: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let input_for_save = key_input.clone();
        let presence = match &self.key_presence {
            Some(Ok(keys)) if keys.get(&self.key_provider).copied().unwrap_or(false) => {
                "Key saved in the OS keychain."
            }
            Some(Ok(_)) => "No key saved for this provider.",
            Some(Err(_)) => "Key status could not be read.",
            None => "Checking key status…",
        };
        section(
            "PROVIDER API KEYS",
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(note(
                    "Keys are stored in the OS keychain. Sjel never shows a saved value.",
                    palette.muted,
                ))
                .child(note(presence, palette.muted))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .children(KEY_PROVIDERS.iter().map(|(slug, label)| {
                            let selected = self.key_provider == *slug;
                            let slug = (*slug).to_owned();
                            Button::new(format!("settings-key-{slug}"))
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
                            Button::new("settings-key-save")
                                .primary()
                                .small()
                                .label("Save key")
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    let value = input_for_save.read(cx).value().to_string();
                                    input_for_save
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                    view.save_key(value, cx);
                                })),
                        )
                        .child(if self.confirm_key_removal {
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    Button::new("settings-key-remove-confirm")
                                        .small()
                                        .label("Confirm remove")
                                        .on_click(
                                            cx.listener(|view, _, _, cx| view.remove_key(cx)),
                                        ),
                                )
                                .child(
                                    Button::new("settings-key-remove-cancel")
                                        .small()
                                        .ghost()
                                        .label("Cancel")
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.confirm_key_removal = false;
                                            cx.notify();
                                        })),
                                )
                                .into_any_element()
                        } else {
                            Button::new("settings-key-remove")
                                .small()
                                .ghost()
                                .label("Remove saved key…")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.confirm_key_removal = true;
                                    cx.notify();
                                }))
                                .into_any_element()
                        }),
                )
                .children(
                    self.key_status
                        .as_ref()
                        .map(|status| note(status.clone(), palette.muted)),
                )
                .into_any_element(),
            palette,
        )
        .into_any_element()
    }

    /// Reads the declarations again. Called when the section opens and after a connect.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Ok(client) = self.repo.shell() else {
            self.rows = Some(Err("Sjel is not connected.".into()));
            self.key_presence = Some(Err("Sjel is not connected.".into()));
            cx.notify();
            return;
        };
        if self.loading {
            return;
        }
        self.loading = true;
        cx.spawn(async move |this, cx| {
            let result = shell::run(async move {
                let rows = client.credentials().await;
                let keys = client.provider_key_status().await;
                (rows, keys)
            })
            .await;
            let _ = this.update(cx, |view, cx| {
                view.loading = false;
                match result {
                    Some((rows, keys)) => {
                        view.rows =
                            Some(rows.map_err(|error| {
                                format!("Could not read the credentials: {error}")
                            }));
                        view.key_presence = Some(
                            keys.map(|keys| {
                                keys.into_iter()
                                    .map(|key| (key.provider, key.present))
                                    .collect()
                            })
                            .map_err(|error| error.to_string()),
                        );
                    }
                    None => {
                        view.rows = Some(Err("Could not read the credentials.".into()));
                        view.key_presence = Some(Err("Could not read key status.".into()));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn drop_file(&mut self, id: &str, paths: &ExternalPaths, cx: &mut Context<Self>) {
        let Some(path) = paths.paths().iter().find(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        }) else {
            self.outcome.insert(
                id.to_owned(),
                (false, "Drop the client JSON file (.json).".into()),
            );
            cx.notify();
            return;
        };
        self.dropped.insert(id.to_owned(), path.clone());
        self.outcome.remove(id);
        cx.notify();
    }

    fn connect(&mut self, row: CredentialRow, cx: &mut Context<Self>) {
        let Some(file) = self.dropped.get(&row.id).cloned() else {
            return;
        };
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(row.id.clone());
        self.outcome.insert(
            row.id.clone(),
            (true, "Waiting for Google in your browser…".into()),
        );
        cx.notify();
        let id = row.id.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { sjel_gpui_core::accounts::connect_google(&row, &file) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.busy = None;
                let line = match &result {
                    Ok(()) => (
                        true,
                        "Connected. The grant is in the OS keychain.".to_owned(),
                    ),
                    Err(error) => (false, error.clone()),
                };
                if result.is_ok() {
                    view.dropped.remove(&id);
                }
                view.outcome.insert(id, line);
                view.refresh(cx);
            });
        })
        .detach();
    }

    fn move_to_keychain(&mut self, row: CredentialRow, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(row.id.clone());
        self.outcome.insert(
            row.id.clone(),
            (true, "Moving the grant to the keychain…".into()),
        );
        cx.notify();
        let id = row.id.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { sjel_gpui_core::accounts::move_to_keychain(&row) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.busy = None;
                let line = match result {
                    Ok(()) => (
                        true,
                        "Moved. The grant is in the keychain and the env file no longer holds it."
                            .to_owned(),
                    ),
                    Err(error) => (false, error),
                };
                view.outcome.insert(id, line);
                view.refresh(cx);
            });
        })
        .detach();
    }

    fn card(&self, row: &CredentialRow, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        if let Some(error) = &row.error {
            return div()
                .px_4()
                .py_3()
                .child(note(format!("{}: {error}", row.capability), palette.red))
                .into_any_element();
        }
        let id = row.id.clone();
        let busy = self.busy.as_deref() == Some(row.id.as_str());
        let dropped = self.dropped.get(&row.id);
        let state_color = if row.in_keychain {
            palette.green
        } else {
            palette.amber
        };
        let drop_label: SharedString = match dropped {
            Some(path) => path
                .file_name()
                .map(|n| format!("Ready: {}", n.to_string_lossy()))
                .unwrap_or_else(|| "Ready".into())
                .into(),
            None => "Drop the client JSON file here".into(),
        };
        let drop_zone = div()
            .id(SharedString::from(format!("drop-{}", row.id)))
            .flex_1()
            .min_w(px(220.))
            .px_3()
            .py_3()
            .rounded(px(8.))
            .border_1()
            .border_dashed()
            .border_color(rgba(palette.border))
            .text_sm()
            .text_color(rgb(if dropped.is_some() {
                palette.text
            } else {
                palette.muted
            }))
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(rgba(palette.raised)))
            .on_drop(cx.listener({
                let id = id.clone();
                move |view, paths: &ExternalPaths, _, cx| view.drop_file(&id, paths, cx)
            }))
            .child(drop_label);
        let connect = Button::new(SharedString::from(format!("connect-{}", row.id)))
            .primary()
            .small()
            .label(if busy { "Connecting…" } else { "Connect" })
            .disabled(dropped.is_none() || self.busy.is_some())
            .on_click(cx.listener({
                let row = row.clone();
                move |view, _, _, cx| view.connect(row.clone(), cx)
            }));
        // Offered only when the old env file holds the grant and the keychain does not.
        let move_button = (row.env_file && !row.in_keychain).then(|| {
            Button::new(SharedString::from(format!("move-{}", row.id)))
                .outline()
                .small()
                .label("Move to keychain")
                .disabled(self.busy.is_some())
                .on_click(cx.listener({
                    let row = row.clone();
                    move |view, _, _, cx| view.move_to_keychain(row.clone(), cx)
                }))
        });
        let steps = row.steps.iter().enumerate().map(|(i, step)| {
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(SharedString::from(format!("{}. {step}", i + 1)))
        });
        div()
            .px_4()
            .py_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().text_sm().child(SharedString::from(row.label.clone())))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(state_color))
                            .child(row.state()),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(SharedString::from(format!(
                        "{} · {} · {}",
                        row.capability,
                        row.id,
                        row.scopes.join(", ")
                    ))),
            )
            .children(steps)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(drop_zone)
                    .child(connect)
                    .children(move_button),
            )
            .children(self.outcome.get(&row.id).map(|(ok, line)| {
                note(line.clone(), if *ok { palette.muted } else { palette.red })
            }))
            .into_any_element()
    }
}

impl Render for Accounts {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());
        let key_input = self.key_input(window, cx);
        if self.rows.is_none() && !self.loading {
            self.refresh(cx);
        }
        let body: AnyElement = match &self.rows {
            None => list(
                vec![note("Reading the declared credentials…", palette.muted)],
                palette,
            ),
            Some(Err(error)) => list(vec![note(error.clone(), palette.red)], palette),
            Some(Ok(rows)) if rows.is_empty() => list(
                vec![note("No capability declares a credential.", palette.muted)],
                palette,
            ),
            Some(Ok(rows)) => {
                let rows = rows.clone();
                list(
                    rows.iter().map(|row| self.card(row, palette, cx)).collect(),
                    palette,
                )
            }
        };
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(self.provider_keys(palette, &key_input, cx))
            .child(section("GOOGLE", body, palette))
            .child(note(
                "The client JSON is read once from the dropped file and never sent to the \
                 shell. Delete the download afterwards; Sjel keeps the grant in the keychain.",
                palette.muted,
            ))
    }
}
