//! SelfModel: architectural units, compile-time couplings, and live topology.
//!
//! Visualizes Sjel's 31+ architectural units (capabilities, libraries, packs, spines),
//! their compile-time couplings, runtime dependencies, and live daemon state.

pub mod cards;
pub mod constants;
pub mod inspector;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, rgb, AnyElement, Context, IntoElement, SharedString, Window,
};

use crate::data::{self, Busy, Remote, Repo};
use crate::primitives::{card, note, placeholder, section};
use crate::theme::ThemeContext;
use sjel_gpui_core::self_model::{
    isolated_units, SelfModelResponse, SelfUnit,
};

use self::cards::{stats_summary, unit_card};
use self::constants::KINDS;
use self::inspector::unit_inspector;

pub struct SelfPage {
    pub(crate) repo: Repo,
    pub(crate) self_model: Remote<SelfModelResponse>,
    pub(crate) selected: Option<String>,
    pub(crate) filter_kind: &'static str,
    pub(crate) busy: Busy,
    pub(crate) notice: Option<String>,
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

    pub(crate) fn run_lifecycle(
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
}

impl gpui::Render for SelfPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;

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
        panels.push(stats_summary(&res, palette));

        // Inspector if selected
        if let Some(inspector) = unit_inspector(self, &res, palette, cx) {
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
            .map(|u| unit_card(self, u, &res, palette, cx))
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
