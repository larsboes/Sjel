//! Owned inventory view, item details, editing, and state history.

use std::collections::BTreeSet;
use std::time::Instant;

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, App, Context, Entity, Subscription, Window,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable};
use sjel_gpui_core::inventory::{
    filter_inventory, format_cents, format_dimensions, group_of, InteriorItemImpact, InventoryItem,
    InventoryRow, InventoryState, InventoryStateHistoryEntry,
};
use sjel_gpui_core::shell::{self, ShellError};

use crate::data::Remote;
use crate::primitives::{
    resizable_table_header, stat_card, stat_strip, tabular_cell, virtual_list, SortOrder,
    SortableColumn,
};
use crate::theme::{tint, Density, Palette};

use super::{nonempty, InventoryPage};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum InventorySort {
    #[default]
    Name,
    Category,
    Dimensions,
    Price,
}

impl InventorySort {
    pub(crate) fn default_order(self) -> SortOrder {
        match self {
            Self::Price | Self::Dimensions => SortOrder::Descending,
            Self::Name | Self::Category => SortOrder::Ascending,
        }
    }
}

pub(crate) fn compare_inventory(
    left: &InventoryRow,
    right: &InventoryRow,
    sort: InventorySort,
    order: SortOrder,
) -> std::cmp::Ordering {
    let volume = |item: &InventoryItem| {
        i64::from(item.width.unwrap_or(0))
            .saturating_mul(i64::from(item.depth.unwrap_or(0)))
            .saturating_mul(i64::from(item.height.unwrap_or(0)))
    };
    let primary = match sort {
        InventorySort::Name => left
            .item
            .label
            .to_lowercase()
            .cmp(&right.item.label.to_lowercase()),
        InventorySort::Category => group_of(&left.item)
            .to_lowercase()
            .cmp(&group_of(&right.item).to_lowercase()),
        InventorySort::Dimensions => volume(&left.item).cmp(&volume(&right.item)),
        InventorySort::Price => left
            .item
            .price_cents
            .unwrap_or(-1)
            .cmp(&right.item.price_cents.unwrap_or(-1)),
    };
    let primary = if order.is_ascending() {
        primary
    } else {
        primary.reverse()
    };
    primary.then_with(|| {
        left.item
            .label
            .to_lowercase()
            .cmp(&right.item.label.to_lowercase())
    })
}

pub(crate) struct InventoryEditForm {
    pub(super) item: InventoryItem,
    pub(super) fields: Vec<(String, String, Entity<InputState>)>,
    pub(super) state_history: Remote<Vec<InventoryStateHistoryEntry>>,
    pub(super) impact: Remote<InteriorItemImpact>,
    pub(super) impact_busy: bool,
    pub(super) error: Option<String>,
    pub(super) saving: bool,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl InventoryEditForm {
    pub(super) fn value(&self, key: &str, cx: &App) -> String {
        self.fields
            .iter()
            .find(|(field, _, _)| field == key)
            .map(|(_, _, input)| input.read(cx).value().to_string())
            .unwrap_or_default()
    }
}

#[derive(Default)]
pub(crate) struct OwnedDerived {
    pub(super) key: Option<(Option<Instant>, u64)>,
    pub(super) owned_count: usize,
    pub(super) total_cents: i64,
    pub(super) categories: Vec<String>,
    pub(super) filtered: Vec<InventoryRow>,
}

impl InventoryPage {
    pub(super) fn set_owned_sort(&mut self, column: InventorySort, cx: &mut Context<Self>) {
        self.owned_sort = match self.owned_sort {
            (current, order) if current == column => (column, order.flip()),
            _ => (column, column.default_order()),
        };
        self.filter_rev = self.filter_rev.wrapping_add(1);
        cx.notify();
    }

