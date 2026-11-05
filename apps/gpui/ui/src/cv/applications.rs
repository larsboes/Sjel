//! CV Application ledger, status filtering, and career pipeline table.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, SharedString};
use sjel_gpui_core::cv::{
    filter_applications, last_status_date, CvApplication, CvView, CvWriteRequest,
};

use crate::primitives::card;
use crate::theme::Palette;

use super::CvPage;

impl CvPage {
    pub(super) fn render_applications_table(
        &self,
        view: &CvView,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_busy = !self.busy.is_empty();
        let filter = &self.filter_status;
        let apps: Vec<&CvApplication> = if filter == "all" {
            view.applications.iter().collect()
        } else {
            filter_applications(view, filter)
        };

        let mut status_counts: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for app in &view.applications {
            *status_counts.entry(&app.status).or_insert(0) += 1;
        }

        let tab_all = Button::new("tab-all")
            .small()
            .when(filter != "all", |b| b.ghost())
            .label(format!("All ({})", view.applications.len()))
            .on_click(cx.listener(|this, _, _, cx| {
                this.filter_status = "all".to_string();
                cx.notify();
            }));

        let status_tabs = view.statuses.iter().map(|st| {
            let count = status_counts.get(st.as_str()).copied().unwrap_or(0);
            let is_active = st == filter;
            let st_clone = st.clone();
            Button::new(SharedString::from(format!("tab-{}", st)))
                .small()
                .when(!is_active, |b| b.ghost())
                .label(format!("{} ({})", st, count))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.filter_status = st_clone.clone();
                    cx.notify();
                }))
        });

        let app_rows = if apps.is_empty() {
            div()
                .p(px(16.0))
                .text_size(px(12.0))
                .text_color(rgb(palette.muted))
                .child("No applications found matching this status.")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .children(apps.into_iter().enumerate().map(|(idx, app)| {
                    let company = app.company.clone();
                    let role = app.role.clone();
                    let slug = app.slug.clone();
                    let status = app.status.clone();
                    let profile_lang = format!("{} · {}", app.profile, app.lang);
                    let last_date = last_status_date(app).to_string();
                    let url_opt = app.url.clone();

                    let next_statuses: Vec<String> = view
                        .statuses
                        .iter()
                        .filter(|s| *s != &status)
                        .cloned()
                        .collect();

                    let status_badge_color = match status.as_str() {
                        "offer" => rgb(palette.green),
                        "interviewing" | "interview" => rgb(palette.accent),
                        "rejected" => rgb(palette.red),
                        _ => rgb(palette.muted),
                    };

                    let next_btns = next_statuses.into_iter().take(2).map(|target_st| {
                        let slug_c = slug.clone();
                        let target_c = target_st.clone();
                        Button::new(SharedString::from(format!("st-{}-{}", slug, target_st)))
                            .small()
                            .ghost()
                            .disabled(is_busy)
                            .label(format!("→ {}", target_st))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.run_action(
                                    CvWriteRequest {
                                        action: "app-status".to_string(),
                                        slug: slug_c.clone(),
                                        status: target_c.clone(),
                                        ..Default::default()
                                    },
                                    window,
                                    cx,
                                );
                            }))
                    });

                    let build_app_btn =
                        Button::new(SharedString::from(format!("build-app-{}", slug)))
                            .small()
                            .ghost()
                            .disabled(is_busy)
                            .label("Build CV")
                            .on_click(cx.listener({
                                let slug_clone = slug.clone();
                                move |this, _, window, cx| {
                                    this.run_action(
                                        CvWriteRequest {
                                            action: "app-build".to_string(),
                                            slug: slug_clone.clone(),
                                            ..Default::default()
                                        },
                                        window,
                                        cx,
                                    );
                                }
                            }));

                    div()
                        .id(SharedString::from(format!("app-row-{}", idx)))
                        .flex()
                        .items_center()
                        .justify_between()
                        .p(px(8.0))
                        .rounded(px(6.0))
                        .bg(if idx % 2 == 0 {
                            rgba(palette.surface)
                        } else {
                            rgba(palette.window_tint)
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .w(px(220.0))
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(palette.text))
                                        .child(company),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(rgb(palette.muted))
                                        .child(role),
                                ),
                        )
                        .child(
                            div()
                                .w(px(110.0))
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child(profile_lang),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .px(px(6.0))
                                        .py(px(2.0))
                                        .rounded(px(4.0))
                                        .bg(status_badge_color)
                                        .text_size(px(11.0))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(0xffffff))
                                        .child(status.clone()),
                                )
                                .children(next_btns),
                        )
                        .child(
                            div()
                                .w(px(90.0))
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child(if last_date.is_empty() {
                                    "-".to_string()
                                } else {
                                    last_date
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(build_app_btn)
                                .children(url_opt.map(|u| {
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(rgb(palette.accent))
                                        .child(format!(
                                            "↗ {}",
                                            u.chars().take(24).collect::<String>()
                                        ))
                                })),
                        )
                }))
                .into_any_element()
        };

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
                            .child("Applications Ledger"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(6.0))
                            .child(tab_all)
                            .children(status_tabs),
                    ),
            )
            .child(app_rows);

        card(content.into_any_element(), palette)
    }
}
