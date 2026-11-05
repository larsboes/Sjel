//! Provider API key entry, masked input state, and keychain operations.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, AnyElement, Context, Entity, Window};
use sjel_gpui_core::shell;
use zeroize::Zeroizing;

use super::constants::KEY_PROVIDERS;
use super::Accounts;
use crate::primitives::{note, section};
use crate::theme::Palette;

impl Accounts {
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

    pub(crate) fn key_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
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

    pub(crate) fn save_key(&mut self, value: String, cx: &mut Context<Self>) {
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

    pub(crate) fn remove_key(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn provider_keys(
        &self,
        palette: Palette,
        key_input: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let input_for_save = key_input.clone();
        let presence = if let Some(keys) = self.key_presence.value.as_ref() {
            if keys.get(&self.key_provider).copied().unwrap_or(false) {
                "Key saved in the OS keychain."
            } else {
                "No key saved for this provider."
            }
        } else if self.key_presence.error.is_some() {
            "Key status could not be read."
        } else {
            "Checking key status…"
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
}
