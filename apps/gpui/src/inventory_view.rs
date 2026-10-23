//! Household inventory, asset catalog, wishlist budget, and interior spatial layout.

use std::collections::BTreeSet;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, AnyElement, App, Context, Entity, FocusHandle,
    KeyBinding, Render, Subscription, Window, WindowAppearance,
};

use crate::data::{Remote, Repo};
use crate::shell::{self, ShellError};
use crate::theme::Palette;
use sjel_gpui_core::inventory::{
    filter_inventory, format_cents, format_dimensions, group_of, InteriorLayoutSummary,
    InventoryItem, InventoryRow, InventoryWishlist,
};

actions!(
    inventory,
    [
        /// Show the owned assets tab.
        ShowOwned,
        /// Show the wishlist budget tab.
        ShowWishlist,
        /// Show the interior layout summaries tab.
        ShowInterior,
        /// Reload inventory data from the shell.
        RefreshInventory,
        /// Focus the search input field.
        FocusSearch,
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("1", ShowOwned, Some("InventoryView")),
        KeyBinding::new("2", ShowWishlist, Some("InventoryView")),
        KeyBinding::new("3", ShowInterior, Some("InventoryView")),
        KeyBinding::new("/", FocusSearch, Some("InventoryView")),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum InventoryTab {
    #[default]
    Owned,
    Wishlist,
    Interior,
}

pub struct InventoryPage {
    repo: Repo,
    palette: Option<Palette>,
    pub tab: InventoryTab,
    pub rows: Remote<Vec<InventoryRow>>,
    pub wishlist: Remote<InventoryWishlist>,
    pub layouts: Remote<Vec<InteriorLayoutSummary>>,
    pub search_query: String,
    pub selected_category: Option<String>,
    search_input: Option<(Entity<InputState>, Subscription)>,
    focus: FocusHandle,
}

impl InventoryPage {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self {
            repo,
            palette: None,
            tab: InventoryTab::Owned,
            rows: Remote::default(),
            wishlist: Remote::default(),
            layouts: Remote::default(),
            search_query: String::new(),
            selected_category: None,
            search_input: None,
            focus: cx.focus_handle(),
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn activate(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn set_tab(&mut self, tab: InventoryTab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            cx.notify();
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };

        self.rows.loading = true;
        self.wishlist.loading = true;
        self.layouts.loading = true;
        cx.notify();

        // 1. Fetch Inventory Rows
        let s_i = shell.clone();
        let i_work = shell::run(async move { s_i.inventory_rows().await });
        cx.spawn(async move |this, cx| {
            let res = i_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.rows.settle(res);
                cx.notify();
            });
        })
        .detach();

        // 2. Fetch Wishlist
        let s_w = shell.clone();
        let w_work = shell::run(async move { s_w.inventory_wishlist().await });
        cx.spawn(async move |this, cx| {
            let res = w_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.wishlist.settle(res);
                cx.notify();
            });
        })
        .detach();

        // 3. Fetch Interior Layouts
        let s_l = shell.clone();
        let l_work = shell::run(async move { s_l.interior_layouts().await });
        cx.spawn(async move |this, cx| {
            let res = l_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.layouts.settle(res);
                cx.notify();
            });
        })
        .detach();
    }

    fn search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((input, _)) = &self.search_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search inventory…"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                view.search_query = input.read(cx).value().trim().to_lowercase();
                cx.notify();
            }
        });
        self.search_input = Some((input.clone(), subscription));
        input
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab != InventoryTab::Owned {
            self.set_tab(InventoryTab::Owned, cx);
        }
        let field = self.search_field(window, cx);
        field.update(cx, |input, cx| input.focus(window, cx));
    }

    fn render_header_bar(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let owned_count = self
            .rows
            .value
            .as_ref()
            .map(|r| {
                r.iter()
                    .filter(|row| row.state.as_deref() == Some("owned"))
                    .count()
            })
            .unwrap_or(0);
        let wishlist_count = self
            .wishlist
            .value
            .as_ref()
            .map(|w| w.items.len())
            .unwrap_or(0);
        let layouts_count = self.layouts.value.as_ref().map(|l| l.len()).unwrap_or(0);

        div()
            .flex()
            .items_center()
            .justify_between()
            .pb_3()
            .border_b_1()
            .border_color(rgba(palette.border << 8 | 0x22))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(self.render_tab_button(
                        "Owned Items",
                        InventoryTab::Owned,
                        owned_count,
                        palette,
                        cx,
                    ))
                    .child(self.render_tab_button(
                        "Wishlist",
                        InventoryTab::Wishlist,
                        wishlist_count,
                        palette,
                        cx,
                    ))
                    .child(self.render_tab_button(
                        "Interior Layouts",
                        InventoryTab::Interior,
                        layouts_count,
                        palette,
                        cx,
                    )),
            )
            .child(
                div().flex().items_center().gap_3().child(
                    Button::new("refresh-inventory")
                        .small()
                        .ghost()
                        .label("Refresh")
                        .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
                ),
            )
            .into_any_element()
    }

    fn render_tab_button(
        &self,
        label: &str,
        tab: InventoryTab,
        count: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_active = self.tab == tab;
        div()
            .px_3()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .text_sm()
            .font_weight(if is_active {
                gpui::FontWeight::SEMIBOLD
            } else {
                gpui::FontWeight::NORMAL
            })
            .bg(if is_active {
                rgba(palette.accent << 8 | 0x20)
            } else {
                rgba(0)
            })
            .text_color(if is_active {
                rgb(palette.accent)
            } else {
                rgb(palette.muted)
            })
            .hover(|s| s.bg(rgba(palette.border << 8 | 0x30)))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |page, _, _, cx| page.set_tab(tab, cx)),
            )
            .child(format!("{label} ({count})"))
            .into_any_element()
    }

    fn render_owned_view(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(all_rows) = &self.rows.value else {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Loading inventory…")
                .into_any_element();
        };

        let owned_rows: Vec<_> = all_rows
            .iter()
            .filter(|r| r.state.as_deref() == Some("owned"))
            .cloned()
            .collect();

        // Compute categories
        let mut categories = BTreeSet::new();
        let mut total_cents: i64 = 0;
        for r in &owned_rows {
            categories.insert(group_of(&r.item));
            if let Some(cents) = r.item.price_cents {
                total_cents += cents;
            }
        }

        let filtered = filter_inventory(
            &owned_rows,
            &self.search_query,
            self.selected_category.as_deref(),
            Some("owned"),
        );

        let search_field = self.search_field(window, cx);

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                // Stat Cards
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.render_stat_card(
                        "Owned Assets",
                        &owned_rows.len().to_string(),
                        palette,
                    ))
                    .child(self.render_stat_card(
                        "Estimated Value",
                        &format_cents(Some(total_cents)),
                        palette,
                    ))
                    .child(self.render_stat_card(
                        "Categories",
                        &categories.len().to_string(),
                        palette,
                    )),
            )
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
                                        cx.notify();
                                    }))
                            })),
                    ),
            )
            .child(if filtered.is_empty() {
                div()
                    .p_8()
                    .text_center()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("No inventory items match the current filters.")
                    .into_any_element()
            } else {
                let mut cards = Vec::new();
                for r in &filtered {
                    cards.push(self.render_item_card(&r.item, palette));
                }
                div()
                    .flex()
                    .flex_col()
                    .gap_2p5()
                    .children(cards)
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_stat_card(&self, label: &str, value: &str, palette: Palette) -> AnyElement {
        div()
            .flex_1()
            .p_3()
            .rounded_lg()
            .bg(rgba(palette.raised << 8 | 0x40))
            .border_1()
            .border_color(rgba(palette.border << 8 | 0x30))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(label.to_string()),
            )
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child(value.to_string()),
            )
            .into_any_element()
    }

    fn render_item_card(&self, item: &InventoryItem, palette: Palette) -> AnyElement {
        let dim = format_dimensions(item.width, item.depth, item.height);
        let price = format_cents(item.price_cents);

        div()
            .p_3p5()
            .rounded_lg()
            .bg(rgba(palette.raised << 8 | 0x30))
            .border_1()
            .border_color(rgba(palette.border << 8 | 0x25))
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(item.label.clone()),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_full()
                                    .text_xs()
                                    .bg(rgba(palette.accent << 8 | 0x20))
                                    .text_color(rgb(palette.accent))
                                    .child(group_of(item)),
                            )
                            .children(if !item.kind.is_empty() && item.kind != "piece" {
                                Some(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded_full()
                                        .text_xs()
                                        .bg(rgba(palette.border << 8 | 0x25))
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
                            .child(format!("Dim: {dim}"))
                            .children(item.size.as_ref().map(|s| format!("Size: {s}")))
                            .children(item.color.as_ref().map(|c| format!("Color: {c}")))
                            .children(item.notes.as_ref().map(|n| format!("Note: {n}"))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(rgb(palette.text))
                            .child(price),
                    )
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
            .into_any_element()
    }

    fn render_wishlist_view(&self, palette: Palette, _cx: &mut Context<Self>) -> AnyElement {
        let Some(wishlist) = &self.wishlist.value else {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Loading wishlist budget…")
                .into_any_element();
        };

        let low = format_cents(Some(wishlist.total_low_cents));
        let high = format_cents(Some(wishlist.total_high_cents));

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                // Budget banner
                div()
                    .p_4()
                    .rounded_lg()
                    .bg(rgba(palette.raised << 8 | 0x40))
                    .border_1()
                    .border_color(rgba(palette.border << 8 | 0x30))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child("Estimated Total Budget Range"),
                            )
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(palette.accent))
                                    .child(format!("{low} – {high}")),
                            )
                            .children(if wishlist.unpriced_count > 0 {
                                Some(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.amber))
                                        .child(format!(
                                            "{} unpriced items (budget is a floor)",
                                            wishlist.unpriced_count
                                        )),
                                )
                            } else {
                                None
                            }),
                    )
                    .children(wishlist.monthly_balance.as_ref().map(|b| {
                        div()
                            .flex()
                            .flex_col()
                            .items_end()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!(
                                        "Median Surplus: {}/mo",
                                        format_cents(Some(b.median_cents))
                                    )),
                            )
                            .children(wishlist.months_to_pay.map(|m| {
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.green))
                                    .child(format!("{:.1} months to fund", m))
                            }))
                    })),
            )
            .child(if wishlist.items.is_empty() {
                div()
                    .p_8()
                    .text_center()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("No open wishlist items.")
                    .into_any_element()
            } else {
                let mut cards = Vec::new();
                for item in &wishlist.items {
                    cards.push(self.render_wish_item(item, palette));
                }
                div()
                    .flex()
                    .flex_col()
                    .gap_2p5()
                    .children(cards)
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_wish_item(&self, item: &InventoryItem, palette: Palette) -> AnyElement {
        let cost_range = if let Some(price) = item.price_cents {
            format_cents(Some(price))
        } else if item.cost_min_cents.is_some() || item.cost_max_cents.is_some() {
            format!(
                "{} – {}",
                format_cents(item.cost_min_cents),
                format_cents(item.cost_max_cents)
            )
        } else {
            "unpriced".to_string()
        };

        let dim = format_dimensions(item.width, item.depth, item.height);

        div()
            .p_3p5()
            .rounded_lg()
            .bg(rgba(palette.raised << 8 | 0x30))
            .border_1()
            .border_color(rgba(palette.border << 8 | 0x25))
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(item.label.clone()),
                            )
                            .children(item.priority.as_ref().map(|pri| {
                                let color = match pri.as_str() {
                                    "required" | "decided" => palette.red,
                                    "recommended" => palette.amber,
                                    _ => palette.accent,
                                };
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_full()
                                    .text_xs()
                                    .bg(rgba(color << 8 | 0x20))
                                    .text_color(rgb(color))
                                    .child(pri.clone())
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("Target Dim: {dim}"))
                            .children(if !item.replaces.is_empty() {
                                Some(format!("Replaces: {}", item.replaces.join(", ")))
                            } else {
                                None
                            })
                            .children(item.notes.as_ref().map(|n| format!("Note: {n}"))),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child(cost_range),
            )
            .into_any_element()
    }

    fn render_interior_view(&self, palette: Palette, _cx: &mut Context<Self>) -> AnyElement {
        let Some(layouts) = &self.layouts.value else {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Loading interior layouts…")
                .into_any_element();
        };

        if layouts.is_empty() {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("No interior layouts found.")
                .into_any_element();
        }

        let mut cards = Vec::new();
        for l in layouts {
            cards.push(self.render_layout_card(l, palette));
        }

        div()
            .flex()
            .flex_col()
            .gap_3()
            .children(cards)
            .into_any_element()
    }

    fn render_layout_card(&self, l: &InteriorLayoutSummary, palette: Palette) -> AnyElement {
        let status_color = if l.pass {
            palette.green
        } else if l.hard > 0 {
            palette.red
        } else {
            palette.amber
        };

        let status_text = if l.pass {
            "Pass".to_string()
        } else if l.hard > 0 {
            format!("{} hard violations", l.hard)
        } else {
            format!("{} soft warnings", l.soft)
        };

        div()
            .p_4()
            .rounded_lg()
            .bg(rgba(palette.raised << 8 | 0x40))
            .border_1()
            .border_color(rgba(palette.border << 8 | 0x30))
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(l.name.clone()),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_full()
                                    .text_xs()
                                    .bg(rgba(status_color << 8 | 0x20))
                                    .text_color(rgb(status_color))
                                    .child(status_text),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("Layout ID: {}", l.id)),
                    ),
            )
            .children(l.occupied_m2.map(|area| {
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(format!("{:.1} m² occupied", area))
            }))
            .into_any_element()
    }
}

