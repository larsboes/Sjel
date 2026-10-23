//! Packs: agent skills deployed across installed harnesses on this machine.
//!
//! Visualizes which skills are current, drifted, outdated, or uninstalled across harnesses
//! (`Claude Code`, `Pi`, `Codex`, `OpenCode`). Provides one-click sync, deploy, and inspect
//! operations backed by `sjel-status` and `tools/harnesses`.

use std::collections::BTreeMap;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, IntoElement, SharedString, Window,
};

use crate::data::{self, Busy, Remote, Repo};
use crate::theme::Palette;
use crate::ui::{card, dot, list, note, placeholder, section};
use sjel_gpui_core::packs::{
    all_packs, deployed_units, held_packs, installed_harnesses, matrix_rows, orphaned_harnesses,
    row_needs_attention, status_of, MatrixRow, PacksView,
};

pub struct PacksPage {
    repo: Repo,
    packs: Remote<PacksView>,
    busy: Busy,
    notice: Option<String>,
    write_error: Option<String>,
    chosen: BTreeMap<String, String>,
    only_attention: bool,
}

impl PacksPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            packs: Remote::default(),
            busy: Busy::default(),
            notice: None,
            write_error: None,
            chosen: BTreeMap::new(),
            only_attention: false,
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.packs,
            &repo,
            |shell| async move { shell.packs().await },
        );
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
        &mut self,
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
                            palette.green & 0xffffff | 0x22000000
                        } else {
                            palette.amber & 0xffffff | 0x22000000
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

    fn matrix_table(
        &mut self,
        view: &PacksView,
        rows: &[MatrixRow],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let shown_harnesses = installed_harnesses(view);
        let is_busy = !self.busy.is_empty();

        // Header row
        let mut header_row = div()
            .flex()
            .items_center()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(rgba(palette.border))
            .text_xs()
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .text_color(rgb(palette.muted))
            .child(div().w(px(140.)).child("Pack"))
            .child(div().w(px(180.)).child("Skill"));

        for h in &shown_harnesses {
            header_row = header_row.child(div().w(px(110.)).child(h.label.clone()));
        }

        let mut body_rows = Vec::new();
        for r in rows {
            if self.only_attention && !row_needs_attention(view, r) {
                continue;
            }

            let mut row_el = div()
                .flex()
                .items_center()
                .px_3()
                .py_2()
                .text_xs()
                .child(
                    div()
                        .w(px(140.))
                        .text_color(rgb(palette.muted))
                        .child(r.pack.clone()),
                )
                .child(
                    div()
                        .w(px(180.))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(rgb(palette.text))
                        .child(r.skill.clone()),
                );

            for h in &shown_harnesses {
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
                    "drifted" => div()
                        .px_2()
                        .py(px(1.))
                        .rounded(px(4.))
                        .bg(rgba(palette.amber & 0xffffff | 0x22000000))
                        .text_color(rgb(palette.amber))
                        .child("drifted")
                        .into_any_element(),
                    "not-deployed" => div()
                        .text_color(rgb(palette.border))
                        .child("-")
                        .into_any_element(),
                    other => div()
                        .px_2()
                        .py(px(1.))
                        .rounded(px(4.))
                        .bg(rgba(palette.red & 0xffffff | 0x22000000))
                        .text_color(rgb(palette.red))
                        .child(other.to_string())
                        .into_any_element(),
                };

                row_el = row_el.child(div().w(px(110.)).child(cell));
            }

            body_rows.push(row_el.into_any_element());
        }

        div()
            .flex()
            .flex_col()
            .rounded(px(12.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .overflow_hidden()
            .child(header_row)
            .child(list(body_rows, palette))
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
}

impl gpui::Render for PacksPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());

        let Some(view) = self.packs.value.clone() else {
            let body = match &self.packs.error {
                Some(error) => note(error.to_string(), palette.red),
                None => placeholder(320.),
            };
            return section("Packs", body, palette).into_any_element();
        };

        let rows = matrix_rows(&view);
        let packs_list = all_packs(&rows);

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
        for orphan in self.orphan_cards(&view, palette) {
            panels.push(orphan);
        }

        // Harness summary cards
        panels.push(self.harness_cards(&view, &packs_list, palette, cx));

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

        let matrix = self.matrix_table(&view, &rows, palette, cx);
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
        for stray in self.strays_section(&view, palette) {
            panels.push(stray);
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
    fn test_pick_pack_defaults_to_first() {
        let repo = Repo::new(Err(sjel_gpui_core::shell::ShellError::NotSetUp));
        let page = PacksPage::new(repo);
        let packs = vec!["coding".to_string(), "deliberation".to_string()];
        assert_eq!(page.pick_pack("claude", &packs), "coding");
    }
}
