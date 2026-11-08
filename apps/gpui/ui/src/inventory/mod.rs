//! Household inventory, asset catalog, wishlist budget, and interior spatial layout.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, AnyElement, App, Context, Entity, FocusHandle,
    KeyBinding, Render, ScrollHandle, Subscription, Window,
};

use crate::data::{Remote, Repo};
use crate::interior::RoomPlanHistory;
use crate::primitives::{column_width, cycle_column_width, key_bar, sub_tabs, SortOrder};
use crate::theme::{tint, Palette, ThemeContext};
use sjel_gpui_core::interior::RoomPlanSource;
use sjel_gpui_core::inventory::{
    InteriorLayoutSummary, InventoryRow, InventoryWishlist, MailProposal, WishGroup,
};
use sjel_gpui_core::shell::{self, ShellError};

mod mail_review;
mod owned;
mod wish_compare;
mod wishlist;

use mail_review::MailReviewForm;
use owned::{InventoryEditForm, InventorySort, OwnedDerived};
use wishlist::{GroupForm, WishForm, WishlistDerived};

actions!(
    inventory,
    [
        /// Show the owned assets tab.
        ShowOwned,
        /// Show the wishlist budget tab.
        ShowWishlist,
        /// Show the interior layout summaries tab.
        ShowInterior,
        /// Show mail proposals awaiting review.
        ShowMailReview,
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
        KeyBinding::new("4", ShowMailReview, Some("InventoryView")),
        KeyBinding::new("/", FocusSearch, Some("InventoryView")),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum InventoryTab {
    #[default]
    Owned,
    Wishlist,
    Interior,
    MailReview,
}

pub struct InventoryPage {
    pub(super) repo: Repo,
    pub(super) palette: Option<Palette>,
    pub tab: InventoryTab,
    pub rows: Remote<Vec<InventoryRow>>,
    pub wishlist: Remote<InventoryWishlist>,
    pub wish_groups: Remote<Vec<WishGroup>>,
    /// Every group's needs and candidates, for the comparison view.
    pub comparisons: Remote<Vec<sjel_gpui_core::inventory::compare::GroupComparison>>,
    /// The spec the comparison's scatter is drawn against. None opens on the group's default.
    pub(super) compare_axis: Option<String>,
    pub layouts: Remote<Vec<InteriorLayoutSummary>>,
    pub(super) mail_proposals: Remote<Vec<MailProposal>>,
    pub(super) mail_form: Option<MailReviewForm>,
    pub(super) mail_busy: bool,
    pub(super) mail_message: Option<String>,
    pub(super) roomplan_history: Entity<RoomPlanHistory>,
    pub search_query: String,
    pub selected_category: Option<String>,
    pub(super) owned_sort: (InventorySort, SortOrder),
    pub(super) col_widths: std::collections::HashMap<&'static str, f32>,
    /// The wish group the wishlist is narrowed to, by id. None shows every open wish.
    pub selected_group: Option<String>,
    pub(super) filter_rev: u64,
    pub(super) owned_derived: OwnedDerived,
    pub(super) owned_list: gpui::ListState,
    pub(super) wishlist_derived: WishlistDerived,
    pub scroll: ScrollHandle,
    pub(super) search_input: Option<(Entity<InputState>, Subscription)>,
    pub(super) wish_form: Option<WishForm>,
    pub(super) group_form: Option<GroupForm>,
    /// The wish whose group picker is open, by id.
    pub(super) group_menu: Option<String>,
    /// The group a write is in flight for, so one press cannot make two.
    pub(super) group_busy: Option<String>,
    pub(super) confirm_remove_group: Option<String>,
    pub(super) edit_form: Option<InventoryEditForm>,
    pub(super) wish_busy: Option<String>,
    pub(super) wish_notice: Option<String>,
    pub(super) wish_error: Option<String>,
    pub(super) confirm_remove_wish: Option<String>,
    pub(super) focus: FocusHandle,
}

impl InventoryPage {
    /// Where an opened scan is cached and previewed. The shell supplies it: the Mac opens Quick
    /// Look, the phone hands it to its host.
    pub fn set_cache_preview_handler(
        &mut self,
        handler: fn(&str, &str, Vec<u8>),
        cx: &mut Context<Self>,
    ) {
        self.roomplan_history.update(cx, |history, _| {
            history.set_cache_preview_handler(Some(handler))
        });
    }

    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        let source = repo.shell().ok().map(RoomPlanSource::Shell);
        let roomplan_history = cx.new(|cx| RoomPlanHistory::new(source, cx));
        Self {
            repo,
            palette: None,
            tab: InventoryTab::Owned,
            rows: Remote::default(),
            wishlist: Remote::default(),
            wish_groups: Remote::default(),
            comparisons: Remote::default(),
            compare_axis: None,
            layouts: Remote::default(),
            mail_proposals: Remote::default(),
            mail_form: None,
            mail_busy: false,
            mail_message: None,
            roomplan_history,
            search_query: String::new(),
            selected_category: None,
            owned_sort: (InventorySort::Name, SortOrder::Ascending),
            col_widths: Default::default(),
            selected_group: None,
            filter_rev: 0,
            owned_derived: OwnedDerived::default(),
            owned_list: gpui::ListState::new(0, gpui::ListAlignment::Top, px(600.)),
            wishlist_derived: WishlistDerived::default(),
            scroll: ScrollHandle::new(),
            search_input: None,
            wish_form: None,
            group_form: None,
            group_menu: None,
            group_busy: None,
            confirm_remove_group: None,
            edit_form: None,
            wish_busy: None,
            wish_notice: None,
            wish_error: None,
            confirm_remove_wish: None,
            focus: cx.focus_handle(),
        }
    }

    pub fn scroll(&self) -> &ScrollHandle {
        &self.scroll
    }

    pub fn bench_scroll(&mut self, fraction: f32, cx: &mut Context<Self>) {
        if self.tab == InventoryTab::Owned {
            self.sync_owned_derived();
            let rows = self.owned_derived.filtered.len();
            let item_ix = (rows.saturating_sub(1) as f32 * fraction).round() as usize;
            self.owned_list.scroll_to(gpui::ListOffset {
                item_ix,
                offset_in_item: px(0.),
            });
        } else {
            let max = self.scroll.max_offset().y;
            self.scroll
                .set_offset(gpui_kit::point(px(0.), -max * fraction));
        }
        cx.notify();
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
        self.roomplan_history
            .update(cx, |history, cx| history.set_palette(palette, cx));
    }

    pub fn activate(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub(super) fn col_width(&self, id: &'static str, default: f32) -> f32 {
        column_width(&self.col_widths, id, default)
    }

    pub(super) fn resize_column(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let default = match id {
            "category" => Some(130.),
            "dimensions" => Some(160.),
            "price" => Some(110.),
            _ => None,
        };
        if let Some(default) = default {
            cycle_column_width(&mut self.col_widths, id, default);
            cx.notify();
        }
    }

    pub fn set_tab(&mut self, tab: InventoryTab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            if tab == InventoryTab::Interior {
                self.roomplan_history
                    .update(cx, |history, cx| history.refresh(cx));
            }
            cx.notify();
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.mail_proposals.settle(Err(error));
                cx.notify();
                return;
            }
        };
        self.load_mail(cx);

        self.rows.loading = true;
        self.wishlist.loading = true;
        self.wish_groups.loading = true;
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

        // 3. Fetch Wish Groups
        let s_g = shell.clone();
        let g_work = shell::run(async move { s_g.inventory_wish_groups().await });
        cx.spawn(async move |this, cx| {
            let res = g_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.wish_groups.settle(res);
                // A group that is no longer on the list would narrow the wishlist to nothing.
                let known = page.wish_groups.value.as_ref().is_some_and(|groups| {
                    page.selected_group
                        .as_deref()
                        .is_some_and(|id| groups.iter().any(|group| group.id == id))
                });
                if !known {
                    page.selected_group = None;
                    page.filter_rev = page.filter_rev.wrapping_add(1);
                }
                cx.notify();
            });
        })
        .detach();

        // 4. Fetch the wish comparison: needs, specs and price histories per group.
        let s_c = shell.clone();
        let c_work = shell::run(async move { s_c.inventory_wish_comparison().await });
        cx.spawn(async move |this, cx| {
            let res = c_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.comparisons.settle(res);
                cx.notify();
            });
        })
        .detach();

        // 5. Fetch Interior Layouts
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

        self.roomplan_history
            .update(cx, |history, cx| history.refresh(cx));
    }

    fn search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((input, _)) = &self.search_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search inventory…"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                view.search_query = input.read(cx).value().trim().to_lowercase();
                view.filter_rev = view.filter_rev.wrapping_add(1);
                cx.notify();
            }
        });
        self.search_input = Some((input.clone(), subscription));
        input
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The Interior tab draws no search field, so a focus there would land nowhere.
        if matches!(self.tab, InventoryTab::Interior | InventoryTab::MailReview) {
            self.set_tab(InventoryTab::Owned, cx);
        }
        let field = self.search_field(window, cx);
        field.update(cx, |input, cx| input.focus(window, cx));
    }

    fn render_header_bar(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        self.sync_owned_derived();
        let owned_count = self.owned_derived.owned_count;
        let wishlist_count = self
            .wishlist
            .value
            .as_ref()
            .map(|w| w.items.len())
            .unwrap_or(0);
        let layouts_count = self.layouts.value.as_ref().map(|l| l.len()).unwrap_or(0);

        let mail_count = self.mail_proposals.value.as_ref().map_or(0, |items| {
            items.iter().filter(|p| p.decision.is_none()).count()
        });
        let entity = cx.entity();

        div()
            .flex()
            .items_center()
            .justify_between()
            .pb_3()
            .border_b_1()
            .border_color(rgba(tint(palette.border, 0x22)))
            .child(sub_tabs(
                "inventory",
                vec![
                    (
                        "Owned Items",
                        Some(owned_count),
                        self.tab == InventoryTab::Owned,
                    ),
                    (
                        "Wishlist",
                        Some(wishlist_count),
                        self.tab == InventoryTab::Wishlist,
                    ),
                    (
                        "Interior Layouts",
                        Some(layouts_count),
                        self.tab == InventoryTab::Interior,
                    ),
                    (
                        "Mail review",
                        Some(mail_count),
                        self.tab == InventoryTab::MailReview,
                    ),
                ],
                palette,
                move |idx, _window, cx| {
                    entity.update(cx, |page, cx| {
                        let next = match idx {
                            0 => InventoryTab::Owned,
                            1 => InventoryTab::Wishlist,
                            2 => InventoryTab::Interior,
                            _ => InventoryTab::MailReview,
                        };
                        page.set_tab(next, cx);
                    });
                },
            ))
            .child(
                div().flex().items_center().gap_3().child(
                    Button::new("inventory-header-new-entry")
                        .small()
                        .primary()
                        .label(if self.wish_form.is_some() {
                            "Cancel"
                        } else {
                            "+ New entry"
                        })
                        .disabled(self.wish_busy.as_deref() == Some("new"))
                        .on_click(
                            cx.listener(|page, _, window, cx| page.open_wish_form(window, cx)),
                        ),
                ),
            )
            .into_any_element()
    }

    fn render_interior_view(&self, palette: Palette) -> AnyElement {
        let layouts = match self.layouts.value.as_ref() {
            None if self.layouts.loading => div()
                .p_4()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Loading interior layouts…")
                .into_any_element(),
            None => div()
                .p_4()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Interior layouts are unavailable.")
                .into_any_element(),
            Some(layouts) if layouts.is_empty() => div()
                .p_4()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("No interior layouts found yet.")
                .into_any_element(),
            Some(layouts) => {
                let cards = layouts
                    .iter()
                    .map(|layout| self.render_layout_card(layout, palette))
                    .collect::<Vec<_>>();
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .children(cards)
                    .into_any_element()
            }
        };

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.roomplan_history.clone())
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Layouts"),
            )
            .child(layouts)
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
            .bg(rgba(tint(palette.raised, 0x40)))
            .border_1()
            .border_color(rgba(tint(palette.border, 0x30)))
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
                                    .bg(rgba(tint(status_color, 0x20)))
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
            .unwrap_or_else(|| ThemeContext::of(window, cx).palette);
        self.roomplan_history
            .update(cx, |history, cx| history.set_palette(palette, cx));

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
            .on_action(cx.listener(|page, _: &ShowMailReview, _, cx| {
                page.set_tab(InventoryTab::MailReview, cx)
            }))
            .on_action(cx.listener(|page, _: &RefreshInventory, _, cx| page.refresh(cx)))
            .on_action(
                cx.listener(|page, _: &FocusSearch, window, cx| page.focus_search(window, cx)),
            )
            .flex()
            .flex_col()
            .size_full()
            .gap_4()
            .child(self.render_header_bar(palette, cx))
            .child(if self.tab == InventoryTab::Owned {
                div()
                    .id("inventory-content")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_owned_view(palette, window, cx))
                    .into_any_element()
            } else {
                div()
                    .id("inventory-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .vertical_scrollbar(&self.scroll)
                    .child(match self.tab {
                        InventoryTab::Owned => unreachable!(),
                        InventoryTab::Wishlist => self.render_wishlist_view(palette, window, cx),
                        InventoryTab::Interior => self.render_interior_view(palette),
                        InventoryTab::MailReview => self.render_mail_review(palette, cx),
                    })
                    .into_any_element()
            })
            .child(key_bar(
                &[("1 / 2 / 3 / 4", "switch tab"), ("/", "search")],
                palette,
            ))
    }
}

pub(super) fn nonempty(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
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
            page.set_tab(InventoryTab::MailReview, cx);
            assert_eq!(page.tab, InventoryTab::MailReview);
            assert_eq!(page.owned_sort, (InventorySort::Name, SortOrder::Ascending));
            page.set_owned_sort(InventorySort::Price, cx);
            assert_eq!(
                page.owned_sort,
                (InventorySort::Price, SortOrder::Descending)
            );
            page.set_owned_sort(InventorySort::Price, cx);
            assert_eq!(
                page.owned_sort,
                (InventorySort::Price, SortOrder::Ascending)
            );
            page.resize_column("price", cx);
            assert_eq!(page.col_width("price", 110.), 149.);
        });
    }
}
