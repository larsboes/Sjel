//! CV Master header, profile builder controls, output card, and built PDF list.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, SharedString};
use sjel_gpui_core::cv::{count_stale_pdfs, pdf_for_profile, CvView, CvWriteRequest};

use crate::primitives::{card, dot};
use crate::theme::Palette;

use super::CvPage;

impl CvPage {
    pub(super) fn render_master_header(&self, view: &CvView, palette: Palette) -> AnyElement {
        let mtime_str = if view.master_mtime > 0 {
            chrono::DateTime::from_timestamp(view.master_mtime as i64, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| format!("epoch {}", view.master_mtime))
        } else {
            "unknown".to_string()
        };

        let stale_count = count_stale_pdfs(view);
        let linkedin_date = view.linkedin_pulled.as_deref().unwrap_or("Never");

        let header_content = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(dot(palette.accent))
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.text))
                                    .child("Master Curriculum Vitae"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(palette.muted))
                                    .child(format!("({})", view.master)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(palette.muted))
                                    .child(format!("Last edited: {}", mtime_str)),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(if stale_count > 0 {
                                        rgb(palette.amber)
                                    } else {
                                        rgb(palette.green)
                                    })
                                    .child(format!(
                                        "{} PDF(s) built ({} stale)",
                                        view.pdfs.len(),
                                        stale_count
                                    )),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .text_size(px(11.0))
                    .text_color(rgb(palette.muted))
                    .child(format!("LinkedIn pulled: {}", linkedin_date))
                    .child("•")
                    .child("Single source of truth in master_cv.yaml"),
            );

        card(header_content.into_any_element(), palette)
    }

    pub(super) fn render_builder_panel(
        &self,
        view: &CvView,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_busy = !self.busy.is_empty();
        let selected_profile = self.selected_profile.clone();
        let selected_lang = self.selected_lang.clone();
        let pdf_match = pdf_for_profile(view, &selected_profile, &selected_lang);

        let pdf_status_element = match pdf_match {
            Some(pdf) if pdf.stale => div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(dot(palette.amber))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(palette.amber))
                        .child("Stale (older than master)"),
                ),
            Some(_) => div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(dot(palette.green))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(palette.green))
                        .child("Built and up to date"),
                ),
            None => div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(dot(palette.muted))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(palette.muted))
                        .child("Not built yet"),
                ),
        };

        let profile_buttons = view.profiles.iter().cloned().map(|prof| {
            let is_selected = prof == selected_profile;
            let prof_clone = prof.clone();
            Button::new(SharedString::from(format!("prof-{}", prof)))
                .small()
                .when(!is_selected, |b| b.ghost())
                .label(prof)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected_profile = prof_clone.clone();
                    cx.notify();
                }))
        });

        let lang_en = Button::new("lang-en")
            .small()
            .when(selected_lang != "en", |b| b.ghost())
            .label("English (en)")
            .on_click(cx.listener(|this, _, _, cx| {
                this.selected_lang = "en".to_string();
                cx.notify();
            }));

        let lang_de = Button::new("lang-de")
            .small()
            .when(selected_lang != "de", |b| b.ghost())
            .label("German (de)")
            .on_click(cx.listener(|this, _, _, cx| {
                this.selected_lang = "de".to_string();
                cx.notify();
            }));

        let btn_build = Button::new("btn-build")
            .small()
            .label("Build PDF")
            .disabled(is_busy)
            .on_click(cx.listener({
                let profile = selected_profile.clone();
                let lang = selected_lang.clone();
                move |this, _, window, cx| {
                    this.run_action(
                        CvWriteRequest {
                            action: "build".to_string(),
                            profile: profile.clone(),
                            lang: lang.clone(),
                            ..Default::default()
                        },
                        window,
                        cx,
                    );
                }
            }));

        let btn_build_all = Button::new("btn-build-all")
            .small()
            .ghost()
            .label("Build All")
            .disabled(is_busy)
            .on_click(cx.listener(|this, _, window, cx| {
                this.run_action(
                    CvWriteRequest {
                        action: "build-all".to_string(),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
            }));

        let btn_li_pull = Button::new("btn-li-pull")
            .small()
            .ghost()
            .label("Pull LinkedIn")
            .disabled(is_busy)
            .on_click(cx.listener(|this, _, window, cx| {
                this.run_action(
                    CvWriteRequest {
                        action: "linkedin-pull".to_string(),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
            }));

        let btn_li_diff = Button::new("btn-li-diff")
            .small()
            .ghost()
            .label("LinkedIn Diff")
            .disabled(is_busy || view.linkedin_pulled.is_none())
            .on_click(cx.listener(|this, _, window, cx| {
                this.run_action(
                    CvWriteRequest {
                        action: "linkedin-diff".to_string(),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
            }));

        let btn_li_sheet = Button::new("btn-li-sheet")
            .small()
            .ghost()
            .label(format!("Paste Sheet ({})", selected_lang))
            .disabled(is_busy)
            .on_click(cx.listener({
                let lang = selected_lang.clone();
                move |this, _, window, cx| {
                    this.run_action(
                        CvWriteRequest {
                            action: "linkedin-sheet".to_string(),
                            lang: lang.clone(),
                            ..Default::default()
                        },
                        window,
                        cx,
                    );
                }
            }));

        let content = div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child("Profile & PDF Builder"),
                    )
                    .child(pdf_status_element),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .w(px(70.0))
                                    .text_size(px(12.0))
                                    .text_color(rgb(palette.muted))
                                    .child("Profile:"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap(px(6.0))
                                    .children(profile_buttons),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .w(px(70.0))
                                    .text_size(px(12.0))
                                    .text_color(rgb(palette.muted))
                                    .child("Language:"),
                            )
                            .child(div().flex().gap(px(6.0)).child(lang_en).child(lang_de)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt(px(6.0))
                    .border_t_1()
                    .border_color(rgba(palette.border))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(btn_build)
                            .child(btn_build_all),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(btn_li_pull)
                            .child(btn_li_diff)
                            .child(btn_li_sheet),
                    ),
            );

        card(content.into_any_element(), palette)
    }

    pub(super) fn render_output_card(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let output = self.output.as_ref()?;
        let dismiss_btn = Button::new("btn-dismiss-output")
            .small()
            .ghost()
            .label("Dismiss")
            .on_click(cx.listener(|this, _, _, cx| {
                this.output = None;
                cx.notify();
            }));

        let content = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child("Output / Diff"),
                    )
                    .child(dismiss_btn),
            )
            .child(
                div()
                    .p(px(8.0))
                    .rounded(px(4.0))
                    .bg(rgba(palette.surface))
                    .text_size(px(11.0))
                    .text_color(rgb(palette.text))
                    .font_family(".AppleSystemUIFontMonospaced")
                    .child(output.clone()),
            );

        Some(card(content.into_any_element(), palette))
    }

    pub(super) fn render_pdfs_list(&self, view: &CvView, palette: Palette) -> AnyElement {
        let pdf_rows = view.pdfs.iter().map(|pdf| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .p(px(6.0))
                .rounded(px(4.0))
                .bg(rgba(palette.surface))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(rgb(palette.text))
                                .child(pdf.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child(format!("({})", pdf.path)),
                        ),
                )
                .child(if pdf.stale {
                    div()
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded(px(3.0))
                        .bg(rgb(palette.amber))
                        .text_size(px(10.0))
                        .text_color(rgb(0xffffff))
                        .child("stale: older than master")
                } else {
                    div()
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded(px(3.0))
                        .bg(rgb(palette.green))
                        .text_size(px(10.0))
                        .text_color(rgb(0xffffff))
                        .child("current")
                })
        });

        let content = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child("Built PDF Files"),
            )
            .child(div().flex().flex_col().gap(px(4.0)).children(pdf_rows));

        card(content.into_any_element(), palette)
    }
}
