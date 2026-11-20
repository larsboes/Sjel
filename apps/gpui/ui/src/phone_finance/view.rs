//! Phone finance dashboard, registers, and import review.

use gpui::{
    div, prelude::*, relative, rgb, rgba, Context, Div, Entity, Focusable, FontWeight,
    SharedString, Subscription, Task, Window,
};
use sjel_gpui_core::{
    finance::{
        calculate_monthly_burn, filter_transactions, format_exact_money, SubscriptionSummary,
        TransactionFilter,
    },
    shell::{BalanceKind, FinanceTransaction},
};

use super::{
    format_signed_cents as money, load, review_candidate, snapshots, FinanceTab, Snapshot,
};
use crate::{
    ely::{
        buttons::{Button, ButtonVariant},
        forms::{Input, InputEvent, TextInput},
        theme::ControlSize,
    },
    theme::{Palette, ThemeContext},
};

pub struct PhoneFinance {
    snapshot: Snapshot,
    tab: FinanceTab,
    search: Entity<TextInput>,
    category: Option<String>,
    active_only: bool,
    _input: [Subscription; 3],
    _updates: Task<()>,
}

impl PhoneFinance {
    pub fn new(window: &mut Window, keyboard: fn(bool), cx: &mut Context<Self>) -> Self {
        let mut source = snapshots().subscribe();
        let snapshot = source.borrow_and_update().clone();
        let search = cx.new(|cx| {
            TextInput::new(window, cx).placeholder("Search description, account, or category…")
        });
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
            tab: FinanceTab::Overview,
            search,
            category: None,
            active_only: true,
            _input: [input, on_focus, on_blur],
            _updates: updates,
        }
    }

    fn overview(&self, palette: Palette) -> Div {
        let dashboard = &self.snapshot.dashboard;
        let summary = &dashboard.summary;
        let currency = &summary.currency;
        let worth = dashboard
            .tracked_net_worth
            .as_ref()
            .and_then(|value| {
                format_exact_money(&value.value.mantissa, value.value.scale, &value.currency)
            })
            .unwrap_or_else(|| "Not tracked".into());
        let metrics = [
            (
                "Net worth",
                worth,
                "Tracked balances & portfolio".to_owned(),
            ),
            (
                "Net flow",
                money(
                    summary.income_cents - summary.personal_spending_cents,
                    currency,
                ),
                format!(
                    "In {} · Out {}",
                    money(summary.income_cents, currency),
                    money(summary.personal_spending_cents, currency)
                ),
            ),
            (
                "Spending",
                money(summary.personal_spending_cents, currency),
                "Personal outflow in window".to_owned(),
            ),
            (
                "Subscriptions",
                money(calculate_monthly_burn(&self.snapshot.subscriptions), "EUR"),
                "Monthly recurring burn".to_owned(),
            ),
        ];
        let mut overview = column().children(metrics.map(|(label, value, note)| {
            card(palette)
                .child(caption(palette, label))
                .child(title(value))
                .child(caption(palette, note))
        }));
        if let Some(planning) = dashboard
            .planning
            .as_ref()
            .filter(|plan| !plan.spend_goals.is_empty())
        {
            overview = overview.child(card(palette).child(title("Spend goals")).children(
                planning.spend_goals.iter().map(|goal| {
                    let progress = if goal.target_cents > 0 {
                        (goal.spent_cents as f32 / goal.target_cents as f32).clamp(0., 1.)
                    } else {
                        0.
                    };
                    column()
                        .child(label_value(
                            goal.label.clone(),
                            format!(
                                "{} / {}",
                                money(goal.spent_cents, &goal.currency),
                                money(goal.target_cents, &goal.currency)
                            ),
                        ))
                        .child(div().h_1().rounded_sm().bg(rgba(palette.border)).child(
                            div().h_full().w(relative(progress)).rounded_sm().bg(rgb(
                                if goal.met {
                                    palette.green
                                } else {
                                    palette.accent
                                },
                            )),
                        ))
                        .child(caption(
                            palette,
                            format!(
                                "Remaining {} · {}d left ({}/day)",
                                money(goal.remaining_cents, &goal.currency),
                                goal.days_left,
                                money(goal.needed_per_day_cents, &goal.currency)
                            ),
                        ))
                }),
            ));
        }
        if let Some(balances) = dashboard
            .balance_snapshot
            .as_ref()
            .filter(|balances| !balances.balances.is_empty())
        {
            overview = overview.child(
                card(palette)
                    .child(title(format!("Balances as of {}", balances.as_of)))
                    .children(balances.balances.iter().map(|balance| {
                        let kind = match balance.kind {
                            BalanceKind::Asset => "Asset",
                            BalanceKind::Liability => "Liability",
                        };
                        div()
                            .child(label_value(
                                balance.label.clone(),
                                money(balance.amount_cents, &balances.currency),
                            ))
                            .child(caption(palette, kind))
                    })),
            );
        }
        if !dashboard.transactions.is_empty() {
            overview = overview.child(
                card(palette).child(title("Recent activity")).children(
                    dashboard
                        .transactions
                        .iter()
                        .take(5)
                        .map(|tx| transaction(palette, tx)),
                ),
            );
        }
        overview
    }

    fn transactions(&self, palette: Palette, cx: &Context<Self>) -> Div {
        let dashboard = &self.snapshot.dashboard;
        let filter = TransactionFilter {
            query: self.search.read(cx).text().to_owned(),
            category: self.category.clone(),
        };
        let rows = filter_transactions(&dashboard.transactions, &filter);
        let categories = std::iter::once((None, "All categories".to_owned())).chain(
            dashboard
                .categories
                .iter()
                .take(8)
                .map(|category| (Some(category.clone()), category.clone())),
        );
        column()
            .child(div().flex().flex_wrap().gap_1().children(categories.map(
                |(category, label)| {
                    choice(label.clone(), label, category == self.category).on_click(cx.listener(
                        move |view, _, _, cx| {
                            view.category = category.clone();
                            cx.notify();
                        },
                    ))
                },
            )))
            .child(caption(
                palette,
                format!(
                    "Showing {} of {} transactions",
                    rows.len(),
                    dashboard.transactions.len()
                ),
            ))
            .when(rows.is_empty(), |list| {
                list.child(caption(palette, "No matching transactions"))
            })
            .children(
                rows.into_iter()
                    .take(60)
                    .map(|tx| card(palette).child(transaction(palette, tx))),
            )
    }

    fn subscriptions(&self, palette: Palette, cx: &Context<Self>) -> Div {
        let records = &self.snapshot.subscriptions;
        let rows: Vec<_> = records
            .iter()
            .map(SubscriptionSummary::from_record)
            .filter(|row| !self.active_only || matches!(row.status.as_str(), "active" | "trial"))
            .collect();
        column()
            .child(div().flex().flex_wrap().gap_1().children(
                [(true, "Active only"), (false, "All states")].map(|(active, label)| {
                    choice(label, label, active == self.active_only).on_click(cx.listener(
                        move |view, _, _, cx| {
                            view.active_only = active;
                            cx.notify();
                        },
                    ))
                }),
            ))
            .child(caption(
                palette,
                format!(
                    "Recurring burn · {}/mo",
                    money(calculate_monthly_burn(records), "EUR")
                ),
            ))
            .when(rows.is_empty(), |list| {
                list.child(caption(palette, "No subscriptions listed"))
            })
            .children(rows.into_iter().map(|row| {
                card(palette)
                    .child(label_value(row.name, row.status))
                    .child(caption(
                        palette,
                        format!("{} · {}", row.category, row.cycle),
                    ))
                    .child(format!(
                        "{} ({}/mo)",
                        money(row.amount_cents, &row.currency),
                        money(row.monthly_equivalent_cents, &row.currency)
                    ))
            }))
    }

    fn review(&self, palette: Palette) -> Div {
        let rows = &self.snapshot.candidates;
        let pending = rows.iter().filter(|row| row.state == "pending").count();
        column()
            .child(caption(
                palette,
                format!(
                    "{pending} pending candidates ({} total imported)",
                    rows.len()
                ),
            ))
            .when(pending == 0, |list| {
                list.child(caption(palette, "Review queue clear"))
            })
            .children(rows.iter().filter(|row| row.state == "pending").map(|row| {
                card(palette)
                    .child(label_value(
                        row.description.clone(),
                        money(row.amount_cents, &row.currency),
                    ))
                    .child(caption(
                        palette,
                        format!(
                            "{} · {} → {} ({}%)",
                            row.booked_at,
                            row.source_account,
                            row.proposed_account,
                            row.confidence_basis_points / 100
                        ),
                    ))
                    .child(div().flex().gap_2().children(
                        [("confirm", "Confirm"), ("reject", "Reject")].map(|(decision, label)| {
                            let id = row.id.clone();
                            choice(format!("{id}-{decision}"), label, false)
                                .disabled(self.snapshot.reviewing.contains(&id))
                                .on_click(move |_, _, _| review_candidate(id.clone(), decision))
                        }),
                    ))
            }))
    }
}

