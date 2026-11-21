//! Phone wishlist, owned assets, and alternative comparisons.

use gpui::{
    div, prelude::*, rgb, rgba, AnyElement, Context, Div, Focusable, FontWeight, Subscription,
    Task, Window,
};
use sjel_gpui_core::inventory::{
    budget_of, counted, filter_inventory, format_cents, format_dimensions, group_of, months_to_pay,
    wishlist_view, InventoryItem,
};

use super::{format_item_price, load, snapshots, InventoryTab, Snapshot};
use crate::{
    ely::{
        buttons::{Button, ButtonVariant},
        feedback::InlineMessage,
        forms::{Input, InputEvent, TextInput},
        primitives::Severity,
        theme::ControlSize,
    },
    primitives::{note, surface_card},
    theme::{Palette, ThemeContext},
};

pub struct PhoneInventory {
    snapshot: Snapshot,
    tab: InventoryTab,
    search: gpui::Entity<TextInput>,
    group: Option<String>,
    category: Option<&'static str>,
    _input: [Subscription; 3],
    _updates: Task<()>,
}

impl PhoneInventory {
    pub fn new(window: &mut Window, keyboard: fn(bool), cx: &mut Context<Self>) -> Self {
        let mut source = snapshots().subscribe();
        let snapshot = source.borrow_and_update().clone();
        let search = cx
            .new(|cx| TextInput::new(window, cx).placeholder("Search items, notes, or category…"));
        let input = cx.subscribe_in(&search, window, |_, _, event, _, cx| {
            if matches!(event, InputEvent::Changed) {
                cx.notify();
            }
        });
        let focus = search.read(cx).focus_handle(cx);
        let on_focus = cx.on_focus(&focus, window, move |_, _, _| keyboard(true));
        let on_blur = cx.on_blur(&focus, window, move |_, _, _| keyboard(false));
        let updates = cx.spawn(async move |this, cx| {
            while source.changed().await.is_ok() {
                let snapshot = source.borrow_and_update().clone();
                if this
                    .update(cx, |view, cx| {
                        view.snapshot = snapshot;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            snapshot,
            tab: InventoryTab::Wishlist,
            search,
            group: None,
            category: None,
            _input: [input, on_focus, on_blur],
            _updates: updates,
        }
    }

    fn wishlist(&self, palette: Palette, cx: &Context<Self>) -> Div {
        let snap = &self.snapshot;
        let active = self
            .group
            .as_deref()
            .and_then(|id| snap.groups.iter().find(|group| group.id == id));
        let items = wishlist_view(&snap.wishlist.items, active, self.search.read(cx).text());
        let counted_items = counted(&items, &snap.groups);
        let budget = budget_of(counted_items.iter().copied());
        let low = format_cents(Some(budget.low_cents));
        let price = if budget.low_cents == budget.high_cents {
            low
        } else {
            format!("{low}–{}", format_cents(Some(budget.high_cents)))
        };
        let horizon = snap.wishlist.monthly_balance.as_ref().and_then(|bal| {
            months_to_pay(budget.low_cents, bal).map(|months| {
                format!(
                    "{months:.1} mo at {}/mo median surplus",
                    format_cents(Some(bal.median_cents))
                )
            })
        });
        let groups = std::iter::once((None, "All wishes".to_owned())).chain(
            snap.groups.iter().map(|group| {
                let label = if group.alternatives {
                    format!("{} (alt)", group.title)
                } else {
                    group.title.clone()
                };
                (Some(group.id.clone()), label)
            }),
        );
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                card(palette)
                    .child(title(format!("Counted budget · {price}")))
                    .child(caption(
                        palette,
                        format!(
                            "{} counted of {} visible · {} unpriced",
                            counted_items.len(),
                            items.len(),
                            budget.unpriced
                        ),
                    ))
                    .when_some(horizon, |card, horizon| {
                        card.child(caption(palette, horizon))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(groups.map(|(group, label)| {
                        let selected = group == self.group;
                        let id = group.clone().unwrap_or_else(|| "all-wishes".into());
                        tab_button(id, label, selected).on_click(cx.listener(
                            move |view, _, _, cx| {
                                view.group = group.clone();
                                cx.notify();
                            },
                        ))
                    })),
            )
            .when(items.is_empty(), |body| {
                body.child(caption(palette, "No matching wishlist items"))
            })
            .children(items.iter().map(|item| {
                let mut fields = vec![item_summary(item)];
                if let Some(priority) = item
                    .priority
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                {
                    fields.push(format!("Priority {priority}"));
                }
                if !counted_items.iter().any(|counted| counted.id == item.id) {
                    fields.push("Alternative".into());
                }
                item_card(palette, item, fields.join(" · ")).when_some(
                    item.notes.as_ref().filter(|notes| !notes.trim().is_empty()),
                    |card, notes| card.child(div().text_sm().child(notes.clone())),
                )
            }))
    }

    fn owned(&self, palette: Palette, cx: &Context<Self>) -> Div {
        let categories = [
            (None, "All owned"),
            (Some("furniture"), "Furniture"),
            (Some("gear"), "Gear"),
            (Some("needs"), "Needs"),
        ];
        let items = filter_inventory(
            &self.snapshot.rows,
            self.search.read(cx).text(),
            self.category,
            Some("owned"),
        );
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().flex().flex_wrap().gap_1().children(categories.map(
                |(category, label)| {
                    tab_button(label, label, category == self.category).on_click(cx.listener(
                        move |view, _, _, cx| {
                            view.category = category;
                            cx.notify();
                        },
                    ))
                },
            )))
            .child(caption(
                palette,
                format!("Showing {} owned assets", items.len()),
            ))
            .when(items.is_empty(), |body| {
                body.child(caption(palette, "No matching owned items"))
            })
            .children(items.into_iter().take(80).map(|row| {
                item_card(
                    palette,
                    &row.item,
                    format!("{} · {}", item_summary(&row.item), row.item.id),
                )
            }))
    }

    fn compare(&self, palette: Palette) -> Div {
        let items = &self.snapshot.wishlist.items;
        let counted_items = counted(&items.iter().collect::<Vec<_>>(), &self.snapshot.groups);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .when(self.snapshot.groups.is_empty(), |body| {
                body.child(caption(palette, "No wish groups to compare"))
            })
            .children(self.snapshot.groups.iter().map(|group| {
                let mut members = items
                    .iter()
                    .filter(|item| group.contains(&item.id))
                    .peekable();
                let mode = if group.alternatives {
                    "Alternatives · Cheapest priced option counts"
                } else {
                    "Bundle · All open options add up"
                };
                card(palette)
                    .child(title(group.title.clone()))
                    .child(caption(palette, mode))
                    .when(!group.constraint_text.trim().is_empty(), |card| {
                        card.child(caption(palette, group.constraint_text.clone()))
                    })
                    .when(members.peek().is_none(), |card| {
                        card.child(caption(palette, "No open wishes remaining in this group"))
                    })
                    .children(members.map(|item| {
                        let label = if counted_items.iter().any(|counted| counted.id == item.id) {
                            "Counted in budget"
                        } else {
                            "Alternative option"
                        };
                        div()
                            .border_t_1()
                            .border_color(rgba(palette.border))
                            .py_2()
                            .child(format!("{} · {}", item.label, format_item_price(item)))
                            .child(caption(
                                palette,
                                format!("{} · {label}", item_summary(item)),
                            ))
                    }))
            }))
    }
}

impl Render for PhoneInventory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        let content = match self.tab {
            InventoryTab::Wishlist => self.wishlist(palette, cx),
            InventoryTab::Owned => self.owned(palette, cx),
            InventoryTab::Compare => self.compare(palette),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .bg(rgba(palette.window_tint))
            .text_color(rgb(palette.text))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(title("Inventory & Wishlist").text_xl())
                    .child(
                        Button::new("inventory-refresh", "Refresh")
                            .variant(ButtonVariant::Ghost)
                            .on_click(|_, _, _| load()),
                    ),
            )
            .child(caption(palette, self.snapshot.status.clone()))
            .child(
                div().flex().flex_wrap().gap_1().children(
                    [
                        InventoryTab::Wishlist,
                        InventoryTab::Owned,
                        InventoryTab::Compare,
                    ]
                    .map(|tab| {
                        tab_button(tab.label(), tab.label(), tab == self.tab).on_click(cx.listener(
                            move |view, _, window, cx| {
                                if tab == InventoryTab::Compare {
                                    window.blur(cx);
                                }
                                view.tab = tab;
                                cx.notify();
                            },
                        ))
                    }),
                ),
            )
            .when(self.tab != InventoryTab::Compare, |body| {
                body.child(Input::new(&self.search))
            })
            .when_some(self.snapshot.error.clone(), |body, error| {
                body.child(InlineMessage::new(Severity::Danger, error))
            })
            .child(
                div()
                    .id(self.tab.label())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(content),
            )
    }
}

fn tab_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
    selected: bool,
) -> Button {
    Button::new(id, label)
        .size(ControlSize::Lg)
        .variant(if selected {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Outline
        })
}

fn title(text: impl Into<gpui::SharedString>) -> Div {
    div().font_weight(FontWeight::SEMIBOLD).child(text.into())
}

fn caption(palette: Palette, text: impl Into<gpui::SharedString>) -> AnyElement {
    note(text, palette.muted)
}

fn card(palette: Palette) -> Div {
    surface_card(palette).flex().flex_col().gap_1().p_3()
}

fn item_summary(item: &InventoryItem) -> String {
    let dimensions = format_dimensions(item.width, item.depth, item.height);
    if dimensions == "—" {
        group_of(item)
    } else {
        format!("{} · {dimensions}", group_of(item))
    }
}

fn item_card(palette: Palette, item: &InventoryItem, details: String) -> Div {
    card(palette)
        .child(
            div()
                .flex()
                .justify_between()
                .gap_2()
                .child(title(item.label.clone()))
                .child(format_item_price(item)),
        )
        .child(caption(palette, details))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn phone_inventory_mounts_at_phone_viewport(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::mount_root(
            cx,
            Some(gpui::size(gpui::px(390.), gpui::px(844.))),
            |window, cx| PhoneInventory::new(window, |_| {}, cx),
        );
        view.read_with(cx, |view, _| assert!(view.tab == InventoryTab::Wishlist));
    }
}
