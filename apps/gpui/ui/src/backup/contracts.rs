//! Capability backup contracts cards and views.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, rgb, rgba, AnyElement, Context,
};
use sjel_gpui_core::backup::{format_age_seconds, format_bytes};
use sjel_gpui_core::shell::BackupStatus;

use crate::theme::{tint, Palette};

use super::BackupPage;

impl BackupPage {
    pub(super) fn render_contracts_view(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(backups) = &self.backups.value else {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Loading capability backup contracts…")
                .into_any_element();
        };

        if backups.backups.is_empty() {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("No capabilities declare a backup contract on this machine.")
                .into_any_element();
        }

        let mut cards = Vec::new();
        for b in &backups.backups {
            cards.push(self.render_contract_card(b, palette, cx));
        }

        div()
            .flex()
            .flex_col()
            .gap_3()
            .children(cards)
            .into_any_element()
    }

    pub(super) fn render_contract_card(
        &self,
        b: &BackupStatus,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_running = self
            .action_in_flight
            .get(&format!("backup:{}", b.capability))
            .copied()
            .unwrap_or(false)
            || b.run.as_ref().is_some_and(|r| r.state == "running");
        let is_confirming = self.confirming_cold_copy.as_deref() == Some(&b.capability);

        let attempt_failed = b
            .attempt
            .as_ref()
            .is_some_and(|att| att.exit_code.is_some_and(|code| code != 0));

        let (badge_bg, badge_fg, badge_text) = match b.state.as_str() {
            "ok" | "fresh" => (
                rgba(tint(palette.green, 0x20)),
                palette.green,
                "ok".to_string(),
            ),
            "due" => (
                rgba(tint(palette.amber, 0x20)),
                palette.amber,
                "due".to_string(),
            ),
            "overdue" => (
                rgba(tint(palette.red, 0x20)),
                palette.red,
                "overdue".to_string(),
            ),
            "never" => (
                rgba(tint(palette.red, 0x20)),
                palette.red,
                "never".to_string(),
            ),
            _ => (
                rgba(tint(palette.muted, 0x20)),
                palette.muted,
                b.state.clone(),
            ),
        };

        let cap_name = b.capability.clone();
        let holds_service = b.holds_service;

        div()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(rgba(tint(palette.border, 0x30)))
            .bg(rgba(tint(palette.raised, 0x40)))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_base()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(b.capability.clone()),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded_full()
                                            .text_xs()
                                            .bg(badge_bg)
                                            .text_color(rgb(badge_fg))
                                            .child(badge_text),
                                    )
                                    .children(if b.holds_service {
                                        Some(
                                            div()
                                                .px_2()
                                                .py_0p5()
                                                .rounded_full()
                                                .text_xs()
                                                .bg(rgba(tint(palette.amber, 0x20)))
                                                .text_color(rgb(palette.amber))
                                                .child("holds-service"),
                                        )
                                    } else {
                                        None
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(if b.last_success.is_some() {
                                        format!(
                                            "Backed up {} ({})",
                                            format_age_seconds(b.age_seconds),
                                            format_bytes(b.bytes)
                                        )
                                    } else {
                                        "Never backed up".to_string()
                                    })
                                    .child(format!(
                                        "[advise: {}d, stale: {}d]",
                                        b.advise_days.map(|d| d.to_string()).unwrap_or_else(|| "—".to_string()),
                                        b.stale_days.map(|d| d.to_string()).unwrap_or_else(|| "—".to_string())
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .children(if is_running {
                                Some(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.accent))
                                        .child("Backing up…")
                                        .into_any_element(),
                                )
                            } else if is_confirming {
                                Some(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .child(
                                            Button::new(format!("confirm-{}", cap_name))
                                                .small()
                                                .label("Stop & back up")
                                                .on_click(cx.listener(move |page, _, _, cx| {
                                                    page.trigger_backup(cap_name.clone(), None, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new(format!("cancel-{}", b.capability))
                                                .small()
                                                .ghost()
                                                .label("Cancel")
                                                .on_click(cx.listener(|page, _, _, cx| {
                                                    page.confirming_cold_copy = None;
                                                    cx.notify();
                                                })),
                                        )
                                        .into_any_element(),
                                )
                            } else {
                                let cap_to_backup = cap_name.clone();
                                Some(
                                    Button::new(format!("backup-btn-{}", cap_name))
                                        .small()
                                        .label("Back up now")
                                        .on_click(cx.listener(move |page, _, _, cx| {
                                            if holds_service {
                                                page.confirming_cold_copy = Some(cap_to_backup.clone());
                                                cx.notify();
                                            } else {
                                                page.trigger_backup(cap_to_backup.clone(), None, cx);
                                            }
                                        }))
                                        .into_any_element(),
                                )
                            }),
                    ),
            )
            .children(if let Some(att) = &b.attempt {
                if attempt_failed {
                    Some(
                        div()
                            .p_3()
                            .rounded_md()
                            .bg(rgba(tint(palette.red, 0x15)))
                            .border_1()
                            .border_color(rgba(tint(palette.red, 0x30)))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .text_xs()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.red))
                                    .child(format!(
                                        "Last attempt FAILED (exit {}) · target: {}",
                                        att.exit_code.unwrap_or(-1),
                                        att.target
                                    )),
                            )
                            .children(if !att.detail.is_empty() {
                                Some(
                                    div()
                                        .text_color(rgb(palette.text))
                                        .font_family(".AppleSystemUIFontMonospaced")
                                        .child(att.detail.clone()),
                                )
                            } else {
                                None
                            })
                            .children(if !att.log_path.is_empty() {
                                Some(
                                    div()
                                        .text_color(rgb(palette.muted))
                                        .child(format!("Log: {}", att.log_path)),
                                )
                            } else {
                                None
                            }),
                    )
                } else {
                    None
                }
            } else {
                None
            })
            .children(if is_confirming {
                Some(
                    div()
                        .p_3()
                        .rounded_md()
                        .bg(rgba(tint(palette.amber, 0x15)))
                        .border_1()
                        .border_color(rgba(tint(palette.amber, 0x30)))
                        .text_xs()
                        .text_color(rgb(palette.amber))
                        .child(format!(
                            "A cold copy backup requires temporarily stopping '{}'. It will automatically restart once the archive is written.",
                            b.capability
                        )),
                )
            } else {
                None
            })
            .into_any_element()
    }
}
