//! Wishlist items, budgeting, groups, and candidate comparisons.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, App, Context, Entity, Subscription, Window,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable};
use sjel_gpui_core::inventory::{
    budget_of, counted, format_cents, format_dimensions, months_to_pay, wishlist_view, Budget,
    InventoryItem, InventoryState, NewInventoryItem, WishGroup,
};
use sjel_gpui_core::shell::{self, ShellError};

use crate::theme::{tint, Palette};

use super::{nonempty, InventoryPage, InventoryTab};

pub(crate) struct WishForm {
    pub(super) fields: Vec<(String, Entity<InputState>)>,
    pub(super) kind: String,
    pub(super) state: InventoryState,
    pub(super) error: Option<String>,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl WishForm {
    pub(super) fn value(&self, key: &str, cx: &App) -> String {
        self.fields
            .iter()
            .find(|(field, _)| field == key)
            .map(|(_, input)| input.read(cx).value().to_string())
            .unwrap_or_default()
    }
}

/// The two fields a wish group is made of. Its members are chosen on the wishes themselves.
pub(crate) struct GroupForm {
    pub(super) fields: Vec<(String, String, Entity<InputState>)>,
    pub(super) error: Option<String>,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl GroupForm {
    pub(super) fn value(&self, key: &str, cx: &App) -> String {
        self.fields
            .iter()
            .find(|(field, _, _)| field == key)
            .map(|(_, _, input)| input.read(cx).value().to_string())
            .unwrap_or_default()
    }
}

#[derive(Default)]
pub(crate) struct WishlistDerived {
    pub(super) key: Option<(Option<Instant>, Option<Instant>, u64)>,
    pub(super) shown: Vec<InventoryItem>,
    pub(super) budget: Budget,
    pub(super) whole: Budget,
}

impl InventoryPage {
    pub(super) fn sync_wishlist_derived(&mut self) {
        let key = (
            self.wishlist.loaded_at,
            self.wish_groups.loaded_at,
            self.filter_rev,
        );
        if self.wishlist_derived.key == Some(key) {
            return;
        }
        let mut shown = Vec::new();
        let mut budget = Budget::default();
        let mut whole = Budget::default();
        if let Some(wishlist) = self.wishlist.value.as_ref() {
            let groups = self.wish_groups.value.as_deref().unwrap_or_default();
            let group = self
                .selected_group
                .as_deref()
                .and_then(|id| groups.iter().find(|g| g.id == id));
            let refs = wishlist_view(&wishlist.items, group, &self.search_query);
            budget = budget_of(counted(&refs, groups));
            let every: Vec<&InventoryItem> = wishlist.items.iter().collect();
            whole = budget_of(counted(&every, groups));
            shown = refs.into_iter().cloned().collect();
        }
        self.wishlist_derived = WishlistDerived {
            key: Some(key),
            shown,
            budget,
            whole,
        };
    }