    pub(super) fn sync_owned_derived(&mut self) {
        let key = (self.rows.loaded_at, self.filter_rev);
        if self.owned_derived.key == Some(key) {
            return;
        }
        let mut owned_count = 0;
        let mut total_cents = 0i64;
        let mut categories = BTreeSet::new();
        let mut filtered = Vec::new();
        if let Some(all_rows) = self.rows.value.as_ref() {
            let owned_rows: Vec<_> = all_rows
                .iter()
                .filter(|r| r.state.as_deref() == Some("owned"))
                .cloned()
                .collect();
            owned_count = owned_rows.len();
            for r in &owned_rows {
                categories.insert(group_of(&r.item));
                if let Some(cents) = r.item.price_cents {
                    total_cents += cents;
                }
            }
            filtered = filter_inventory(
                &owned_rows,
                &self.search_query,
                self.selected_category.as_deref(),
                Some("owned"),
            );
            let (sort_col, sort_order) = self.owned_sort;
            filtered.sort_by(|left, right| compare_inventory(left, right, sort_col, sort_order));
        }
        self.owned_list.reset(filtered.len());
        self.owned_derived = OwnedDerived {
            key: Some(key),
            owned_count,
            total_cents,
            categories: categories.into_iter().collect(),
            filtered,
        };
    }

