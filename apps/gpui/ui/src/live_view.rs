//! Shared generative A2UI live view renderer built on Ely primitives.
//!
//! Renders validated [`ValidatedA2uiView`] documents across Agent Seat, Mac Home/Today, and
//! iPhone Today without executing model code, preserving data-class, read-route, and owner-mode
//! proposal boundaries.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    div, prelude::*, px, rems, rgb, rgba, AnyElement, App, Entity, FontWeight, SharedString,
};
use sjel_gpui_core::a2ui::{A2uiClientState, ComponentKind, ValidatedA2uiView};

use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::dashboard::{MetricStrip, MetricTile};
use crate::ely::data_display::{Badge, DescriptionList, Tone};
use crate::ely::forms::{Input, TextInput};
use crate::ely::layout::{Card, CardHeader};
use crate::ely::tables::{Cell as TableCell, Column as TableColumn, DataTable, Row as TableRow};
use crate::ely::theme::ControlSize;
use crate::theme::Palette;

/// Callback invoked when a user clicks an owner-mode proposal button inside a live view.
/// Arguments are `(surface_id, button_node_id, app)`.
pub type ProposeHandler = Arc<dyn Fn(&str, &str, &mut App) + Send + Sync>;

/// Optional map of `/draft/<key>` keys to live [`TextInput`] entities for interactive A2UI forms.
pub type DraftInputs<'a> = &'a HashMap<String, Entity<TextInput>>;

/// Maps an A2UI tone string onto an Ely [`Tone`].
pub fn tone_of(raw: Option<&str>) -> Tone {
    match raw {
        Some("accent") => Tone::Accent,
        Some("success") => Tone::Success,
        Some("warning") => Tone::Warning,
        Some("danger") => Tone::Danger,
        _ => Tone::Neutral,
    }
}

/// Renders a validated A2UI view document using Ely primitives while displaying its data class,
/// bound read routes, and owner-mode proposal badge.
pub fn render_live_view(
    view: &ValidatedA2uiView,
    view_state: &A2uiClientState,
    on_propose: ProposeHandler,
    palette: Palette,
) -> AnyElement {
    render_live_view_interactive(view, view_state, on_propose, None, palette)
}

/// Renders a validated A2UI view document with optional live [`TextInput`] bindings for
/// `ComponentKind::TextField` nodes.
pub fn render_live_view_interactive(
    view: &ValidatedA2uiView,
    view_state: &A2uiClientState,
    on_propose: ProposeHandler,
    draft_inputs: Option<DraftInputs<'_>>,
    palette: Palette,
) -> AnyElement {
    let routes_label = if view.read_routes.is_empty() {
        "static snapshot".to_owned()
    } else {
        view.read_routes.join(", ")
    };
    let meta_bar = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_1p5()
        .child(Badge::new(view.data_class.as_str()).tone(Tone::Accent))
        .child(Badge::new(routes_label).tone(Tone::Neutral))
        .child(Badge::new("owner-mode proposals").tone(Tone::Warning));

    let body = render_a2ui_node(
        &view.root,
        view,
        view_state,
        &on_propose,
        draft_inputs,
        palette,
    );
    div()
        .id(SharedString::from(format!(
            "agent-live-view-{}",
            view.surface_id
        )))
        .debug_selector({
            let id = view.surface_id.clone();
            move || format!("agent-live-view-{id}")
        })
        .px_3()
        .py_2()
        .flex()
        .flex_col()
        .gap_2()
        .child(meta_bar)
        .child(body)
        .into_any_element()
}

