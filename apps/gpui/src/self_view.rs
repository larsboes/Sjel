//! SelfModel: architectural units, compile-time couplings, and live topology.
//!
//! Visualizes Sjel's 31+ architectural units (capabilities, libraries, packs, spines),
//! their compile-time couplings, runtime dependencies, and live daemon state.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, IntoElement, SharedString, Window,
};

use crate::data::{self, Busy, Remote, Repo};
use crate::theme::Palette;
use crate::ui::{card, dot, note, placeholder, section};
use sjel_gpui_core::self_model::{
    incoming_couplings, isolated_units, outgoing_couplings, unit_degree, unit_liveness,
    SelfModelResponse, SelfUnit,
};

const KINDS: [&str; 5] = ["all", "capability", "lib", "pack", "spine"];

pub struct SelfPage {
    repo: Repo,
    self_model: Remote<SelfModelResponse>,
    selected: Option<String>,
    filter_kind: &'static str,
    busy: Busy,
    notice: Option<String>,
}

impl SelfPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            self_model: Remote::default(),
            selected: None,
            filter_kind: "all",
            busy: Busy::default(),
            notice: None,
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.self_model,
            &repo,
            |shell| async move { shell.self_model().await },
        );
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    fn run_lifecycle(
        &mut self,
        unit: String,
        action: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = format!("lifecycle:{}:{}", unit, action);
        let repo = self.repo.clone();
        let target = unit.clone();
        let should_run = action == "start" || action == "restart";

        self.notice = None;

        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, "Service state changed."),
            move |shell| async move {
                if action == "restart" {
                    let _ = shell.set_running(&target, false).await;
                    shell.set_running(&target, true).await
                } else {
                    shell.set_running(&target, should_run).await
                }
            },
            |this, res, cx| {
                if res.is_some() {
                    this.notice = Some("Service state changed.".to_string());
                }
                this.load(cx);
            },
        );
    }

    // ── Components ────────────────────────────────────────────────────────────

    fn stats_summary(&self, res: &SelfModelResponse, palette: Palette) -> AnyElement {
        let model = &res.model;
        let units_count = model.units.len();
        let coupling_count = model.coupling.len();
        let upstreams_count = model.upstreams.len();
        let graph_nodes = model
            .graph
            .as_ref()
            .map(|g| g.nodes.to_string())
            .unwrap_or_else(|| "—".into());

        let stat_item = |label: &'static str, value: String| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_lg()
                        .font_weight(gpui_kit::FontWeight::BOLD)
                        .text_color(rgb(palette.text))
                        .child(value),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(label),
                )
        };

        card(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(stat_item("Units", units_count.to_string()))
                .child(stat_item("Couplings", coupling_count.to_string()))
                .child(stat_item("Graph Nodes", graph_nodes))
                .child(stat_item("Upstreams", upstreams_count.to_string()))
                .into_any_element(),
            palette,
        )
    }

    fn unit_card(
        &self,
        u: &SelfUnit,
        res: &SelfModelResponse,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_selected = self.selected.as_deref() == Some(&u.name);
        let liveness = unit_liveness(res, &u.name);
        let degree = unit_degree(&res.model, &u.name);
        let files = u.code.as_ref().map(|c| c.files).unwrap_or(0);
        let unit_name = u.name.clone();

        let dot_color = match liveness {
            Some(true) => palette.green,
            Some(false) => palette.amber,
            None => palette.border,
        };

        let card_border = if is_selected {
            palette.accent
        } else {
            palette.border
        };

        div()
            .id(SharedString::from(format!("unit-card-{}", u.name)))
            .flex()
            .flex_col()
            .justify_between()
            .p_3()
            .min_w(px(220.))
            .min_h(px(90.))
            .rounded(px(8.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(card_border))
            .cursor_pointer()
            .hover(|s| s.bg(rgba(palette.raised)))
            .on_click(cx.listener(move |page, _, _, cx| {
                if page.selected.as_deref() == Some(&unit_name) {
                    page.selected = None;
                } else {
                    page.selected = Some(unit_name.clone());
                }
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(dot(dot_color))
                            .child(
                                div()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .text_sm()
                                    .text_color(rgb(palette.text))
                                    .child(u.name.clone()),
                            ),
                    )
                    .child(
                        div()
                            .px_1()
                            .py(px(1.))
                            .rounded(px(3.))
                            .bg(rgba(palette.raised))
                            .text_color(rgb(palette.muted))
                            .text_xs()
                            .child(u.kind.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt_2()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(format!("{files} files"))
                    .child(if degree > 0 {
                        format!("{degree} couplings")
                    } else {
                        "isolated".to_string()
                    }),
            )
            .into_any_element()
    }

    fn unit_inspector(
        &mut self,
        res: &SelfModelResponse,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let name = self.selected.as_ref()?;
        let unit = res.model.units.iter().find(|u| &u.name == name)?;
        let is_busy = !self.busy.is_empty();

        let liveness = unit_liveness(res, &unit.name);
        let outgoing = outgoing_couplings(&res.model, &unit.name);
        let incoming = incoming_couplings(&res.model, &unit.name);

        let mut header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_base()
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child(unit.name.clone()),
                    )
                    .child(
                        div()
                            .px_2()
                            .py(px(1.5))
                            .rounded(px(4.))
                            .bg(rgba(palette.raised))
                            .text_color(rgb(palette.muted))
                            .text_xs()
                            .child(unit.kind.clone()),
                    ),
            );

        if let Some(service) = &unit.service {
            let is_up = liveness == Some(true);
            let u_name_toggle = unit.name.clone();
            let action_toggle = if is_up { "stop" } else { "start" };
            let toggle_label = if is_up { "Stop" } else { "Start" };

            let u_name_restart = unit.name.clone();
            let toggle_button = Button::new("toggle-unit-service")
                .small()
                .disabled(is_busy)
                .label(toggle_label)
                .on_click(cx.listener(move |page, _, window, cx| {
                    page.run_lifecycle(u_name_toggle.clone(), action_toggle, window, cx);
                }));

            let restart_button = Button::new("restart-unit-service")
                .small()
                .disabled(is_busy)
                .label("↻ Restart")
                .on_click(cx.listener(move |page, _, window, cx| {
                    page.run_lifecycle(u_name_restart.clone(), "restart", window, cx);
                }));

            header = header.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .children(service.port.as_ref().map(|p| {
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("port :{p}"))
                    }))
                    .child(toggle_button)
                    .child(restart_button),
            );
        }

        let mut details = Vec::new();

        // Dependencies (requires)
        if let Some(service) = &unit.service {
            if !service.requires.is_empty() {
                details.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.muted))
                                .child("Runtime Dependencies"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.text))
                                .child(service.requires.join(", ")),
                        ),
                );
            }
        }

        // Outgoing couplings
        if !outgoing.is_empty() {
            let out_labels: Vec<String> = outgoing
                .iter()
                .map(|c| format!("{} ({})", c.to, c.kinds.join(", ")))
                .collect();
            details.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("Compiles In (Outgoing)"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.text))
                            .child(out_labels.join(" · ")),
                    ),
            );
        }

        // Incoming couplings
        if !incoming.is_empty() {
            let in_labels: Vec<String> = incoming
                .iter()
                .map(|c| format!("{} ({})", c.from, c.kinds.join(", ")))
                .collect();
            details.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("Compiled Into (Incoming)"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.text))
                            .child(in_labels.join(" · ")),
                    ),
            );
        }

        Some(card(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(header)
                .children(details)
                .into_any_element(),
            palette,
        ))
    }
}

