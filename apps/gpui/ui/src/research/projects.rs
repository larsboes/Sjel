//! Active research projects, upstreams and external dependency watches.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, Window,
};
use sjel_gpui_core::research::{group_upstreams, SystemRow, UpstreamRow};

use crate::data;
use crate::primitives::card;
use crate::theme::Palette;

use super::types::SUMMARY_MAX;
use super::ResearchPage;

impl ResearchPage {
    pub(super) fn ensure_filter_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.filter_input {
            return input.clone();
        }
        let input = cx
            .new(|cx| InputState::new(window, cx).placeholder("Filter by name, summary, license…"));
        let subscription =
            cx.subscribe_in(&input, window, |this, input, _event: &InputEvent, _, cx| {
                this.filter_query = input.read(cx).value().to_string();
                cx.notify();
            });
        self.filter_input = Some((input.clone(), subscription));
        input
    }

    pub(super) fn ensure_add_url_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.add_url_input {
            return input.clone();
        }
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://github.com/owner/repo"));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |_view, _input, _event: &InputEvent, _, cx| {
                cx.notify();
            },
        );
        self.add_url_input = Some((input.clone(), subscription));
        input
    }

    pub(super) fn ensure_add_summary_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.add_summary_input {
            return input.clone();
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("What it is, and why it is worth a look")
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |_view, _input, _event: &InputEvent, _, cx| {
                cx.notify();
            },
        );
        self.add_summary_input = Some((input.clone(), subscription));
        input
    }

    pub(super) fn ensure_add_name_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.add_name_input {
            return input.clone();
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Optional name (derived from URL if empty)")
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |_view, _input, _event: &InputEvent, _, cx| {
                cx.notify();
            },
        );
        self.add_name_input = Some((input.clone(), subscription));
        input
    }

    pub(super) fn add_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url_field = self.ensure_add_url_input(window, cx);
        let summary_field = self.ensure_add_summary_input(window, cx);
        let name_field = self.ensure_add_name_input(window, cx);

        let url = url_field.read(cx).value().trim().to_string();
        let summary = summary_field.read(cx).value().trim().to_string();
        let name_raw = name_field.read(cx).value().trim().to_string();
        let name = if name_raw.is_empty() {
            None
        } else {
            Some(name_raw)
        };

        if url.is_empty() || summary.is_empty() || summary.len() > SUMMARY_MAX {
            self.write_error = Some("Valid URL and summary (≤ 110 chars) required.".to_string());
            cx.notify();
            return;
        }

        let repo = self.repo.clone();
        let key = format!("watch:{}", url);

        self.notice = None;
        self.write_error = None;

        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, "Adding project to watch list…"),
            move |shell| async move { shell.watch_upstream(&url, &summary, name.as_deref()).await },
            move |this, result, cx| {
                match result {
                    Some(row) => {
                        this.notice =
                            Some(format!("Added '{}' to upstreams.toml as watch.", row.name));
                        this.show_add_drawer = false;
                    }
                    None => {
                        this.write_error = Some("Failed to add project to watch list.".to_string());
                    }
                }
                this.load(cx);
            },
        );
    }

    pub(super) fn render_projects_tab(
        &mut self,
        systems: &[SystemRow],
        upstreams: &[UpstreamRow],
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let filter_field = self.ensure_filter_input(window, cx);
        let q = self.filter_query.to_lowercase();

        let filtered_systems: Vec<&SystemRow> = systems
            .iter()
            .filter(|s| {
                if q.is_empty() {
                    return true;
                }
                s.name.to_lowercase().contains(&q)
                    || s.why.to_lowercase().contains(&q)
                    || s.kind.to_lowercase().contains(&q)
            })
            .collect();

        let filtered_upstreams: Vec<UpstreamRow> = upstreams
            .iter()
            .filter(|u| {
                if q.is_empty() {
                    return true;
                }
                u.name.to_lowercase().contains(&q)
                    || u.summary.to_lowercase().contains(&q)
                    || u.verdict.to_lowercase().contains(&q)
                    || u.license.to_lowercase().contains(&q)
            })
            .cloned()
            .collect();

        let groups = group_upstreams(&filtered_upstreams);

        let mut panels = Vec::new();

        // 1. Add drawer
        let show_drawer = self.show_add_drawer;
        let drawer_toggle = Button::new("toggle-add-project")
            .small()
            .ghost()
            .label(if show_drawer {
                "Hide Watch Form"
            } else {
                "+ Add a project to watch"
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.show_add_drawer = !this.show_add_drawer;
                cx.notify();
            }));

        let add_header = div()
            .flex()
            .justify_between()
            .items_center()
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child("Watch a Project"),
            )
            .child(drawer_toggle);

        let add_card = if show_drawer {
            let url_field = self.ensure_add_url_input(window, cx);
            let sum_field = self.ensure_add_summary_input(window, cx);
            let name_field = self.ensure_add_name_input(window, cx);

            let sum_len = sum_field.read(cx).value().trim().len();
            let sum_color = if sum_len > SUMMARY_MAX {
                palette.red
            } else {
                palette.muted
            };

            let form_body = div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(add_header)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child("Repository URL"),
                        )
                        .child(Input::new(&url_field)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(rgb(palette.muted))
                                        .child("Summary"),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(rgb(sum_color))
                                        .child(format!("{sum_len} / {SUMMARY_MAX}")),
                                ),
                        )
                        .child(Input::new(&sum_field)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child("Name (optional)"),
                        )
                        .child(Input::new(&name_field)),
                )
                .child(
                    div().flex().justify_end().pt(px(4.0)).child(
                        Button::new("submit-watch-project")
                            .small()
                            .label("Add to Watch")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_project(window, cx);
                            })),
                    ),
                );

            card(form_body.into_any_element(), palette)
        } else {
            card(add_header.into_any_element(), palette)
        };
        panels.push(add_card);

        // 2. Search filter
        panels.push(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .w_full()
                .child(div().flex_1().child(Input::new(&filter_field)))
                .into_any_element(),
        );

        // 3. Runs alongside section (systems.toml)
        let sys_rows = filtered_systems.iter().map(|s| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .p(px(10.0))
                .bg(rgba(palette.surface))
                .rounded(px(6.0))
                .border_1()
                .border_color(rgba(palette.border))
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
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(palette.text))
                                        .child(s.name.clone()),
                                )
                                .when(s.local, |d| {
                                    d.child(
                                        div()
                                            .px(px(5.0))
                                            .py(px(1.0))
                                            .bg(rgba(palette.raised))
                                            .rounded(px(3.0))
                                            .text_size(px(9.0))
                                            .text_color(rgb(palette.muted))
                                            .child("local"),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .bg(rgba(palette.raised))
                                .rounded(px(4.0))
                                .text_size(px(10.0))
                                .text_color(rgb(palette.accent))
                                .child(s.kind.clone()),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(palette.text))
                        .child(s.why.clone()),
                )
                .when_some(s.url.clone(), |d, url| {
                    d.child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(palette.muted))
                            .child(url),
                    )
                })
                .into_any_element()
        });

        let systems_panel = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child(format!("Runs alongside ({})", filtered_systems.len())),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(
                                "Systems Sjel connects to and leaves in charge of their own job.",
                            ),
                    ),
            )
            .child(div().flex().flex_col().gap(px(6.0)).children(sys_rows));
        panels.push(card(systems_panel.into_any_element(), palette));

        // 4. Upstreams grouped by verdict
        for group in groups {
            if group.rows.is_empty() {
                continue;
            }

            let g_badge_color = match group.id.as_str() {
                "built-on" => palette.green,
                "learned-from" => palette.accent,
                "watching" => palette.amber,
                _ => palette.muted,
            };

            let rows = group.rows.iter().map(|u| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .p(px(8.0))
                    .bg(rgba(palette.surface))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .child(
                                        div()
                                            .text_size(px(13.0))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(palette.text))
                                            .child(u.name.clone()),
                                    )
                                    .when(!u.license.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .px(px(4.0))
                                                .py(px(1.0))
                                                .bg(rgba(palette.raised))
                                                .rounded(px(3.0))
                                                .text_size(px(9.0))
                                                .text_color(rgb(palette.muted))
                                                .child(u.license.clone()),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .px(px(6.0))
                                    .py(px(2.0))
                                    .bg(rgba(palette.raised))
                                    .rounded(px(4.0))
                                    .text_size(px(10.0))
                                    .text_color(rgb(g_badge_color))
                                    .child(u.verdict.clone()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(palette.text))
                            .child(u.summary.clone()),
                    )
                    .when(!u.url.is_empty(), |d| {
                        d.child(
                            div()
                                .text_size(px(10.0))
                                .text_color(rgb(palette.muted))
                                .child(u.url.clone()),
                        )
                    })
                    .into_any_element()
            });

            let g_panel = div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_size(px(14.0))
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(rgb(palette.text))
                                .child(format!("{} ({})", group.label, group.rows.len())),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child(group.blurb.clone()),
                        ),
                )
                .child(div().flex().flex_col().gap(px(6.0)).children(rows));

            panels.push(card(g_panel.into_any_element(), palette));
        }

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .children(panels)
            .into_any_element()
    }
}