impl Render for PhoneFinance {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        let pending = self
            .snapshot
            .candidates
            .iter()
            .filter(|row| row.state == "pending")
            .count();
        let content = match self.tab {
            FinanceTab::Overview => self.overview(palette),
            FinanceTab::Transactions => self.transactions(palette, cx),
            FinanceTab::Subscriptions => self.subscriptions(palette, cx),
            FinanceTab::Review => self.review(palette),
        };
        column()
            .size_full()
            .p_4()
            .bg(rgba(palette.surface))
            .text_color(rgb(palette.text))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(title("Finance").text_xl())
                    .child(
                        Button::new("finance-refresh", "Refresh")
                            .variant(ButtonVariant::Ghost)
                            .on_click(|_, _, _| load()),
                    ),
            )
            .child(caption(palette, self.snapshot.status.clone()))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(FinanceTab::ALL.map(|tab| {
                        let label = if tab == FinanceTab::Review && pending > 0 {
                            format!("Review ({pending})")
                        } else {
                            tab.label().to_owned()
                        };
                        choice(tab.label(), label, tab == self.tab).on_click(cx.listener(
                            move |view, _, window, cx| {
                                if view.tab == FinanceTab::Transactions
                                    && tab != FinanceTab::Transactions
                                {
                                    window.blur(cx);
                                }
                                view.tab = tab;
                                cx.notify();
                            },
                        ))
                    })),
            )
            .when(self.tab == FinanceTab::Transactions, |screen| {
                screen.child(Input::new(&self.search))
            })
            .when_some(self.snapshot.error.clone(), |screen, error| {
                screen.child(div().text_sm().text_color(rgb(palette.red)).child(error))
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

fn column() -> Div {
    div().flex().flex_col().gap_2()
}

fn card(palette: Palette) -> Div {
    column()
        .rounded_md()
        .border_1()
        .border_color(rgba(palette.border))
        .p_3()
}

fn title(text: impl Into<SharedString>) -> Div {
    div().font_weight(FontWeight::SEMIBOLD).child(text.into())
}

fn caption(palette: Palette, text: impl Into<SharedString>) -> Div {
    div()
        .text_sm()
        .text_color(rgb(palette.muted))
        .child(text.into())
}

fn choice(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
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

fn label_value(label: String, value: String) -> Div {
    div()
        .flex()
        .justify_between()
        .gap_2()
        .child(title(label))
        .child(value)
}

fn transaction(palette: Palette, tx: &FinanceTransaction) -> Div {
    div()
        .child(label_value(
            tx.description.clone(),
            money(tx.amount_cents, &tx.currency),
        ))
        .child(caption(
            palette,
            format!("{} · {} · {}", tx.date, tx.account, tx.category),
        ))
}