impl gpui::Render for SelfPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());

        let Some(res) = self.self_model.value.clone() else {
            let body = match &self.self_model.error {
                Some(error) => note(error.to_string(), palette.red),
                None => placeholder(320.),
            };
            return section("Self-Model", body, palette).into_any_element();
        };

        let mut panels = Vec::new();

        // Notices
        if let Some(notice) = &self.notice {
            panels.push(card(
                div()
                    .text_xs()
                    .text_color(rgb(palette.green))
                    .child(notice.clone())
                    .into_any_element(),
                palette,
            ));
        }

        // Stats Card
        panels.push(self.stats_summary(&res, palette));

        // Inspector if selected
        if let Some(inspector) = self.unit_inspector(&res, palette, cx) {
            panels.push(inspector);
        }

        // Filter Pills
        let filter_pills: Vec<AnyElement> = KINDS
            .iter()
            .map(|kind| {
                let is_active = self.filter_kind == *kind;
                let k = *kind;
                Button::new(SharedString::from(format!("filter-kind-{}", kind)))
                    .small()
                    .when(!is_active, |b| b.ghost())
                    .label(if *kind == "all" { "All units" } else { *kind })
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.filter_kind = k;
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();

        panels.push(
            div()
                .flex()
                .items_center()
                .gap_2()
                .children(filter_pills)
                .into_any_element(),
        );

        // Filter units
        let matching_units: Vec<&SelfUnit> = res
            .model
            .units
            .iter()
            .filter(|u| self.filter_kind == "all" || u.kind == self.filter_kind)
            .collect();

        let unit_cards: Vec<AnyElement> = matching_units
            .into_iter()
            .map(|u| self.unit_card(u, &res, palette, cx))
            .collect();

        panels.push(
            div()
                .flex()
                .flex_wrap()
                .gap_3()
                .children(unit_cards)
                .into_any_element(),
        );

        // Isolated units section if on "all"
        let isolated = isolated_units(&res.model);
        if self.filter_kind == "all" && !isolated.is_empty() {
            let isolated_lead = div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!(
                    "{} units compile nothing in and are compiled into nothing (self-contained).",
                    isolated.len()
                ));
            panels.push(isolated_lead.into_any_element());
        }

        div()
            .flex()
            .flex_col()
            .gap_6()
            .children(panels)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_self_page_initial_state() {
        let repo = Repo::new(Err(sjel_gpui_core::shell::ShellError::NotSetUp));
        let page = SelfPage::new(repo);
        assert_eq!(page.filter_kind, "all");
        assert!(page.selected.is_none());
    }
}
