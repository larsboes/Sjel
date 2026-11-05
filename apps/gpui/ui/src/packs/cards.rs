//! Orphaned harness warnings, harness summary cards, and unowned stray listings.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, rgb, rgba, AnyElement, Context, IntoElement, SharedString};
use sjel_gpui_core::packs::{deployed_units, held_packs, orphaned_harnesses, PacksView};

use super::PacksPage;
use crate::primitives::{card, dot, list};
use crate::theme::{tint, Palette};

pub(crate) fn render_orphan_cards(view: &PacksView, palette: Palette) -> Vec<AnyElement> {
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

pub(crate) fn render_harness_cards(
    page: &PacksPage,
    view: &PacksView,
    all: &[String],
    palette: Palette,
    cx: &mut Context<PacksPage>,
) -> AnyElement {
    let is_busy = !page.busy.is_empty();
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
        let selected_pack = page.pick_pack(&h_id, all).to_string();

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

        let meta = div()
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

pub(crate) fn render_strays_section(view: &PacksView, palette: Palette) -> Vec<AnyElement> {
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