    pub(super) fn open_item_edit(
        &mut self,
        item: InventoryItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.wish_form = None;
        let definitions = [
            ("label", "Name", item.label.clone(), "Item name".to_owned()),
            (
                "price_cents",
                "Price",
                item.price_cents
                    .map(|cents| format!("{:.2}", cents as f64 / 100.))
                    .unwrap_or_default(),
                "39,90".to_owned(),
            ),
            (
                "category",
                "Category",
                item.category.clone().unwrap_or_default(),
                "Category".to_owned(),
            ),
            (
                "priority",
                "Priority",
                item.priority.clone().unwrap_or_default(),
                "required, recommended…".to_owned(),
            ),
            (
                "width",
                "Width cm",
                item.width
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                "cm".to_owned(),
            ),
            (
                "depth",
                "Depth cm",
                item.depth
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                "cm".to_owned(),
            ),
            (
                "height",
                "Height cm",
                item.height
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                "cm".to_owned(),
            ),
            (
                "size",
                "Size",
                item.size.clone().unwrap_or_default(),
                "M".to_owned(),
            ),
            (
                "color",
                "Colour",
                item.color.clone().unwrap_or_default(),
                "Colour".to_owned(),
            ),
            (
                "link",
                "Product link",
                item.link.clone().unwrap_or_default(),
                "https://…".to_owned(),
            ),
            (
                "notes",
                "Note",
                item.notes.clone().unwrap_or_default(),
                "Optional note".to_owned(),
            ),
        ];
        let mut fields = Vec::new();
        let mut subscriptions = Vec::new();
        for (key, label, value, placeholder) in definitions {
            let input = cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder(placeholder);
                input.set_value(value, window, cx);
                input
            });
            subscriptions.push(
                cx.subscribe_in(&input, window, |page, _, event, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        page.save_item_edit(window, cx);
                    }
                }),
            );
            fields.push((key.to_owned(), label.to_owned(), input));
        }
        if let Some((_, _, input)) = fields.first() {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        let item_id = item.id.clone();
        self.edit_form = Some(InventoryEditForm {
            item,
            fields,
            state_history: Remote::new_loading(),
            impact: Remote::default(),
            impact_busy: false,
            error: None,
            saving: false,
            _subscriptions: subscriptions,
        });
        self.wish_notice = None;
        self.wish_error = None;
        self.load_edit_state_history(item_id, cx);
        cx.notify();
    }

    pub(super) fn load_edit_state_history(&mut self, id: String, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let expected = id.clone();
        let work = shell::run(async move { shell.inventory_state_history(&id).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                if let Some(form) = page.edit_form.as_mut() {
                    if form.item.id == expected {
                        form.state_history.settle(result);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn build_edit_patch(&self, cx: &App) -> Result<serde_json::Value, String> {
        let Some(form) = self.edit_form.as_ref() else {
            return Err("No active edit form.".to_owned());
        };
        let label = form.value("label", cx).trim().to_owned();
        if label.is_empty() {
            return Err("Give the entry a name.".to_owned());
        }
        let price_text = form.value("price_cents", cx);
        let price = if price_text.trim().is_empty() {
            serde_json::Value::Null
        } else {
            match sjel_gpui_core::capture::cents_from(&price_text) {
                Some(cents) if cents >= 0 => serde_json::json!(cents),
                _ => return Err("Enter a price like 39,90 or leave it blank.".to_owned()),
            }
        };
        let dimension = |key: &str| -> Result<serde_json::Value, String> {
            let text = form.value(key, cx);
            if text.trim().is_empty() {
                Ok(serde_json::Value::Null)
            } else {
                text.trim()
                    .parse::<i32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .map_or_else(
                        || {
                            Err("Dimensions must be positive whole centimeters, or blank."
                                .to_owned())
                        },
                        |value| Ok(serde_json::json!(value)),
                    )
            }
        };
        let (width, depth, height) = (
            dimension("width")?,
            dimension("depth")?,
            dimension("height")?,
        );
        let optional = |key: &str| {
            nonempty(form.value(key, cx))
                .map_or(serde_json::Value::Null, |value| serde_json::json!(value))
        };
        Ok(serde_json::json!({
            "label": label,
            "price_cents": price,
            "category": optional("category"),
            "priority": optional("priority"),
            "width": width,
            "depth": depth,
            "height": height,
            "size": optional("size"),
            "color": optional("color"),
            "link": optional("link"),
            "notes": optional("notes"),
        }))
    }

    pub(super) fn preview_item_impact(&mut self, cx: &mut Context<Self>) {
        let (Ok(shell), Some(form)) = (self.repo.shell(), self.edit_form.as_ref()) else {
            return;
        };
        if form.saving || form.impact_busy {
            return;
        }
        let id = form.item.id.clone();
        let patch = match self.build_edit_patch(cx) {
            Ok(patch) => patch,
            Err(error) => {
                if let Some(form) = self.edit_form.as_mut() {
                    form.error = Some(error);
                }
                cx.notify();
                return;
            }
        };
        if let Some(form) = self.edit_form.as_mut() {
            form.error = None;
            form.impact_busy = true;
            form.impact = Remote::new_loading();
        }
        cx.notify();

        let expected = id.clone();
        let work = shell::run(async move { shell.interior_item_impact(&id, patch).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                if let Some(form) = page.edit_form.as_mut() {
                    if form.item.id == expected {
                        form.impact_busy = false;
                        form.impact.settle(result);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn record_edit_state(&mut self, state: InventoryState, cx: &mut Context<Self>) {
        let (Ok(shell), Some(form)) = (self.repo.shell(), self.edit_form.as_ref()) else {
            return;
        };
        if form.saving || self.wish_busy.is_some() {
            return;
        }
        let id = form.item.id.clone();
        self.wish_busy = Some(id.clone());
        let expected = id.clone();
        let note = match state {
            InventoryState::Owned => "Marked owned in GPUI Inventory",
            InventoryState::Wanted => "Marked wanted in GPUI Inventory",
            InventoryState::Gone => "Marked gone in GPUI Inventory",
        };
        let work = shell::run(async move {
            shell.inventory_set_state(&id, state, Some(note)).await?;
            shell.inventory_state_history(&id).await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.wish_busy = None;
                match result {
                    Ok(history) => {
                        if let Some(form) = page.edit_form.as_mut() {
                            if form.item.id == expected {
                                form.state_history.settle(Ok(history));
                            }
                        }
                        page.load(cx);
                    }
                    Err(error) => {
                        if let Some(form) = page.edit_form.as_mut() {
                            form.error = Some(error.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn save_item_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Ok(shell), Some(form)) = (self.repo.shell(), self.edit_form.as_ref()) else {
            return;
        };
        if form.saving {
            return;
        }
        let id = form.item.id.clone();
        let revision = form.item.revision;
        let patch = match self.build_edit_patch(cx) {
            Ok(patch) => patch,
            Err(error) => {
                if let Some(form) = self.edit_form.as_mut() {
                    form.error = Some(error);
                }
                cx.notify();
                return;
            }
        };
        if let Some(form) = self.edit_form.as_mut() {
            form.saving = true;
            form.error = None;
        }
        self.wish_error = None;
        let work =
            shell::run(async move { shell.inventory_patch_item(&id, revision, patch).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                match result {
                    Ok(_) => {
                        page.edit_form = None;
                        page.wish_notice = Some("Saved inventory changes.".to_owned());
                        page.load(cx);
                    }
                    Err(error @ ShellError::Refused(409, _)) => {
                        page.edit_form = None;
                        page.wish_error = Some(format!(
                            "This entry changed elsewhere. Fresh values were loaded; reopen it before saving again. {error}"
                        ));
                        page.load(cx);
                    }
                    Err(error) => {
                        if let Some(form) = page.edit_form.as_mut() {
                            form.saving = false;
                            form.error = Some(error.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        let _ = window;
        cx.notify();
    }

    pub(super) fn render_owned_view(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.rows.value.is_none() {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Loading inventory…")
                .into_any_element();
        }
        self.sync_owned_derived();
        let owned_count = self.owned_derived.owned_count;
        let total_cents = self.owned_derived.total_cents;
        let categories = self.owned_derived.categories.clone();
        let is_empty = self.owned_derived.filtered.is_empty();

        let search_field = self.search_field(window, cx);

        let mut content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_4()
            .child(stat_strip(vec![
                stat_card(
                    "OWNED ASSETS",
                    owned_count.to_string(),
                    None,
                    palette.text,
                    palette,
                ),
                stat_card(
                    "ESTIMATED VALUE",
                    format_cents(Some(total_cents)),
                    None,
                    palette.text,
                    palette,
                ),
                stat_card(
                    "CATEGORIES",
                    categories.len().to_string(),
                    None,
                    palette.text,
                    palette,
                ),
            ]))
            .child(
                // Filter Bar
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(div().w(px(280.)).child(Input::new(&search_field).small()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new("cat-all")
                                    .small()
                                    .label("All")
                                    .when(self.selected_category.is_none(), |b| b.primary())
                                    .when(self.selected_category.is_some(), |b| b.ghost())
                                    .on_click(cx.listener(|page, _, _, cx| {
                                        page.selected_category = None;
                                        page.filter_rev = page.filter_rev.wrapping_add(1);
                                        cx.notify();
                                    })),
                            )
                            .children(categories.into_iter().map(|cat| {
                                let active = self.selected_category.as_deref() == Some(&cat);
                                let cat_name = cat.clone();
                                Button::new(format!("cat-{}", cat))
                                    .small()
                                    .label(cat.clone())
                                    .when(active, |b| b.primary())
                                    .when(!active, |b| b.ghost())
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        page.selected_category =
                                            if active { None } else { Some(cat_name.clone()) };
                                        page.filter_rev = page.filter_rev.wrapping_add(1);
                                        cx.notify();
                                    }))
                            })),
                    ),
            );
        if let Some(form) = self.render_wish_form(palette, cx) {
            content = content.child(form);
        }
        if let Some(form) = self.render_inventory_edit_form(palette, cx) {
            content = content.child(form);
        }
        if let Some(error) = &self.wish_error {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }
        let cat_w = self.col_width("category", 130.);
        let dim_w = self.col_width("dimensions", 160.);
        let price_w = self.col_width("price", 110.);
        let (sort_col, sort_order) = self.owned_sort;
        let sort_state = |column: InventorySort| (sort_col == column).then_some(sort_order);
        let sort_entity = cx.entity();
        let resize_entity = cx.entity();
        let header = resizable_table_header(
            "inventory-owned",
            vec![
                SortableColumn::flex("name", "ITEM").sorted(sort_state(InventorySort::Name)),
                SortableColumn::fixed("category", "CATEGORY", px(cat_w))
                    .sorted(sort_state(InventorySort::Category)),
                SortableColumn::fixed("dimensions", "DIMENSIONS", px(dim_w))
                    .sorted(sort_state(InventorySort::Dimensions)),
                SortableColumn::fixed("price", "VALUE", px(price_w))
                    .right_align()
                    .sorted(sort_state(InventorySort::Price)),
                SortableColumn::spacer(px(64.)),
            ],
            Density::Desktop,
            palette,
            move |col_id, _, cx| {
                let column = match col_id {
                    "name" => InventorySort::Name,
                    "category" => InventorySort::Category,
                    "dimensions" => InventorySort::Dimensions,
                    "price" => InventorySort::Price,
                    _ => return,
                };
                sort_entity.update(cx, |page, cx| page.set_owned_sort(column, cx));
            },
            move |col_id, _, cx| {
                resize_entity.update(cx, |page, cx| page.resize_column(col_id, cx));
            },
        );
        content
            .child(header)
            .child(if is_empty {
                div()
                    .p_8()
                    .text_center()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("No inventory items match the current filters.")
                    .into_any_element()
            } else {
                let entity = cx.entity();
                virtual_list(
                    "inventory-owned-list",
                    &self.owned_list,
                    move |ix, _window, cx| {
                        entity.update(cx, |page, cx| {
                            let Some(row) = page.owned_derived.filtered.get(ix) else {
                                return div().into_any_element();
                            };
                            let item = row.item.clone();
                            div()
                                .pb_1p5()
                                .child(page.render_item_card(&item, palette, cx))
                                .into_any_element()
                        })
                    },
                )
            })
            .into_any_element()
    }

    pub(super) fn render_item_card(
        &self,
        item: &InventoryItem,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let dim = format_dimensions(item.width, item.depth, item.height);
        let price = format_cents(item.price_cents);
        let cat_w = self.col_width("category", 130.);
        let dim_w = self.col_width("dimensions", 160.);
        let price_w = self.col_width("price", 110.);
        let item = item.clone();

        div()
            .px_3()
            .py_2p5()
            .rounded_lg()
            .bg(rgba(tint(palette.raised, 0x30)))
            .border_1()
            .border_color(rgba(tint(palette.border, 0x25)))
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(item.label.clone()),
                            )
                            .children(if !item.kind.is_empty() && item.kind != "piece" {
                                Some(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded_full()
                                        .text_xs()
                                        .bg(rgba(tint(palette.border, 0x25)))
                                        .text_color(rgb(palette.muted))
                                        .child(item.kind.clone()),
                                )
                            } else {
                                None
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .children(item.size.as_ref().map(|s| format!("Size: {s}")))
                            .children(item.color.as_ref().map(|c| format!("Color: {c}")))
                            .children(item.notes.as_ref().map(|n| format!("Note: {n}"))),
                    ),
            )
            .child(
                div().w(px(cat_w)).flex_none().flex().items_center().child(
                    div()
                        .max_w_full()
                        .truncate()
                        .px_2()
                        .py_0p5()
                        .rounded_full()
                        .text_xs()
                        .bg(rgba(tint(palette.accent, 0x20)))
                        .text_color(rgb(palette.accent))
                        .child(group_of(&item)),
                ),
            )
            .child(tabular_cell(dim, px(dim_w), palette.muted))
            .child(
                div()
                    .w(px(price_w))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_end()
                    .child(tabular_cell(price, px(price_w), palette.text))
                    .children(if let Some(count) = item.count {
                        if count > 1 {
                            Some(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!("qty: {count}")),
                            )
                        } else {
                            None
                        }
                    } else {
                        None
                    }),
            )
            .child(
                div().w(px(64.)).flex_none().flex().justify_end().child(
                    Button::new(format!("inventory-edit-{}", item.id))
                        .small()
                        .ghost()
                        .label("Edit")
                        .on_click(cx.listener(move |page, _, window, cx| {
                            page.open_item_edit(item.clone(), window, cx)
                        })),
                ),
            )
            .into_any_element()
    }

    pub(super) fn render_inventory_edit_form(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let form = self.edit_form.as_ref()?;
        let mut fields = div().flex().flex_col().gap_2().flex_none().w_full();
        for (key, label, input) in &form.fields {
            fields = fields.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(label.clone()),
                    )
                    .child(Input::new(input).small())
                    .id(format!("inventory-edit-{key}")),
            );
        }
        if let Some(error) = &form.error {
            fields = fields.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }
        let saving = form.saving;
        let impact_busy = form.impact_busy;
        let state_busy = self.wish_busy.is_some();
        let current_state = form
            .state_history
            .value
            .as_ref()
            .and_then(|rows| rows.last())
            .map(|row| row.state.as_str())
            .unwrap_or("");

        let state_buttons = div().flex().gap_1().children(
            [
                (InventoryState::Owned, "Owned"),
                (InventoryState::Wanted, "Wanted"),
                (InventoryState::Gone, "Gone"),
            ]
            .into_iter()
            .map(|(state, label)| {
                let active = current_state == state.as_str();
                Button::new(format!("inventory-edit-state-{}", state.as_str()))
                    .small()
                    .label(label)
                    .when(active, |b| b.primary())
                    .when(!active, |b| b.ghost())
                    .disabled(saving || state_busy)
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.record_edit_state(state, cx);
                    }))
            }),
        );

        let mut history_box = div()
            .p_3()
            .rounded_md()
            .bg(rgba(tint(palette.surface, 0x35)))
            .border_1()
            .border_color(rgba(tint(palette.border, 0x22)))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("STATE HISTORY"),
                    )
                    .child(state_buttons),
            );
        if let Some(entries) = form.state_history.value.as_ref() {
            if entries.is_empty() {
                history_box = history_box.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("No state transitions recorded."),
                );
            } else {
                for entry in entries {
                    let line = match entry.note.as_deref().filter(|n| !n.trim().is_empty()) {
                        Some(note) => format!("{} · {} · {note}", entry.state, entry.since),
                        None => format!("{} · {}", entry.state, entry.since),
                    };
                    history_box = history_box
                        .child(div().text_xs().text_color(rgb(palette.text)).child(line));
                }
            }
        } else if let Some(error) = form.state_history.error.as_ref() {
            history_box = history_box.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(format!("State history unavailable: {error}")),
            );
        } else if form.state_history.loading {
            history_box = history_box.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("Loading state history…"),
            );
        }

        let mut impact_box = None;
        if let Some(impact) = form.impact.value.as_ref() {
            let mut box_el = div()
                .p_3()
                .rounded_md()
                .bg(rgba(tint(palette.surface, 0x35)))
                .border_1()
                .border_color(rgba(tint(palette.border, 0x22)))
                .flex()
                .flex_col()
                .gap_1p5()
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "LAYOUT IMPACT · {} → {} passing of {} layouts",
                            impact.passing_before, impact.passing_after, impact.layouts
                        )),
                );
            if impact.changed.is_empty() {
                box_el = box_el.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("No layout verdicts change with these values."),
                );
            } else {
                for change in &impact.changed {
                    let before = if change.before.pass { "pass" } else { "fail" };
                    let after = if change.after.pass { "pass" } else { "fail" };
                    box_el = box_el.child(
                        div()
                            .text_xs()
                            .text_color(if change.after.pass {
                                rgb(palette.green)
                            } else {
                                rgb(palette.red)
                            })
                            .child(format!(
                                "{} · {before} → {after} (hard: {}, soft: {})",
                                change.layout,
                                change.after.hard.len(),
                                change.after.soft.len()
                            )),
                    );
                }
            }
            impact_box = Some(box_el);
        } else if let Some(error) = form.impact.error.as_ref() {
            impact_box = Some(
                div()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(rgba(tint(palette.border, 0x22)))
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(format!("Layout impact preview failed: {error}")),
            );
        }

        Some(
            div()
                .p_4()
                .rounded_lg()
                .bg(rgba(tint(palette.raised, 0x30)))
                .border_1()
                .border_color(rgba(tint(palette.border, 0x25)))
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(format!("Edit {}", form.item.label)),
                        )
                        .child(
                            Button::new("inventory-edit-cancel")
                                .small()
                                .ghost()
                                .label("Cancel")
                                .disabled(saving)
                                .on_click(cx.listener(|page, _, _, cx| {
                                    page.edit_form = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(fields)
                .child(history_box)
                .children(impact_box)
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("inventory-edit-impact")
                                .small()
                                .ghost()
                                .label(if impact_busy {
                                    "Checking layouts…"
                                } else {
                                    "Preview layout impact"
                                })
                                .disabled(saving || impact_busy)
                                .on_click(cx.listener(|page, _, _, cx| {
                                    page.preview_item_impact(cx);
                                })),
                        )
                        .child(
                            Button::new("inventory-edit-save")
                                .small()
                                .primary()
                                .label(if saving { "Saving…" } else { "Save changes" })
                                .disabled(saving)
                                .on_click(cx.listener(|page, _, window, cx| {
                                    page.save_item_edit(window, cx)
                                })),
                        ),
                )
                .into_any_element(),
        )
    }
}
