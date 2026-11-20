//! Phone Inventory & Wishlist screen drawn by NSH: on-the-go wishlist lookup, budget & payback
//! metrics, owned household assets, and wish-group alternative comparisons (`sjel_gpui_core::inventory`).

use nsh_base::{button, input};
use nsh_core::prelude::*;
use sjel_gpui_core::inventory::{
    budget_of, counted, filter_inventory, format_cents, format_dimensions, group_of, months_to_pay,
    wishlist_view, InventoryItem,
};

use crate::nsh_ui::{
    caption, empty_state, heading, metric_tile, pill_tab, status_chip, surface_card,
};

use crate::phone_inventory::{format_item_price, load, snapshots, InventoryTab, Snapshot};
pub use crate::phone_inventory::connect;

fn wishlist_section(
    snap: &Snapshot,
    query: Signal<String>,
    selected_group: Signal<Option<String>>,
) -> nsh_core::element::AnyElement {
    let active_group = selected_group
        .get()
        .and_then(|id| snap.groups.iter().find(|g| g.id == id));
    let visible = wishlist_view(&snap.wishlist.items, active_group, &query.get());
    let counted_items = counted(&visible, &snap.groups);
    let budget = budget_of(counted_items.iter().copied());
    let budget_text = if budget.low_cents == budget.high_cents {
        format_cents(Some(budget.low_cents))
    } else {
        format!(
            "{}–{}",
            format_cents(Some(budget.low_cents)),
            format_cents(Some(budget.high_cents))
        )
    };
    let payback_text = snap
        .wishlist
        .monthly_balance
        .as_ref()
        .and_then(|bal| months_to_pay(budget.low_cents, bal))
        .map(|m| format!("{m:.1} mo"))
        .unwrap_or_else(|| format!("{} items", counted_items.len()));
    let payback_caption = snap
        .wishlist
        .monthly_balance
        .as_ref()
        .map(|bal| {
            format!(
                "At {}/mo median surplus",
                format_cents(Some(bal.median_cents))
            )
        })
        .unwrap_or_else(|| format!("{} unpriced", budget.unpriced));

    let mut group_pills = view().flex_row().flex_wrap().gap_1p5().child(pill_tab(
        "All wishes",
        selected_group.get().is_none(),
        move || selected_group.set(None),
    ));
    for group in &snap.groups {
        let gid = group.id.clone();
        let active = selected_group.with(|g| g.as_deref() == Some(group.id.as_str()));
        let label = if group.alternatives {
            format!("{} (alt)", group.title)
        } else {
            group.title.clone()
        };
        group_pills = group_pills.child(pill_tab(label, active, move || {
            selected_group.set(Some(gid.clone()))
        }));
    }

    let mut out = view()
        .gap_3()
        .child(
            view()
                .flex_row()
                .gap_3()
                .child(metric_tile(
                    "COUNTED BUDGET",
                    budget_text,
                    format!(
                        "{} counted of {} visible",
                        counted_items.len(),
                        visible.len()
                    ),
                ))
                .child(metric_tile(
                    "SURPLUS HORIZON",
                    payback_text,
                    payback_caption,
                )),
        )
        .child(group_pills);

    if visible.is_empty() {
        out = out.child(empty_state(
            "No matching wishlist items",
            "Clear the search filter or select another wish group.",
        ));
    } else {
        for item in visible {
            let is_counted = counted_items.iter().any(|c| c.id == item.id);
            let dims = format_dimensions(item.width, item.depth, item.height);
            let category = group_of(item);
            let mut meta_parts = vec![category];
            if dims != "—" {
                meta_parts.push(dims);
            }
            if let Some(prio) = &item.priority {
                if !prio.trim().is_empty() {
                    meta_parts.push(format!("Priority {prio}"));
                }
            }
            if !is_counted {
                meta_parts.push("Alternative (not counted)".into());
            }
            let mut card = surface_card()
                .px_4()
                .py_3()
                .gap_1()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            text(item.label.clone())
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .flex_1(),
                        )
                        .child(status_chip(format_item_price(item), is_counted)),
                )
                .child(caption(meta_parts.join(" · ")));
            if let Some(notes) = &item.notes {
                if !notes.trim().is_empty() {
                    card = card.child(text(notes.clone()).text_xs());
                }
            }
            out = out.child(card);
        }
    }

    out.into_element()
}

