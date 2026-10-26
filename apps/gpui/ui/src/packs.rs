//! Packs: agent skills deployed across installed harnesses on this machine.
//!
//! Visualizes which skills are current, drifted, outdated, or uninstalled across harnesses
//! (`Claude Code`, `Pi`, `Codex`, `OpenCode`). Provides one-click sync, deploy, and inspect
//! operations backed by `sjel-status` and `tools/harnesses`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, relative, rgb, rgba, AnyElement, Context, IntoElement, ScrollHandle,
    SharedString, Window,
};

use crate::data::{self, Busy, Remote, Repo};
use crate::primitives::{card, dot, list, note, placeholder, section};
use crate::theme::{tint, Palette};
use sjel_gpui_core::packs::{
    all_packs, deployed_units, held_packs, installed_harnesses, matrix_rows, orphaned_harnesses,
    row_needs_attention, status_of, MatrixRow, PacksView,
};

pub struct PacksPage {
    repo: Repo,
    /// Behind an `Arc` so a render does not clone the payload.
    packs: Remote<Arc<PacksView>>,
    /// What the payload reduces to, recomputed when it settles and not once per frame.
    derived: Option<Derived>,
    busy: Busy,
    notice: Option<String>,
    write_error: Option<String>,
    chosen: BTreeMap<String, String>,
    only_attention: bool,
    /// The page scrolls itself, so the shell gives it the full window width.
    scroll: ScrollHandle,
    /// Horizontal scroll once the harness columns hit their minimum widths.
    matrix_scroll: ScrollHandle,
}

/// The payload's reductions, kept until a new measurement settles.
struct Derived {
    loaded_at: Option<Instant>,
    rows: Vec<MatrixRow>,
    pack_names: Vec<String>,
}

// Matrix column widths: share of the row and minimum. Header and body share them, so they align.
const PACK_COL: f32 = 0.16;
const SKILL_COL: f32 = 0.26;
const PACK_MIN: f32 = 96.;
const SKILL_MIN: f32 = 132.;
const HARNESS_MIN: f32 = 84.;

/// One matrix column, sized by its share of the row rather than by what it holds.
fn column(basis: f32, min: f32, inner: AnyElement) -> impl IntoElement {
    div()
        .flex_basis(relative(basis))
        .flex_shrink_1()
        .min_w(px(min))
        .child(inner)
}

