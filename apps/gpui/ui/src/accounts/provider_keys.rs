//! Provider API key entry, masked input state, and keychain operations.

use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::forms::{Input, InputEvent, TextInput};
use crate::ely::theme::ControlSize;
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
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !KEY_PROVIDERS.iter().any(|(slug, _)| *slug == provider) {
            return;
        }
        if self.key_provider == provider {
            return;
        }
        if let Some((input, _)) = &self.key_input {
            input.update(cx, |input, cx| input.set_text("", cx));
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
    ) -> Entity<TextInput> {
        if let Some((input, _)) = &self.key_input {
            return input.clone();
        }
        let input = cx.new(|cx| {
            TextInput::new(window, cx)
                .masked()
                .placeholder("API key (hidden while typing)")
        });
        let subscription = cx.subscribe_in(&input, window, |view, input, event, _window, cx| {
            if matches!(event, InputEvent::Submit) {
                let value = input.read(cx).text().to_owned();
                input.update(cx, |input, cx| input.set_text("", cx));
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
        key_input: &Entity<TextInput>,
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
                            Button::new(format!("settings-key-{slug}"), *label)
                                .size(ControlSize::Sm)
                                .when(selected, |button| button.variant(ButtonVariant::Primary))
                                .when(!selected, |button| button.variant(ButtonVariant::Ghost))
                                .on_click(cx.listener(move |view, _, _window, cx| {
                                    if let Some((input, _)) = &view.key_input {
                                        input.update(cx, |input, cx| input.set_text("", cx));
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
                            Button::new("settings-key-save", "Save key")
                                .variant(ButtonVariant::Primary)
                                .size(ControlSize::Sm)
                                .on_click(cx.listener(move |view, _, _window, cx| {
                                    let value = input_for_save.read(cx).text().to_owned();
                                    input_for_save.update(cx, |input, cx| input.set_text("", cx));
                                    view.save_key(value, cx);
                                })),
                        )
                        .child(if self.confirm_key_removal {
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    Button::new("settings-key-remove-confirm", "Confirm remove")
                                        .size(ControlSize::Sm)
                                        .on_click(
                                            cx.listener(|view, _, _, cx| view.remove_key(cx)),
                                        ),
                                )
                                .child(
                                    Button::new("settings-key-remove-cancel", "Cancel")
                                        .size(ControlSize::Sm)
                                        .variant(ButtonVariant::Ghost)
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.confirm_key_removal = false;
                                            cx.notify();
                                        })),
                                )
                                .into_any_element()
                        } else {
                            Button::new("settings-key-remove", "Remove saved key…")
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
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