fn owned_section(
    snap: &Snapshot,
    query: Signal<String>,
    category_filter: Signal<Option<&'static str>>,
) -> nsh_core::element::AnyElement {
    let filtered = filter_inventory(
        &snap.rows,
        &query.get(),
        category_filter.get(),
        Some("owned"),
    );

    let mut cat_pills = view().flex_row().flex_wrap().gap_1p5().child(pill_tab(
        "All owned",
        category_filter.get().is_none(),
        move || category_filter.set(None),
    ));
    for (key, label) in [
        ("furniture", "Furniture"),
        ("gear", "Gear"),
        ("needs", "Needs"),
    ] {
        let active = category_filter.get() == Some(key);
        cat_pills = cat_pills.child(pill_tab(label, active, move || {
            category_filter.set(Some(key))
        }));
    }

    let mut out = view()
        .gap_2()
        .child(cat_pills)
        .child(caption(format!("Showing {} owned assets", filtered.len())));

    if filtered.is_empty() {
        out = out.child(empty_state(
            "No matching owned items",
            "Try another category or search term.",
        ));
    } else {
        for row in filtered.into_iter().take(80) {
            let item = &row.item;
            let dims = format_dimensions(item.width, item.depth, item.height);
            let category = group_of(item);
            let subtitle = if dims == "—" {
                format!("{category} · {}", item.id)
            } else {
                format!("{category} · {dims} · {}", item.id)
            };
            out = out.child(
                surface_card()
                    .px_4()
                    .py_3()
                    .gap_1()
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                text(item.label.clone())
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .flex_1(),
                            )
                            .child(status_chip(format_item_price(item), false)),
                    )
                    .child(caption(subtitle)),
            );
        }
    }
    out.into_element()
}

fn compare_section(snap: &Snapshot) -> nsh_core::element::AnyElement {
    let mut out = view().gap_3();
    if snap.groups.is_empty() {
        return out
            .child(empty_state(
                "No wish groups to compare",
                "Create wish groups on the Mac or connect to load group alternatives.",
            ))
            .into_element();
    }

    let all_refs: Vec<&InventoryItem> = snap.wishlist.items.iter().collect();
    let counted_items = counted(&all_refs, &snap.groups);

    for group in &snap.groups {
        let mode_label = if group.alternatives {
            "Alternatives · Cheapest priced option counts"
        } else {
            "Bundle · All open options add up"
        };
        let mut card = surface_card()
            .px_4()
            .py_3()
            .gap_2()
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        text(group.title.clone())
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(status_chip(
                        if group.alternatives {
                            "Alternatives"
                        } else {
                            "Bundle"
                        },
                        group.alternatives,
                    )),
            )
            .child(caption(mode_label));

        if !group.constraint_text.trim().is_empty() {
            card = card.child(caption(format!("Constraint: {}", group.constraint_text)));
        }

        let members: Vec<&InventoryItem> = snap
            .wishlist
            .items
            .iter()
            .filter(|item| group.contains(&item.id))
            .collect();
        if members.is_empty() {
            card = card.child(caption("No open wishes remaining in this group."));
        } else {
            for member in members {
                let is_counted = counted_items.iter().any(|c| c.id == member.id);
                let dims = format_dimensions(member.width, member.depth, member.height);
                card = card.child(
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .py_1()
                        .child(
                            view()
                                .flex_1()
                                .gap_0p5()
                                .child(
                                    text(member.label.clone())
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM),
                                )
                                .child(caption(if dims == "—" {
                                    if is_counted {
                                        "Counted in budget".into()
                                    } else {
                                        "Alternative option".into()
                                    }
                                } else if is_counted {
                                    format!("{dims} · Counted in budget")
                                } else {
                                    format!("{dims} · Alternative option")
                                })),
                        )
                        .child(status_chip(format_item_price(member), is_counted)),
                );
            }
        }
        out = out.child(card);
    }

    out.into_element()
}

