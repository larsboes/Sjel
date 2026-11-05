//! Packs: agent skills deployed across installed harnesses on this machine.
//!
//! Visualizes which skills are current, drifted, outdated, or uninstalled across harnesses
//! (`Claude Code`, `Pi`, `Codex`, `OpenCode`). Provides one-click sync, deploy, and inspect
//! operations backed by `sjel-status` and `tools/harnesses`.

mod cards;
mod matrix;
mod types;

use std::collections::BTreeMap;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, rgb, AnyElement, Context, IntoElement, ScrollHandle, Window};

use crate::data::{self, Busy, Remote, Repo};
use crate::primitives::{card, note, placeholder, section};
use crate::theme::ThemeContext;
use sjel_gpui_core::packs::{all_packs, matrix_rows, PacksView};

pub(crate) use cards::{render_harness_cards, render_orphan_cards, render_strays_section};
pub(crate) use matrix::render_matrix_table;
pub(crate) use types::Derived;

pub struct PacksPage {
    pub(crate) repo: Repo,
    /// Behind an `Arc` so a render does not clone the payload.
    pub(crate) packs: Remote<Arc<PacksView>>,
    /// What the payload reduces to, recomputed when it settles and not once per frame.
    pub(crate) derived: Option<Derived>,
    pub(crate) busy: Busy,
    pub(crate) notice: Option<String>,
    pub(crate) write_error: Option<String>,
    pub(crate) chosen: BTreeMap<String, String>,
    pub(crate) only_attention: bool,
    /// The page scrolls itself, so the shell gives it the full window width.
    pub(crate) scroll: ScrollHandle,
    /// Horizontal scroll once the harness columns hit their minimum widths.
    pub(crate) matrix_scroll: ScrollHandle,
}

impl PacksPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            packs: Remote::default(),
            derived: None,
            busy: Busy::default(),
            notice: None,
            write_error: None,
            chosen: BTreeMap::new(),
            only_attention: false,
            scroll: ScrollHandle::new(),
            matrix_scroll: ScrollHandle::new(),
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.packs,
            &repo,
            |shell| async move { shell.packs().await.map(Arc::new) },
        );
    }

    /// Derive the matrix once per settled read, not once per frame.
    fn settle_derived(&mut self) {
        let Some(view) = self.packs.value.as_ref() else {
            self.derived = None;
            return;
        };
        let loaded_at = self.packs.loaded_at;
        if self
            .derived
            .as_ref()
            .is_some_and(|d| d.loaded_at == loaded_at)
        {
            return;
        }
        let rows = matrix_rows(view);
        let pack_names = all_packs(&rows);
        self.derived = Some(Derived {
            loaded_at,
            rows,
            pack_names,
        });
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn scroll_handle(&self) -> ScrollHandle {
        self.scroll.clone()
    }

    pub(crate) fn write(
        &mut self,
        action: &'static str,
        harness: String,
        packs: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = format!("{}:{}:{}", action, harness, packs.join(","));
        let action_str = action.to_string();
        let harness_clone = harness.clone();
        let packs_clone = packs.clone();
        let repo = self.repo.clone();

        self.notice = None;
        self.write_error = None;

        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, "Pack operation executed."),
            move |shell| async move {
                shell
                    .write_pack(&action_str, &harness_clone, &packs_clone)
                    .await
            },
            |this, result, cx| {
                match result {
                    Some(res) => {
                        this.notice = Some(format!(
                            "{} {} {} — {}",
                            res.action,
                            res.harness,
                            res.packs.join(", "),
                            if res.detail.is_empty() {
                                "done"
                            } else {
                                &res.detail
                            }
                        ));
                    }
                    None => {
                        // Error already captured by data::write or shell error
                    }
                }
                this.load(cx);
            },
        );
    }

    pub(crate) fn pick_pack<'a>(&'a self, harness_id: &str, all: &'a [String]) -> &'a str {
        if let Some(p) = self.chosen.get(harness_id) {
            if all.iter().any(|x| x == p) {
                return p.as_str();
            }
        }
        all.first().map(|s| s.as_str()).unwrap_or("")
    }

    /// The page's own scroll boundary; the shell then gives it the full window width.
    fn frame(&self, body: AnyElement) -> impl IntoElement {
        div()
            .id("packs-page")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .vertical_scrollbar(&self.scroll)
            .child(body)
    }
}

impl gpui::Render for PacksPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        self.settle_derived();

        let Some(payload) = self.packs.value.clone() else {
            let body = match &self.packs.error {
                Some(error) => note(error.to_string(), palette.red),
                None => placeholder(320.),
            };
            return self
                .frame(section("Packs", body, palette).into_any_element())
                .into_any_element();
        };
        let Some(derived) = self.derived.as_ref() else {
            return self
                .frame(section("Packs", placeholder(320.), palette).into_any_element())
                .into_any_element();
        };
        let view = payload.as_ref();

        let mut panels = Vec::new();

        // Notices or errors
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
        if let Some(err) = &self.write_error {
            panels.push(card(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(err.clone())
                    .into_any_element(),
                palette,
            ));
        }

        // Orphaned alerts
        for orphan in render_orphan_cards(view, palette) {
            panels.push(orphan);
        }

        // Harness summary cards
        panels.push(render_harness_cards(
            self,
            view,
            &derived.pack_names,
            palette,
            cx,
        ));

        // Filter bar + Matrix
        let filter_button = Button::new("toggle-attention-filter")
            .small()
            .when(!self.only_attention, |b| b.ghost())
            .label(if self.only_attention {
                "Showing: Attention needed"
            } else {
                "Filter: All rows"
            })
            .on_click(cx.listener(|page, _, _, cx| {
                page.only_attention = !page.only_attention;
                cx.notify();
            }));

        let matrix_header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_xs()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child("Skill Matrix"),
            )
            .child(filter_button);

        let matrix =
            render_matrix_table(self, view, &derived.rows, &self.matrix_scroll, palette, cx);
        panels.push(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(matrix_header)
                .child(matrix)
                .into_any_element(),
        );

        // Strays
        for stray in render_strays_section(view, palette) {
            panels.push(stray);
        }

        self.frame(
            div()
                .flex()
                .flex_col()
                .gap_6()
                .max_w(px(1280.))
                .children(panels)
                .into_any_element(),
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pick_pack_defaults_to_first() {
        let repo = Repo::new(Err(sjel_gpui_core::shell::ShellError::NotSetUp));
        let page = PacksPage::new(repo);
        let packs = vec!["coding".to_string(), "deliberation".to_string()];
        assert_eq!(page.pick_pack("claude", &packs), "coding");
    }
}
