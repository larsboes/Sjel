//! The skill matrix: one row per (pack, skill), one column per installed harness.

use gpui_kit::component::button::Button;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, rgb, rgba, AnyElement, Context, IntoElement, ScrollHandle, SharedString,
};
use sjel_gpui_core::packs::{
    installed_harnesses, row_needs_attention, status_of, MatrixRow, PacksView,
};

use super::types::{column, pill, HARNESS_MIN, PACK_COL, PACK_MIN, SKILL_COL, SKILL_MIN};
use super::PacksPage;
use crate::theme::Palette;

/// The skill matrix: one row per (pack, skill), one column per installed harness. Header,
/// separators and rows share one rounded container and one set of column widths.
pub(crate) fn render_matrix_table(
    page: &PacksPage,
    view: &PacksView,
    rows: &[MatrixRow],
    matrix_scroll: &ScrollHandle,
    palette: Palette,
    cx: &mut Context<PacksPage>,
) -> AnyElement {
    let shown = installed_harnesses(view);
    let is_busy = !page.busy.is_empty();
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
        if page.only_attention && !row_needs_attention(view, r) {
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
                    .text_color(rgb(palette.muted))
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
        .track_scroll(matrix_scroll)
        .horizontal_scrollbar(matrix_scroll)
        .child(table)
        .into_any_element()
}
