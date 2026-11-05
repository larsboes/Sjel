//! Backup targets, storage endpoints, cadence selectors, and rehearsal verification.
use gpui_kit::component::button::Button;
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, rgb, rgba, AnyElement, Context};
use sjel_gpui_core::backup::{BackupTargetRecord, VerifyTargetResult};

use crate::theme::{tint, Palette};

use super::BackupPage;

impl BackupPage {
    pub(super) fn render_targets_view(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(targets) = &self.targets.value else {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Loading declared backup targets…")
                .into_any_element();
        };

        if targets.is_empty() {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("No backup targets are declared on this machine.")
                .into_any_element();
        }

        let mut cards = Vec::new();
        for t in targets {
            cards.push(self.render_target_card(t, palette, cx));
        }

        div()
            .flex()
            .flex_col()
            .gap_3()
            .children(cards)
            .into_any_element()
    }

    pub(super) fn render_target_card(
        &self,
        t: &BackupTargetRecord,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_verifying = self.verifying.get(&t.id).copied().unwrap_or(false);
        let tid = t.id.clone();

        let present_color = match t.present.as_str() {
            "true" => palette.green,
            "false" => palette.red,
            _ => palette.muted,
        };

        let verdict = self.verify_verdicts.get(&t.id).cloned().or_else(|| {
            t.verified_verdict.as_ref().map(|v| VerifyTargetResult {
                target: t.id.clone(),
                verdict: v.clone(),
                detail: t.verified_detail.clone().unwrap_or_default(),
            })
        });

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
                                            .child(t.id.clone()),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded_full()
                                            .text_xs()
                                            .bg(rgba(tint(palette.border, 0x30)))
                                            .text_color(rgb(palette.muted))
                                            .child(t.kind.clone()),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded_full()
                                            .text_xs()
                                            .bg(rgba(tint(present_color, 0x20)))
                                            .text_color(rgb(present_color))
                                            .child(match t.present.as_str() {
                                                "true" => "present".to_string(),
                                                "false" => "missing".to_string(),
                                                other => other.to_string(),
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .font_family(".AppleSystemUIFontMonospaced")
                                    .child(if !t.host.is_empty() {
                                        format!("{}:{}", t.host, t.path)
                                    } else {
                                        t.path.clone()
                                    }),
                            ),
                    )
                    .child(
                        Button::new(format!("verify-{}", t.id))
                            .small()
                            .label(if is_verifying {
                                "Verifying…"
                            } else {
                                "Verify target"
                            })
                            .loading(is_verifying)
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.verify_target(tid.clone(), cx);
                            })),
                    ),
            )
            .children(verdict.map(|v| {
                let is_ok = v.verdict == "verified";
                let tone = if is_ok { palette.green } else { palette.red };
                div()
                    .p_2p5()
                    .rounded_md()
                    .bg(rgba(tint(tone, 0x15)))
                    .border_1()
                    .border_color(rgba(tint(tone, 0x30)))
                    .text_xs()
                    .text_color(rgb(tone))
                    .child(format!(
                        "Verification {}: {}",
                        v.verdict,
                        if v.detail.is_empty() {
                            "rehearsal pass"
                        } else {
                            &v.detail
                        }
                    ))
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .pt_2()
                    .border_t_1()
                    .border_color(rgba(tint(palette.border, 0x20)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Scheduled Cadence:"),
                    )
                    .child(self.render_cadence_selector(t, palette, cx)),
            )
            .into_any_element()
    }

    pub(super) fn render_cadence_selector(
        &self,
        t: &BackupTargetRecord,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let presets = [
            ("Off", None),
            ("12h", Some(12)),
            ("Daily (24h)", Some(24)),
            ("48h", Some(48)),
            ("Weekly (168h)", Some(168)),
        ];

        let target_id = t.id.clone();
        let current_interval = t.interval_hours;

        div()
            .flex()
            .items_center()
            .gap_1()
            .children(presets.into_iter().map(|(label, hours)| {
                let tid = target_id.clone();
                let is_selected = current_interval == hours;
                div()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_xs()
                    .cursor_pointer()
                    .bg(if is_selected {
                        rgba(tint(palette.accent, 0x25))
                    } else {
                        rgba(tint(palette.raised, 0x20))
                    })
                    .text_color(if is_selected {
                        rgb(palette.accent)
                    } else {
                        rgb(palette.muted)
                    })
                    .border_1()
                    .border_color(if is_selected {
                        rgb(palette.accent)
                    } else {
                        rgba(tint(palette.border, 0x30))
                    })
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |page, _, _, cx| {
                            page.set_target_policy(tid.clone(), hours, cx);
                        }),
                    )
                    .child(label)
            }))
            .into_any_element()
    }
}