impl Render for InventoryPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self
            .palette
            .unwrap_or_else(|| Palette::for_appearance(WindowAppearance::Dark));

        div()
            .id("inventory-page")
            .key_context("InventoryView")
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|page, _: &ShowOwned, _, cx| page.set_tab(InventoryTab::Owned, cx)),
            )
            .on_action(
                cx.listener(|page, _: &ShowWishlist, _, cx| {
                    page.set_tab(InventoryTab::Wishlist, cx)
                }),
            )
            .on_action(
                cx.listener(|page, _: &ShowInterior, _, cx| {
                    page.set_tab(InventoryTab::Interior, cx)
                }),
            )
            .on_action(cx.listener(|page, _: &RefreshInventory, _, cx| page.refresh(cx)))
            .on_action(
                cx.listener(|page, _: &FocusSearch, window, cx| page.focus_search(window, cx)),
            )
            .flex()
            .flex_col()
            .size_full()
            .gap_4()
            .child(self.render_header_bar(palette, cx))
            .child(
                div()
                    .id("inventory-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(match self.tab {
                        InventoryTab::Owned => self.render_owned_view(palette, window, cx),
                        InventoryTab::Wishlist => self.render_wishlist_view(palette, cx),
                        InventoryTab::Interior => self.render_interior_view(palette, cx),
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn test_inventory_tab_switching(cx: &mut gpui::TestAppContext) {
        let page = cx.new(|cx| {
            let repo = Repo::new(Err(ShellError::NotSetUp));
            InventoryPage::new(repo, cx)
        });

        page.update(cx, |page, cx| {
            assert_eq!(page.tab, InventoryTab::Owned);
            page.set_tab(InventoryTab::Wishlist, cx);
            assert_eq!(page.tab, InventoryTab::Wishlist);
            page.set_tab(InventoryTab::Interior, cx);
            assert_eq!(page.tab, InventoryTab::Interior);
        });
    }
}