    pub(super) fn open_group_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.group_form.is_some() {
            self.group_form = None;
            cx.notify();
            return;
        }
        let definitions = [
            ("title", "Title", "Bad", "What the group is called"),
            (
                "constraint",
                "Constraint",
                "",
                "What bounds it, e.g. one month's surplus",
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
                        page.create_group(window, cx);
                    }
                }),
            );
            fields.push((key.to_owned(), label.to_owned(), input));
        }
        if let Some((_, _, input)) = fields.first() {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        self.group_form = Some(GroupForm {
            fields,
            error: None,
            _subscriptions: subscriptions,
        });
        self.wish_notice = None;
        self.wish_error = None;
        cx.notify();
    }

    pub(super) fn create_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Ok(shell), Some(form)) = (self.repo.shell(), self.group_form.as_ref()) else {
            return;
        };
        if self.group_busy.is_some() {
            return;
        }
        let title = form.value("title", cx).trim().to_owned();
        if title.is_empty() {
            if let Some(form) = self.group_form.as_mut() {
                form.error = Some("Give the group a name.".to_owned());
            }
            cx.notify();
            return;
        }
        let constraint_text = form.value("constraint", cx).trim().to_owned();
        let id = sjel_gpui_core::capture::slug(&title);
        let write_id = id.clone();
        let write_title = title.clone();
        let write_constraint = constraint_text.clone();
        self.group_busy = Some(id.clone());
        self.wish_error = None;
        let work = shell::run(async move {
            shell
                .inventory_save_wish_group(&write_id, &write_title, &write_constraint)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.group_busy = None;
                match result {
                    Ok(()) => {
                        page.group_form = None;
                        page.selected_group = Some(id.clone());
                        page.filter_rev = page.filter_rev.wrapping_add(1);
                        page.wish_notice = Some(format!("Group \"{title}\" created."));
                        page.load(cx);
                    }
                    Err(error) => {
                        if let Some(form) = page.group_form.as_mut() {
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

    /// Adds a wish to a group, or takes it out again. The capability replaces the whole membership,
    /// so the list is rebuilt from the group as it was last read.
    pub(super) fn toggle_group_member(
        &mut self,
        group: WishGroup,
        item_id: String,
        cx: &mut Context<Self>,
    ) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        if self.group_busy.is_some() {
            return;
        }
        let joining = !group.contains(&item_id);
        let mut ids = group.item_ids();
        if joining {
            ids.push(item_id.clone());
        } else {
            ids.retain(|id| id != &item_id);
        }
        self.group_busy = Some(group.id.clone());
        self.wish_error = None;
        let group_id = group.id.clone();
        let group_title = group.title.clone();
        let work =
            shell::run(async move { shell.inventory_set_wish_group_items(&group_id, &ids).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.group_busy = None;
                match result {
                    Ok(()) => {
                        page.wish_notice = Some(if joining {
                            format!("Added to \"{group_title}\".")
                        } else {
                            format!("Removed from \"{group_title}\".")
                        });
                        page.load(cx);
                    }
                    Err(error) => page.wish_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Takes a group off the list. Its wishes stay in the wishlist.
    pub(super) fn remove_group(&mut self, id: String, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        if self.group_busy.is_some() {
            return;
        }
        self.group_busy = Some(id.clone());
        self.confirm_remove_group = None;
        self.wish_error = None;
        let write_id = id.clone();
        let work = shell::run(async move { shell.inventory_delete_wish_group(&write_id).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.group_busy = None;
                match result {
                    Ok(()) => {
                        if page.selected_group.as_deref() == Some(id.as_str()) {
                            page.selected_group = None;
                            page.filter_rev = page.filter_rev.wrapping_add(1);
                        }
                        page.wish_notice =
                            Some("Group removed. Its wishes stay on the list.".to_owned());
                        page.load(cx);
                    }
                    Err(error) => page.wish_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn open_wish_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.wish_form.is_some() {
            self.wish_form = None;
            cx.notify();
            return;
        }
        self.edit_form = None;
        let definitions = [
            ("label", "What do you want?", "Item or need"),
            ("price", "Estimated price", "39,90"),
            ("category", "Category", "Furniture, clothes…"),
            ("link", "Product link", "https://…"),
        ];
        let mut fields = Vec::new();
        let mut subscriptions = Vec::new();
        for (key, label, placeholder) in definitions {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            subscriptions.push(cx.subscribe_in(&input, window, |page, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    page.create_wish(cx);
                }
            }));
            fields.push((format!("{key} · {label}"), input));
        }
        if let Some((_, input)) = fields.first() {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        self.wish_form = Some(WishForm {
            fields,
            kind: "piece".to_owned(),
            state: if self.tab == InventoryTab::Wishlist {
                InventoryState::Wanted
            } else {
                InventoryState::Owned
            },
            error: None,
            _subscriptions: subscriptions,
        });
        self.wish_notice = None;
        self.wish_error = None;
        cx.notify();
    }

    pub(super) fn create_wish(&mut self, cx: &mut Context<Self>) {
        let (Some(shell), Some(form)) = (self.repo.shell().ok(), self.wish_form.as_ref()) else {
            return;
        };
        if self.wish_busy.is_some() {
            return;
        }
        let label = form
            .value("label · What do you want?", cx)
            .trim()
            .to_owned();
        if label.is_empty() {
            if let Some(form) = self.wish_form.as_mut() {
                form.error = Some("Give the wish a name.".to_owned());
            }
            cx.notify();
            return;
        }
        let price_text = form.value("price · Estimated price", cx);
        let price_cents = if price_text.trim().is_empty() {
            None
        } else {
            match sjel_gpui_core::capture::cents_from(&price_text) {
                Some(cents) if cents >= 0 => Some(cents),
                _ => {
                    if let Some(form) = self.wish_form.as_mut() {
                        form.error = Some("Enter a price like 39,90.".to_owned());
                    }
                    cx.notify();
                    return;
                }
            }
        };
        let category = nonempty(form.value("category · Category", cx));
        let link = nonempty(form.value("link · Product link", cx));
        let slug = sjel_gpui_core::capture::slug(&label);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let wish = NewInventoryItem {
            id: format!("{slug}-{timestamp}"),
            kind: form.kind.clone(),
            label,
            state: form.state,
            note: "Created in GPUI Inventory".to_owned(),
            price_cents,
            category,
            link,
        };
        self.wish_busy = Some("new".to_owned());
        self.wish_notice = None;
        self.wish_error = None;
        if let Some(form) = self.wish_form.as_mut() {
            form.error = None;
        }
        let created_state = wish.state;
        let work = shell::run(async move { shell.inventory_create_item(&wish).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.wish_busy = None;
                match result {
                    Ok(()) => {
                        page.wish_form = None;
                        page.wish_notice = Some(match created_state {
                            InventoryState::Owned => "Added to inventory.".to_owned(),
                            InventoryState::Wanted => "Added to the wishlist.".to_owned(),
                            InventoryState::Gone => "Added as a gone item.".to_owned(),
                        });
                        page.load(cx);
                    }
                    Err(error) => {
                        if let Some(form) = page.wish_form.as_mut() {
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

    pub(super) fn set_wish_state(
        &mut self,
        id: String,
        state: InventoryState,
        cx: &mut Context<Self>,
    ) {
        if self.wish_busy.is_some() {
            return;
        }
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        self.wish_busy = Some(id.clone());
        self.wish_notice = None;
        self.wish_error = None;
        self.confirm_remove_wish = None;
        let write_id = id.clone();
        let note = match state {
            InventoryState::Owned => "Marked bought in GPUI Wishlist",
            InventoryState::Gone => "Removed from GPUI Wishlist",
            InventoryState::Wanted => "Restored from GPUI Wishlist",
        };
        let work = shell::run(async move {
            shell
                .inventory_set_state(&write_id, state, Some(note))
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.wish_busy = None;
                match result {
                    Ok(_) => {
                        page.wish_notice = Some(match state {
                            InventoryState::Owned => "Marked as bought.".to_owned(),
                            InventoryState::Gone => "Removed from the open wishlist.".to_owned(),
                            InventoryState::Wanted => "Restored to the wishlist.".to_owned(),
                        });
                        page.load(cx);
                    }
                    Err(error) => page.wish_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_wish_form(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let form = self.wish_form.as_ref()?;
        let mut fields = div().flex().flex_col().gap_2().flex_none().w_full();
        for (key, input) in &form.fields {
            let (field, label) = key.split_once(" · ").unwrap_or((key, key));
            fields = fields.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(label.to_owned()),
                    )
                    .child(Input::new(input).small())
                    .id(format!("wishlist-new-{field}")),
            );
        }
        let kind = form.kind.clone();
        let state = form.state;
        let saving = self.wish_busy.as_deref() == Some("new");
        let mut card = div()
            .p_4()
            .rounded(px(12.))
            .bg(rgba(tint(palette.raised, 0x30)))
            .border_1()
            .border_color(rgba(tint(palette.border, 0x25)))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(if self.tab == InventoryTab::Wishlist {
                        "New wishlist entry"
                    } else {
                        "New inventory entry"
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("wishlist-kind-piece")
                            .small()
                            .label("Product")
                            .when(kind == "piece", |button| button.primary())
                            .when(kind != "piece", |button| button.ghost())
                            .on_click(cx.listener(|page, _, _, cx| {
                                if let Some(form) = page.wish_form.as_mut() {
                                    form.kind = "piece".to_owned();
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("wishlist-kind-slot")
                            .small()
                            .label("Flat need")
                            .when(kind == "slot", |button| button.primary())
                            .when(kind != "slot", |button| button.ghost())
                            .on_click(cx.listener(|page, _, _, cx| {
                                if let Some(form) = page.wish_form.as_mut() {
                                    form.kind = "slot".to_owned();
                                }
                                cx.notify();
                            })),
                    )
                    .when(self.tab == InventoryTab::Owned, |row| {
                        row.child(
                            Button::new("inventory-new-owned")
                                .small()
                                .label("Owned")
                                .when(state == InventoryState::Owned, |button| button.primary())
                                .when(state != InventoryState::Owned, |button| button.ghost())
                                .on_click(cx.listener(|page, _, _, cx| {
                                    if let Some(form) = page.wish_form.as_mut() {
                                        form.state = InventoryState::Owned;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("inventory-new-wanted")
                                .small()
                                .label("Wanted")
                                .when(state == InventoryState::Wanted, |button| button.primary())
                                .when(state != InventoryState::Wanted, |button| button.ghost())
                                .on_click(cx.listener(|page, _, _, cx| {
                                    if let Some(form) = page.wish_form.as_mut() {
                                        form.state = InventoryState::Wanted;
                                    }
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(fields);
        if let Some(error) = &form.error {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }
        Some(
            card.child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("wishlist-create-cancel")
                            .small()
                            .ghost()
                            .label("Cancel")
                            .disabled(saving)
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.wish_form = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("wishlist-create-save")
                            .small()
                            .primary()
                            .label(if saving {
                                "Adding…"
                            } else {
                                "Add to wishlist"
                            })
                            .disabled(saving)
                            .on_click(cx.listener(|page, _, _, cx| page.create_wish(cx))),
                    ),
            )
            .into_any_element(),
        )
    }

    pub(super) fn render_wishlist_view(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.wishlist.value.is_none() {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Loading wishlist budget…")
                .into_any_element();
        }
        self.sync_wishlist_derived();
        let (total_items, monthly_balance) = self
            .wishlist
            .value
            .as_ref()
            .map_or((0, None), |w| (w.items.len(), w.monthly_balance.clone()));

        let groups = self.wish_groups.value.clone().unwrap_or_default();
        // A group that has just been deleted would narrow the list to nothing, so the selection is
        // resolved against the groups that exist.
        let group = self
            .selected_group
            .as_deref()
            .and_then(|id| groups.iter().find(|group| group.id == id))
            .cloned();
        let shown = self.wishlist_derived.shown.clone();
        let filtered = group.is_some() || !self.search_query.trim().is_empty();

        // The banner describes what the list shows. When a group or a search narrows it, the range
        // of the whole wishlist stays visible underneath instead of disappearing.
        // A group of alternatives is one purchase: only its cheapest member is in a total.
        let budget = self.wishlist_derived.budget;
        let whole = self.wishlist_derived.whole;
        let range = format!(
            "{} – {}",
            format_cents(Some(budget.low_cents)),
            format_cents(Some(budget.high_cents))
        );
        let label = match &group {
            Some(group) if group.alternatives => format!(
                "{} · {} open · pick one, the cheapest counts",
                group.title,
                shown.len()
            ),
            Some(group) => format!("{} · {} open", group.title, shown.len()),
            None if filtered => format!("Matching wishes · {} of {}", shown.len(), total_items),
            None => "Estimated Total Budget Range".to_owned(),
        };
        let whole_line = filtered.then(|| {
            format!(
                "All {} open: {} – {}",
                total_items,
                format_cents(Some(whole.low_cents)),
                format_cents(Some(whole.high_cents))
            )
        });
        let months = monthly_balance
            .as_ref()
            .and_then(|balance| months_to_pay(budget.low_cents, balance));
        let search_field = self.search_field(window, cx);
        let group_busy = self.group_busy.is_some();

        let mut content = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Wishlist"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("wishlist-new-group")
                                    .small()
                                    .ghost()
                                    .label(if self.group_form.is_some() {
                                        "Cancel"
                                    } else {
                                        "+ New group"
                                    })
                                    .disabled(group_busy)
                                    .on_click(cx.listener(|page, _, window, cx| {
                                        page.open_group_form(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("wishlist-new")
                                    .small()
                                    .primary()
                                    .label(if self.wish_form.is_some() {
                                        "Cancel"
                                    } else {
                                        "+ Add wish"
                                    })
                                    .disabled(self.wish_busy.as_deref() == Some("new"))
                                    .on_click(cx.listener(|page, _, window, cx| {
                                        page.open_wish_form(window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
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
                                Button::new("group-all")
                                    .small()
                                    .label(format!("All ({total_items})"))
                                    .when(group.is_none(), |b| b.primary())
                                    .when(group.is_some(), |b| b.ghost())
                                    .on_click(cx.listener(|page, _, _, cx| {
                                        page.selected_group = None;
                                        page.filter_rev = page.filter_rev.wrapping_add(1);
                                        cx.notify();
                                    })),
                            )
                            .children(groups.iter().map(|candidate| {
                                let active =
                                    group.as_ref().is_some_and(|group| group.id == candidate.id);
                                let id = candidate.id.clone();
                                Button::new(format!("group-{}", candidate.id))
                                    .small()
                                    .label(format!(
                                        "{} ({})",
                                        candidate.title, candidate.items_count
                                    ))
                                    .when(active, |b| b.primary())
                                    .when(!active, |b| b.ghost())
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        page.selected_group =
                                            if active { None } else { Some(id.clone()) };
                                        page.filter_rev = page.filter_rev.wrapping_add(1);
                                        cx.notify();
                                    }))
                            })),
                    ),
            );
        if let Some(form) = self.render_group_form(palette, cx) {
            content = content.child(form);
        }
        if let Some(group) = &group {
            let mut row = div().flex().items_center().justify_between().gap_3().child(
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(if group.constraint_text.trim().is_empty() {
                        "No constraint set.".to_owned()
                    } else {
                        group.constraint_text.clone()
                    }),
            );
            let toggle = group.clone();
            row = row.child(
                Button::new(format!("group-alternatives-{}", group.id))
                    .small()
                    .ghost()
                    .label(if group.alternatives {
                        "Pick one: on"
                    } else {
                        "Pick one: off"
                    })
                    .disabled(group_busy)
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.set_alternatives(toggle.clone(), cx);
                    })),
            );
            let id = group.id.clone();
            let confirming = self.confirm_remove_group.as_deref() == Some(id.as_str());
            row = row.child(
                Button::new(format!("group-remove-{id}"))
                    .small()
                    .ghost()
                    .label(if confirming {
                        "Confirm removal"
                    } else {
                        "Remove group"
                    })
                    .disabled(group_busy)
                    .on_click(cx.listener(move |page, _, _, cx| {
                        if page.confirm_remove_group.as_deref() == Some(id.as_str()) {
                            page.remove_group(id.clone(), cx);
                        } else {
                            page.confirm_remove_group = Some(id.clone());
                            cx.notify();
                        }
                    })),
            );
            content = content.child(row);
        }
        if let Some(form) = self.render_wish_form(palette, cx) {
            content = content.child(form);
        }
        if let Some(notice) = &self.wish_notice {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(notice.clone()),
            );
        }
        if let Some(error) = &self.wish_error {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }
        let comparison = group
            .as_ref()
            .and_then(|group| self.render_comparison(&group.id, palette, cx));
        content
            .when_some(
                self.render_inventory_edit_form(palette, cx),
                |content, form| content.child(form),
            )
            .child(
                // Budget banner
                div()
                    .p_4()
                    .rounded(px(12.))
                    .bg(rgba(tint(palette.raised, 0x30)))
                    .border_1()
                    .border_color(rgba(tint(palette.border, 0x25)))
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
                                    .child(label),
                            )
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(palette.accent))
                                    .child(range),
                            )
                            .children((budget.unpriced > 0).then(|| {
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.amber))
                                    .child(format!(
                                        "{} unpriced items (budget is a floor)",
                                        budget.unpriced
                                    ))
                            }))
                            .children(whole_line.map(|line| {
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(line)
                            })),
                    )
                    .children(monthly_balance.as_ref().map(|b| {
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
                            .children(months.map(|m| {
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.green))
                                    .child(format!("{:.1} months to fund", m))
                            }))
                    })),
            )
            .children(comparison)
            .child(if shown.is_empty() {
                div()
                    .p_8()
                    .text_center()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(match (&group, filtered) {
                        (Some(_), _) if total_items == 0 => {
                            "No open wishlist items.".to_owned()
                        }
                        (Some(group), _) => format!(
                            "Nothing from the open wishlist is in \"{}\" yet. Pick Group on a wish to add it.",
                            group.title
                        ),
                        (None, true) => "No wish matches the search.".to_owned(),
                        (None, false) => "No open wishlist items.".to_owned(),
                    })
                    .into_any_element()
            } else {
                let mut cards = Vec::new();
                for item in &shown {
                    cards.push(self.render_wish_item(item, &groups, palette, cx));
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

    pub(super) fn render_group_form(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let form = self.group_form.as_ref()?;
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
                    .id(format!("group-{key}")),
            );
        }
        Some(
            div()
                .p_4()
                .rounded(px(12.))
                .bg(rgba(tint(palette.raised, 0x30)))
                .border_1()
                .border_color(rgba(tint(palette.border, 0x25)))
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("New wish group"),
                )
                .child(fields)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("group-create-save")
                                .small()
                                .primary()
                                .label("Create group")
                                .on_click(
                                    cx.listener(|page, _, window, cx| {
                                        page.create_group(window, cx)
                                    }),
                                ),
                        )
                        .child(
                            Button::new("group-create-cancel")
                                .small()
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|page, _, _, cx| {
                                    page.group_form = None;
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_wish_item(
        &self,
        item: &InventoryItem,
        groups: &[WishGroup],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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

        let id = item.id.clone();
        let group_id = id.clone();
        let bought_id = id.clone();
        let edit_item = item.clone();
        let remove_id = id.clone();
        let busy = self.wish_busy.as_deref() == Some(id.as_str());
        let confirming_remove = self.confirm_remove_wish.as_deref() == Some(id.as_str());
        let picker_open = self.group_menu.as_deref() == Some(id.as_str());
        let mut card = div()
            .p_3p5()
            .rounded(px(12.))
            .bg(rgba(tint(palette.raised, 0x25)))
            .border_1()
            .border_color(rgba(tint(palette.border, 0x22)))
            .hover(|s| {
                s.bg(rgba(tint(palette.raised, 0x38)))
                    .border_color(rgba(tint(palette.border, 0x45)))
            })
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
                                            .bg(rgba(tint(color, 0x20)))
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
                            .flex()
                            .flex_col()
                            .items_end()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.text))
                                    .child(cost_range),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        Button::new(format!("wishlist-group-{id}"))
                                            .small()
                                            .ghost()
                                            .label("Group")
                                            .disabled(groups.is_empty())
                                            .on_click(cx.listener(move |page, _, _, cx| {
                                                page.group_menu = if page.group_menu.as_deref()
                                                    == Some(group_id.as_str())
                                                {
                                                    None
                                                } else {
                                                    Some(group_id.clone())
                                                };
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new(format!("wishlist-edit-{id}"))
                                            .small()
                                            .ghost()
                                            .label("Edit")
                                            .on_click(cx.listener(move |page, _, window, cx| {
                                                page.open_item_edit(edit_item.clone(), window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new(format!("wishlist-bought-{id}"))
                                            .small()
                                            .primary()
                                            .label(if busy { "Saving…" } else { "Mark bought" })
                                            .disabled(self.wish_busy.is_some())
                                            .on_click(cx.listener(move |page, _, _, cx| {
                                                page.set_wish_state(
                                                    bought_id.clone(),
                                                    InventoryState::Owned,
                                                    cx,
                                                )
                                            })),
                                    )
                                    .child(
                                        Button::new(format!("wishlist-remove-{id}"))
                                            .small()
                                            .ghost()
                                            .label(if confirming_remove {
                                                "Confirm removal"
                                            } else {
                                                "No longer wanted"
                                            })
                                            .disabled(self.wish_busy.is_some())
                                            .on_click(cx.listener(move |page, _, _, cx| {
                                                if page.confirm_remove_wish.as_deref()
                                                    == Some(remove_id.as_str())
                                                {
                                                    page.set_wish_state(
                                                        remove_id.clone(),
                                                        InventoryState::Gone,
                                                        cx,
                                                    );
                                                } else {
                                                    page.confirm_remove_wish =
                                                        Some(remove_id.clone());
                                                    cx.notify();
                                                }
                                            })),
                                    )
                                    .when(confirming_remove, |row| {
                                        row.child(
                                            Button::new(format!("wishlist-remove-cancel-{id}"))
                                                .small()
                                                .ghost()
                                                .label("Cancel")
                                                .on_click(cx.listener(move |page, _, _, cx| {
                                                    page.confirm_remove_wish = None;
                                                    cx.notify();
                                                })),
                                        )
                                    }),
                            ),
                    ),
            );
        if picker_open {
            card = card.child(self.render_group_picker(item, groups, palette, cx));
        }
        card.into_any_element()
    }

    /// The picker under one wish: one chip per group, filled where this wish is already a member.
    pub(super) fn render_group_picker(
        &self,
        item: &InventoryItem,
        groups: &[WishGroup],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = self.group_busy.is_some();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .pt_2()
            .border_t_1()
            .border_color(rgba(tint(palette.border, 0x22)))
            .children(groups.iter().map(|candidate| {
                let member = candidate.contains(&item.id);
                let group = candidate.clone();
                let item_id = item.id.clone();
                Button::new(format!("wish-group-{}-{}", candidate.id, item.id))
                    .small()
                    .label(if member {
                        format!("✓ {}", candidate.title)
                    } else {
                        candidate.title.clone()
                    })
                    .when(member, |button| button.primary())
                    .when(!member, |button| button.ghost())
                    .disabled(busy)
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.toggle_group_member(group.clone(), item_id.clone(), cx);
                    }))
            }))
            .into_any_element()
    }
}