/// Phone Inventory & Wishlist screen with Wishlist, Owned, and Compare sub-tabs.
#[component]
pub fn InventoryScreen() -> impl IntoElement {
    let snap = crate::nsh_ui::snapshot(snapshots());
    let tab = signal(InventoryTab::Wishlist);
    let query = signal(String::new());
    let selected_group = signal(None::<String>);
    let category_filter = signal(None::<&'static str>);

    view()
        .flex_1()
        .px_4()
        .pt_2()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    view()
                        .gap_0p5()
                        .child(heading("Inventory & Wishlist"))
                        .child(text(move || snap.with(|s| s.status.clone())).text_xs()),
                )
                .child(
                    button()
                        .px_3()
                        .py_1p5()
                        .rounded_lg()
                        .border(px(1.0))
                        .border_color(move || nsh_base::theme().border)
                        .on_press(load)
                        .child(text("Refresh").text_xs().font_weight(FontWeight::SEMIBOLD)),
                ),
        )
        .child(dynamic(move || {
            let active = tab.get();
            view()
                .flex_row()
                .flex_wrap()
                .gap_1p5()
                .child(pill_tab(
                    "Wishlist",
                    active == InventoryTab::Wishlist,
                    move || tab.set(InventoryTab::Wishlist),
                ))
                .child(pill_tab(
                    "Owned",
                    active == InventoryTab::Owned,
                    move || tab.set(InventoryTab::Owned),
                ))
                .child(pill_tab(
                    "Compare",
                    active == InventoryTab::Compare,
                    move || tab.set(InventoryTab::Compare),
                ))
                .into_element()
        }))
        .child(dynamic(move || {
            (tab.get() != InventoryTab::Compare).then(|| {
                input()
                    .test_id("inventory-search")
                    .placeholder("Search items, notes, or category…")
                    .value(move || query.get())
                    .on_change(move |v: &str| query.set(v.to_owned()))
                    .into_element()
            })
        }))
        .child(scroll().flex_1().child(dynamic(move || {
            let current = snap.get();
            let mut body = view().gap_3().pb_6();
            if let Some(err) = &current.error {
                body = body.child(text(err.clone()).text_sm());
            }
            let section = match tab.get() {
                InventoryTab::Wishlist => wishlist_section(&current, query, selected_group),
                InventoryTab::Owned => owned_section(&current, query, category_filter),
                InventoryTab::Compare => compare_section(&current),
            };
            body.child(section).into_element()
        })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phone_inventory::set_snapshot;
    use sjel_gpui_core::inventory::{InventoryRow, InventoryWishlist, MonthlyBalance, WishGroup};

    #[test]
    fn phone_inventory_renders_wishlist_budget_owned_assets_and_group_comparison() {
        use nsh_core::element::Root;
        use nsh_engine::EngineConfig;
        use nsh_testing::Headless;

        let chair_a = InventoryItem {
            id: "chair-a".into(),
            kind: "piece".into(),
            label: "Oak Lounge Chair".into(),
            width: Some(72),
            depth: Some(78),
            height: Some(75),
            price_cents: Some(45_000),
            category: Some("furniture".into()),
            ..Default::default()
        };
        let chair_b = InventoryItem {
            id: "chair-b".into(),
            kind: "piece".into(),
            label: "Walnut Reading Chair".into(),
            width: Some(70),
            depth: Some(75),
            height: Some(74),
            price_cents: Some(68_000),
            category: Some("furniture".into()),
            ..Default::default()
        };
        let owned_desk = InventoryItem {
            id: "desk-1".into(),
            kind: "piece".into(),
            label: "Standing Studio Desk".into(),
            width: Some(160),
            depth: Some(80),
            height: Some(74),
            price_cents: Some(89_000),
            category: Some("furniture".into()),
            ..Default::default()
        };

        let rows = vec![InventoryRow {
            item: owned_desk,
            state: Some("owned".into()),
        }];
        let wishlist = InventoryWishlist {
            items: vec![chair_a.clone(), chair_b.clone()],
            total_low_cents: 45_000,
            total_high_cents: 68_000,
            unpriced_count: 0,
            monthly_balance: Some(MonthlyBalance {
                median_cents: 90_000,
                months: 6,
                from: "2026-05".into(),
                to: "2026-10".into(),
            }),
            months_to_pay: Some(0.5),
        };
        let groups = vec![WishGroup {
            id: "seating".into(),
            title: "Reading Corner Chair".into(),
            constraint_text: "Max width 75 cm".into(),
            alternatives: true,
            items: vec![chair_a, chair_b],
            items_count: 2,
            total_low_cents: 45_000,
            total_high_cents: 45_000,
        }];
        set_snapshot(rows, wishlist, groups);

        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(Root::of::<InventoryScreen>())
        })
        .expect("InventoryScreen mounts in headless NSH");

        app.wait_for("text:Inventory & Wishlist").unwrap();
        app.wait_for("text:Oak Lounge Chair").unwrap();
        app.wait_for("text:Walnut Reading Chair").unwrap();
        app.wait_for("text:450.00 €").unwrap();

        app.press("text:Owned").unwrap();
        app.wait_for("text:Standing Studio Desk").unwrap();

        app.press("text:Compare").unwrap();
        app.wait_for("text:Reading Corner Chair").unwrap();
        app.wait_for("text:Alternatives · Cheapest priced option counts")
            .unwrap();
    }
}