/// A status chip: the tone says what the value means, the label names it.
fn pill(tone: u32, label: String) -> AnyElement {
    div()
        .px_2()
        .py(px(1.))
        .rounded(px(4.))
        .bg(rgba(tint(tone, 0x22)))
        .text_color(rgb(tone))
        .child(label)
        .into_any_element()
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

    fn write(
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

    fn pick_pack<'a>(&'a self, harness_id: &str, all: &'a [String]) -> &'a str {
        if let Some(p) = self.chosen.get(harness_id) {
            if all.iter().any(|x| x == p) {
                return p.as_str();
            }
        }
        all.first().map(|s| s.as_str()).unwrap_or("")
    }

    // ── Components ────────────────────────────────────────────────────────────

    fn orphan_cards(&self, view: &PacksView, palette: Palette) -> Vec<AnyElement> {
        orphaned_harnesses(view)
            .into_iter()
            .map(|harness| {
                let count = deployed_units(harness).len();
                let packs = held_packs(harness).join(" ");
                card(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(dot(palette.amber))
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                        .text_color(rgb(palette.amber))
                                        .child(format!(
                                            "{} is not installed, and {} units are deployed at its skill root",
                                            harness.label, count
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!(
                                    "Nothing on this machine reads them. Run: {} remove {}",
                                    harness.cli, packs
                                )),
                        )
                        .into_any_element(),
                    palette,
                )
            })
            .collect()
    }

    fn harness_cards(
        &self,
        view: &PacksView,
        all: &[String],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_busy = !self.busy.is_empty();
        let mut cards = Vec::new();

        for harness in &view.harnesses {
            let h_id = harness.id.clone();
            let label = harness.label.clone();
            let installed = harness.installed;
            let model = harness.model.clone();
            let dest = harness
                .destination
                .clone()
                .unwrap_or_else(|| harness.marker.clone());
            let deployed_count = deployed_units(harness).len();
            let unowned_count = harness.unowned.len();
            let held = held_packs(harness);
            let selected_pack = self.pick_pack(&h_id, all).to_string();

            let header = div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_sm()
                        .text_color(rgb(palette.text))
                        .child(label),
                )
                .child(
                    div()
                        .px_2()
                        .py(px(1.5))
                        .rounded(px(4.))
                        .bg(rgba(if installed {
                            tint(palette.green, 0x22)
                        } else {
                            tint(palette.amber, 0x22)
                        }))
                        .text_color(rgb(if installed {
                            palette.green
                        } else {
                            palette.amber
                        }))
                        .text_xs()
                        .child(if installed { "installed" } else { "absent" }),
                );

            let meta =
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(if model == "registry" {
                        "reads the Pack source in place"
                    } else {
                        "copies, and can drift"
                    });

            let counts = div().text_xs().text_color(rgb(palette.text)).child(format!(
                "{} deployed{}",
                deployed_count,
                if unowned_count > 0 {
                    format!(" · {unowned_count} unowned")
                } else {
                    "".to_string()
                }
            ));

            let path = div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .truncate()
                .child(dest);

            let mut card_content = div()
                .flex()
                .flex_col()
                .gap_2()
                .child(header)
                .child(meta)
                .child(counts)
                .child(path);

            if installed {
                let h_id_for_sync = h_id.clone();
                let held_for_sync = held.clone();
                let sync_button = Button::new(SharedString::from(format!("sync-{}", h_id)))
                    .small()
                    .label("Sync all")
                    .disabled(is_busy || held.is_empty())
                    .on_click(cx.listener(move |page, _, window, cx| {
                        page.write(
                            "sync",
                            h_id_for_sync.clone(),
                            held_for_sync.clone(),
                            window,
                            cx,
                        );
                    }));

                let h_id_for_deploy = h_id.clone();
                let pack_for_deploy = selected_pack.clone();
                let deploy_button = Button::new(SharedString::from(format!("deploy-{}", h_id)))
                    .small()
                    .label("Deploy")
                    .disabled(is_busy || pack_for_deploy.is_empty())
                    .on_click(cx.listener(move |page, _, window, cx| {
                        page.write(
                            "deploy",
                            h_id_for_deploy.clone(),
                            vec![pack_for_deploy.clone()],
                            window,
                            cx,
                        );
                    }));

                let h_id_for_remove = h_id.clone();
                let pack_for_remove = selected_pack.clone();
                let remove_button = Button::new(SharedString::from(format!("remove-{}", h_id)))
                    .small()
                    .danger()
                    .label("Remove")
                    .disabled(is_busy || pack_for_remove.is_empty())
                    .on_click(cx.listener(move |page, _, window, cx| {
                        page.write(
                            "remove",
                            h_id_for_remove.clone(),
                            vec![pack_for_remove.clone()],
                            window,
                            cx,
                        );
                    }));

                let h_id_cycle = h_id.clone();
                let all_packs_list = all.to_vec();
                let current_selection = selected_pack.clone();
                let pack_picker = div()
                    .id(SharedString::from(format!("cycle-pack-{}", h_id)))
                    .px_2()
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(rgba(palette.raised))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .text_xs()
                    .cursor_pointer()
                    .child(if current_selection.is_empty() {
                        "select pack".to_string()
                    } else {
                        current_selection
                    })
                    .on_click(cx.listener(move |page, _, _, cx| {
                        if all_packs_list.is_empty() {
                            return;
                        }
                        let next = match all_packs_list.iter().position(|p| p == &selected_pack) {
                            Some(idx) => all_packs_list[(idx + 1) % all_packs_list.len()].clone(),
                            None => all_packs_list[0].clone(),
                        };
                        page.chosen.insert(h_id_cycle.clone(), next);
                        cx.notify();
                    }));

                let actions = div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .pt_2()
                    .child(sync_button)
                    .child(pack_picker)
                    .child(deploy_button)
                    .child(remove_button);

                card_content = card_content.child(actions);
            }

            cards.push(
                div()
                    .flex_1()
                    .min_w(px(260.))
                    .child(card(card_content.into_any_element(), palette)),
            );
        }

        div()
            .flex()
            .flex_wrap()
            .gap_3()
            .children(cards)
            .into_any_element()
    }

    /// The skill matrix: one row per (pack, skill), one column per installed harness. Header,
    /// separators and rows share one rounded container and one set of column widths.
    fn matrix_table(
        &self,
        view: &PacksView,
        rows: &[MatrixRow],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let shown = installed_harnesses(view);
        let is_busy = !self.busy.is_empty();
        let columns = shown.len().max(1) as f32;
        let harness_col = (1. - PACK_COL - SKILL_COL) / columns;

        let header = div()
            .flex()
            .items_center()
            .gap_1()
            .px_3()
            .py_2()
            .text_xs()
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .text_color(rgb(palette.muted))
            .child(column(
                PACK_COL,
                PACK_MIN,
                div().child("Pack").into_any_element(),
            ))
            .child(column(
                SKILL_COL,
                SKILL_MIN,
                div().child("Skill").into_any_element(),
            ))
            .children(shown.iter().map(|h| {
                column(
                    harness_col,
                    HARNESS_MIN,
                    div().truncate().child(h.label.clone()).into_any_element(),
                )
            }));

        let mut body = Vec::new();
        for r in rows {
            if self.only_attention && !row_needs_attention(view, r) {
                continue;
            }

            let mut row = div()
                .flex()
                .items_center()
                .gap_1()
                .px_3()
                .py_2()
                .text_xs()
                .child(column(
                    PACK_COL,
                    PACK_MIN,
                    div()
                        .truncate()
                        .text_color(rgb(palette.muted))
                        .child(r.pack.clone())
                        .into_any_element(),
                ))
                .child(column(
                    SKILL_COL,
                    SKILL_MIN,
                    div()
                        .truncate()
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(rgb(palette.text))
                        .child(r.skill.clone())
                        .into_any_element(),
                ));

            for h in &shown {
                let status = status_of(h, &r.pack, &r.skill);
                let cell: AnyElement = match status {
                    "current" => div()
                        .text_color(rgb(palette.muted))
                        .child("·")
                        .into_any_element(),
                    "outdated" | "missing" => {
                        let is_missing = status == "missing";
                        let verb = if is_missing { "Deploy" } else { "Sync" };
                        let action = if is_missing { "deploy" } else { "sync" };
                        let h_id = h.id.clone();
                        let p_name = r.pack.clone();
                        Button::new(SharedString::from(format!(
                            "cell-{}-{}-{}",
                            h.id, r.pack, r.skill
                        )))
                        .small()
                        .label(verb)
                        .disabled(is_busy)
                        .on_click(cx.listener(move |page, _, window, cx| {
                            page.write(action, h_id.clone(), vec![p_name.clone()], window, cx);
                        }))
                        .into_any_element()
                    }
                    "drifted" => pill(palette.amber, "drifted".to_string()),
                    "not-deployed" => div()
                        .text_color(rgb(palette.border))
                        .child("-")
                        .into_any_element(),
                    other => pill(palette.red, other.to_string()),
                };

                row = row.child(column(harness_col, HARNESS_MIN, cell));
            }

            body.push(row.into_any_element());
        }

        let mut table = div()
            .min_w(px(PACK_MIN + SKILL_MIN + columns * HARNESS_MIN))
            .flex()
            .flex_col()
            .rounded(px(12.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .overflow_hidden()
            .child(header);
        for row in body {
            table = table
                .child(div().h(px(1.)).bg(rgba(palette.border)))
                .child(row);
        }

        // Scroll sideways once the harness minimums outgrow the page.
        div()
            .id("pack-matrix")
            .overflow_x_scroll()
            .track_scroll(&self.matrix_scroll)
            .horizontal_scrollbar(&self.matrix_scroll)
            .child(table)
            .into_any_element()
    }

    fn strays_section(&self, view: &PacksView, palette: Palette) -> Vec<AnyElement> {
        let mut sections = Vec::new();

        for harness in &view.harnesses {
            if harness.unowned.is_empty() {
                continue;
            }
            let rows: Vec<AnyElement> = harness
                .unowned
                .iter()
                .map(|stray| {
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_3()
                        .py_2()
                        .text_xs()
                        .child(
                            div()
                                .px_1()
                                .py(px(1.))
                                .rounded(px(3.))
                                .bg(rgba(palette.raised))
                                .text_color(rgb(if stray.kind == "copy" {
                                    palette.amber
                                } else {
                                    palette.muted
                                }))
                                .child(stray.kind.clone()),
                        )
                        .child(
                            div()
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .text_color(rgb(palette.text))
                                .child(stray.name.clone()),
                        )
                        .children(
                            stray
                                .detail
                                .as_ref()
                                .map(|d| div().text_color(rgb(palette.muted)).child(d.clone())),
                        )
                        .into_any_element()
                })
                .collect();

            let title = format!("{}: unowned at destination", harness.label);
            sections.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child(title),
                    )
                    .child(list(rows, palette))
                    .into_any_element(),
            );
        }

        sections
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
        let palette = Palette::for_appearance(window.appearance());
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
        for orphan in self.orphan_cards(view, palette) {
            panels.push(orphan);
        }

        // Harness summary cards
        panels.push(self.harness_cards(view, &derived.pack_names, palette, cx));

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

        let matrix = self.matrix_table(view, &derived.rows, palette, cx);
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
        for stray in self.strays_section(view, palette) {
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