fn render_a2ui_node(
    node_id: &str,
    view: &ValidatedA2uiView,
    view_state: &A2uiClientState,
    on_propose: &ProposeHandler,
    draft_inputs: Option<DraftInputs<'_>>,
    palette: Palette,
) -> AnyElement {
    let Some(node) = view.nodes.get(node_id) else {
        return div().into_any_element();
    };
    match node.kind {
        ComponentKind::Card => {
            let mut card = Card::new();
            if let Some(title) = &node.title {
                let mut header = CardHeader::new(title.clone());
                if let Some(sub) = &node.subtitle {
                    header = header.description(sub.clone());
                }
                card = card.header(header);
            }
            for child_id in &node.children {
                card = card.child(render_a2ui_node(
                    child_id,
                    view,
                    view_state,
                    on_propose,
                    draft_inputs,
                    palette,
                ));
            }
            card.into_any_element()
        }
        ComponentKind::MetricStrip => {
            let mut strip = MetricStrip::new();
            for child_id in &node.children {
                if let Some(tile_node) = view
                    .nodes
                    .get(child_id)
                    .filter(|n| n.kind == ComponentKind::MetricTile)
                {
                    let mut tile = MetricTile::new(
                        SharedString::from(format!(
                            "a2ui-tile-{}-{}",
                            view.surface_id, tile_node.id
                        )),
                        tile_node.label.clone().unwrap_or_default(),
                        tile_node.value.clone().unwrap_or_else(|| "—".into()),
                    );
                    if let Some(detail) = &tile_node.detail {
                        tile = tile.detail(detail.clone());
                    }
                    strip = strip.child(tile);
                }
            }
            strip.into_any_element()
        }
        ComponentKind::MetricTile => {
            let mut tile = MetricTile::new(
                SharedString::from(format!("a2ui-tile-{}-{}", view.surface_id, node.id)),
                node.label.clone().unwrap_or_default(),
                node.value.clone().unwrap_or_else(|| "—".into()),
            );
            if let Some(detail) = &node.detail {
                tile = tile.detail(detail.clone());
            }
            tile.into_any_element()
        }
        ComponentKind::DescriptionList => {
            let list = node
                .children
                .iter()
                .filter_map(|child_id| {
                    let item = view.nodes.get(child_id)?;
                    (item.kind == ComponentKind::DescriptionItem).then(|| {
                        (
                            item.label.clone().unwrap_or_default(),
                            item.value.clone().unwrap_or_default(),
                        )
                    })
                })
                .fold(DescriptionList::new(), |dl, (k, v)| dl.item(k, v));
            list.into_any_element()
        }
        ComponentKind::DescriptionItem => div()
            .flex()
            .justify_between()
            .gap_2()
            .text_xs()
            .child(
                div()
                    .text_color(rgb(palette.muted))
                    .child(node.label.clone().unwrap_or_default()),
            )
            .child(node.value.clone().unwrap_or_default())
            .into_any_element(),
        ComponentKind::DataTable => {
            let cols: Vec<TableColumn> = node
                .columns
                .iter()
                .map(|c| TableColumn::new(c.key.clone(), c.label.clone()).min_width(rems(6.)))
                .collect();
            let rows: Rc<Vec<TableRow>> = Rc::new(
                node.rows
                    .iter()
                    .enumerate()
                    .map(|(ix, row_map)| {
                        let cells: Vec<TableCell> = node
                            .columns
                            .iter()
                            .map(|col| {
                                TableCell::Text(
                                    row_map.get(&col.key).cloned().unwrap_or_default().into(),
                                )
                            })
                            .collect();
                        TableRow::new(format!("row-{ix}"), cells)
                    })
                    .collect(),
            );
            DataTable::new(
                SharedString::from(format!("a2ui-table-{}-{}", view.surface_id, node.id)),
                cols,
            )
            .rows(rows)
            .into_any_element()
        }
        ComponentKind::Badge => Badge::new(node.label.clone().unwrap_or_default())
            .tone(tone_of(node.tone.as_deref()))
            .into_any_element(),
        ComponentKind::Text => div()
            .text_sm()
            .child(node.value.clone().unwrap_or_default())
            .into_any_element(),
        ComponentKind::Form => {
            let mut form_box = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .rounded(px(6.))
                .border_1()
                .border_color(rgba(palette.border));
            if let Some(title) = &node.title {
                form_box = form_box.child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.clone()),
                );
            }
            for child_id in &node.children {
                form_box = form_box.child(render_a2ui_node(
                    child_id,
                    view,
                    view_state,
                    on_propose,
                    draft_inputs,
                    palette,
                ));
            }
            form_box.into_any_element()
        }
        ComponentKind::TextField => {
            let draft_key = node
                .field
                .as_deref()
                .and_then(|f| f.strip_prefix("/draft/"))
                .unwrap_or("");
            let current = view_state
                .draft_value(draft_key)
                .map(str::to_owned)
                .or_else(|| node.value.clone())
                .unwrap_or_default();
            let field_el = if let Some(input) = draft_inputs.and_then(|map| map.get(draft_key)) {
                Input::new(input).size(ControlSize::Sm).into_any_element()
            } else {
                div()
                    .px_2()
                    .py_1()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .text_xs()
                    .child(current)
                    .into_any_element()
            };
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(node.label.clone().unwrap_or_default()),
                )
                .child(field_el)
                .into_any_element()
        }
        ComponentKind::Button => {
            let surface_id = view.surface_id.clone();
            let button_node_id = node.id.clone();
            let handler = Arc::clone(on_propose);
            Button::new(
                SharedString::from(format!("a2ui-btn-{surface_id}-{button_node_id}")),
                node.label.clone().unwrap_or_else(|| "Propose".into()),
            )
            .size(ControlSize::Sm)
            .variant(ButtonVariant::Secondary)
            .on_click(move |_, _, cx| {
                handler(&surface_id, &button_node_id, cx);
            })
            .into_any_element()
        }
    }
}
