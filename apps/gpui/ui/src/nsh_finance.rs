//! Phone Finance screen drawn by NSH: overview, spend goals, transactions, subscriptions, and
//! candidate review (`sjel_gpui_core::finance`).

use nsh_base::{button, input};
use nsh_core::prelude::*;
use sjel_gpui_core::finance::{
    calculate_monthly_burn, filter_transactions, format_exact_money, SubscriptionSummary,
    TransactionFilter,
};
use sjel_gpui_core::shell::{BalanceKind, FinanceCandidate};

use crate::nsh_ui::{
    action, caption, empty_state, heading, metric_tile, pill_tab, progress_bar, section_heading,
    snapshot, status_chip, surface_card,
};

use crate::phone_finance::{
    format_signed_cents, load, review_candidate, snapshots, FinanceTab, Snapshot,
};
pub use crate::phone_finance::connect;

fn overview_section(snap: &Snapshot) -> nsh_core::element::AnyElement {
    let summary = &snap.dashboard.summary;
    let currency = if summary.currency.is_empty() {
        "EUR"
    } else {
        summary.currency.as_str()
    };
    let net_cents = summary.income_cents - summary.personal_spending_cents;
    let net_worth_text = snap
        .dashboard
        .tracked_net_worth
        .as_ref()
        .and_then(|nw| format_exact_money(&nw.value.mantissa, nw.value.scale, &nw.currency))
        .unwrap_or_else(|| "Not tracked".into());
    let monthly_burn = calculate_monthly_burn(&snap.subscriptions);

    let mut out = view()
        .gap_3()
        .child(
            view()
                .flex_row()
                .gap_3()
                .child(metric_tile(
                    "NET WORTH",
                    net_worth_text,
                    "Tracked balances & portfolio",
                ))
                .child(metric_tile(
                    "NET FLOW",
                    format_signed_cents(net_cents, currency),
                    format!(
                        "In {} · Out {}",
                        format_signed_cents(summary.income_cents, currency),
                        format_signed_cents(summary.personal_spending_cents, currency)
                    ),
                )),
        )
        .child(
            view()
                .flex_row()
                .gap_3()
                .child(metric_tile(
                    "SPENDING",
                    format_signed_cents(summary.personal_spending_cents, currency),
                    "Personal outflow in window",
                ))
                .child(metric_tile(
                    "SUBSCRIPTIONS",
                    format_signed_cents(monthly_burn, "EUR"),
                    "Monthly recurring burn",
                )),
        );

    let goals = snap
        .dashboard
        .planning
        .as_ref()
        .map(|p| p.spend_goals.as_slice())
        .unwrap_or(&[]);
    if !goals.is_empty() {
        let mut goals_card = surface_card()
            .px_4()
            .py_3()
            .gap_2()
            .child(section_heading("SPEND GOALS"));
        for goal in goals {
            let ratio = if goal.target_cents > 0 {
                goal.spent_cents as f32 / goal.target_cents as f32
            } else {
                0.0
            };
            goals_card = goals_card.child(
                view()
                    .gap_1()
                    .py_1()
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                text(goal.label.clone())
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM),
                            )
                            .child(status_chip(
                                format!(
                                    "{} / {}",
                                    format_signed_cents(goal.spent_cents, &goal.currency),
                                    format_signed_cents(goal.target_cents, &goal.currency)
                                ),
                                !goal.met,
                            )),
                    )
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(caption(format!(
                                "Remaining {} · {}d left ({}/day)",
                                format_signed_cents(goal.remaining_cents, &goal.currency),
                                goal.days_left,
                                format_signed_cents(goal.needed_per_day_cents, &goal.currency)
                            )))
                            .child(progress_bar(ratio)),
                    ),
            );
        }
        out = out.child(goals_card);
    }

    if let Some(balance_snap) = &snap.dashboard.balance_snapshot {
        if !balance_snap.balances.is_empty() {
            let mut bal_card =
                surface_card()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .child(section_heading(format!(
                        "BALANCES AS OF {}",
                        balance_snap.as_of
                    )));
            for bal in &balance_snap.balances {
                let kind_label = match bal.kind {
                    BalanceKind::Asset => "Asset",
                    BalanceKind::Liability => "Liability",
                };
                bal_card = bal_card.child(
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .py_1()
                        .child(
                            view()
                                .gap_0p5()
                                .child(
                                    text(bal.label.clone())
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM),
                                )
                                .child(caption(kind_label)),
                        )
                        .child(
                            text(format_signed_cents(
                                bal.amount_cents,
                                &balance_snap.currency,
                            ))
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD),
                        ),
                );
            }
            out = out.child(bal_card);
        }
    }

    if !snap.dashboard.transactions.is_empty() {
        let mut recent = surface_card()
            .px_4()
            .py_3()
            .gap_2()
            .child(section_heading("RECENT ACTIVITY"));
        for tx in snap.dashboard.transactions.iter().take(5) {
            recent = recent.child(transaction_row(tx));
        }
        out = out.child(recent);
    }

    out.into_element()
}

fn transaction_row(tx: &sjel_gpui_core::shell::FinanceTransaction) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .py_1p5()
        .child(
            view()
                .flex_1()
                .gap_0p5()
                .child(
                    text(tx.description.clone())
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .number_of_lines(1u32),
                )
                .child(caption(format!(
                    "{} · {} · {}",
                    tx.date, tx.account, tx.category
                ))),
        )
        .child(
            text(format_signed_cents(tx.amount_cents, &tx.currency))
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD),
        )
}

fn transactions_section(
    snap: &Snapshot,
    query: Signal<String>,
    category: Signal<Option<String>>,
) -> nsh_core::element::AnyElement {
    let filter = TransactionFilter {
        query: query.get(),
        category: category.get(),
    };
    let filtered = filter_transactions(&snap.dashboard.transactions, &filter);
    let mut cat_strip = view().flex_row().flex_wrap().gap_1p5().child(pill_tab(
        "All categories",
        category.get().is_none(),
        move || category.set(None),
    ));
    for cat in snap.dashboard.categories.iter().take(8) {
        let cat_name = cat.clone();
        let active = category.with(|c| c.as_deref() == Some(cat.as_str()));
        cat_strip = cat_strip.child(pill_tab(cat_name.clone(), active, move || {
            category.set(Some(cat_name.clone()))
        }));
    }

    let mut out = view().gap_2().child(cat_strip).child(caption(format!(
        "Showing {} of {} transactions",
        filtered.len(),
        snap.dashboard.transactions.len()
    )));

    if filtered.is_empty() {
        out = out.child(empty_state(
            "No matching transactions",
            "Adjust the search query or category filter.",
        ));
    } else {
        for tx in filtered.into_iter().take(60) {
            out = out.child(surface_card().px_3().py_2().child(transaction_row(tx)));
        }
    }
    out.into_element()
}

fn subscriptions_section(
    snap: &Snapshot,
    active_only: Signal<bool>,
) -> nsh_core::element::AnyElement {
    let summaries: Vec<SubscriptionSummary> = snap
        .subscriptions
        .iter()
        .map(SubscriptionSummary::from_record)
        .filter(|s| !active_only.get() || s.status == "active" || s.status == "trial")
        .collect();
    let monthly_burn = calculate_monthly_burn(&snap.subscriptions);

    let mut out = view().gap_3().child(
        view()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                view()
                    .flex_row()
                    .gap_1p5()
                    .child(pill_tab("Active only", active_only.get(), move || {
                        active_only.set(true)
                    }))
                    .child(pill_tab("All states", !active_only.get(), move || {
                        active_only.set(false)
                    })),
            )
            .child(status_chip(
                format!("Burn: {}/mo", format_signed_cents(monthly_burn, "EUR")),
                true,
            )),
    );

    if summaries.is_empty() {
        out = out.child(empty_state(
            "No subscriptions listed",
            "Connect to your Mac or switch to All states.",
        ));
    } else {
        for sub in summaries {
            let is_active = sub.status == "active" || sub.status == "trial";
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
                            .child(text(sub.name).text_sm().font_weight(FontWeight::SEMIBOLD))
                            .child(status_chip(sub.status, is_active)),
                    )
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(caption(format!("{} · {}", sub.category, sub.cycle)))
                            .child(
                                text(format!(
                                    "{} ({}/mo)",
                                    format_signed_cents(sub.amount_cents, &sub.currency),
                                    format_signed_cents(
                                        sub.monthly_equivalent_cents,
                                        &sub.currency
                                    )
                                ))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM),
                            ),
                    ),
            );
        }
    }
    out.into_element()
}

fn review_section(snap: &Snapshot) -> nsh_core::element::AnyElement {
    let pending: Vec<&FinanceCandidate> = snap
        .candidates
        .iter()
        .filter(|c| c.state == "pending")
        .collect();
    let mut out = view().gap_2().child(caption(format!(
        "{} pending candidates ({} total imported)",
        pending.len(),
        snap.candidates.len()
    )));
    if pending.is_empty() {
        out = out.child(empty_state(
            "Review queue clear",
            "All imported finance candidates have been confirmed or rejected.",
        ));
    } else {
        for candidate in pending {
            let confirm_id = candidate.id.clone();
            let reject_id = candidate.id.clone();
            out = out.child(
                surface_card()
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
                                text(candidate.description.clone())
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .flex_1(),
                            )
                            .child(
                                text(format_signed_cents(
                                    candidate.amount_cents,
                                    &candidate.currency,
                                ))
                                .text_sm()
                                .font_weight(FontWeight::BOLD),
                            ),
                    )
                    .child(caption(format!(
                        "{} · {} → {} ({}%)",
                        candidate.booked_at,
                        candidate.source_account,
                        candidate.proposed_account,
                        candidate.confidence_basis_points / 100
                    )))
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .child(action("Confirm", move || {
                                review_candidate(confirm_id.clone(), "confirm")
                            }))
                            .child(action("Reject", move || {
                                review_candidate(reject_id.clone(), "reject")
                            })),
                    ),
            );
        }
    }
    out.into_element()
}

/// Phone Finance screen with Overview, Transactions, Subscriptions, and Review sub-tabs.
#[component]
pub fn FinanceScreen() -> impl IntoElement {
    let snap = snapshot(snapshots());
    let tab = signal(FinanceTab::Overview);
    let query = signal(String::new());
    let category = signal(None::<String>);
    let active_only = signal(true);

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
                        .child(heading("Finance"))
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
            let pending_count =
                snap.with(|s| s.candidates.iter().filter(|c| c.state == "pending").count());
            let review_label = if pending_count > 0 {
                format!("Review ({pending_count})")
            } else {
                "Review".into()
            };
            view()
                .flex_row()
                .flex_wrap()
                .gap_1p5()
                .child(pill_tab(
                    "Overview",
                    active == FinanceTab::Overview,
                    move || tab.set(FinanceTab::Overview),
                ))
                .child(pill_tab(
                    "Transactions",
                    active == FinanceTab::Transactions,
                    move || tab.set(FinanceTab::Transactions),
                ))
                .child(pill_tab(
                    "Subscriptions",
                    active == FinanceTab::Subscriptions,
                    move || tab.set(FinanceTab::Subscriptions),
                ))
                .child(pill_tab(
                    review_label,
                    active == FinanceTab::Review,
                    move || tab.set(FinanceTab::Review),
                ))
                .into_element()
        }))
        .child(dynamic(move || {
            (tab.get() == FinanceTab::Transactions).then(|| {
                input()
                    .test_id("finance-search")
                    .placeholder("Search description, account, or category…")
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
                FinanceTab::Overview => overview_section(&current),
                FinanceTab::Transactions => transactions_section(&current, query, category),
                FinanceTab::Subscriptions => subscriptions_section(&current, active_only),
                FinanceTab::Review => review_section(&current),
            };
            body.child(section).into_element()
        })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phone_finance::set_snapshot;
    use sjel_gpui_core::shell::{
        FinanceDashboardProjection, FinanceDashboardSummary, FinancePlanningView, FinanceSpendGoal,
        FinanceTransaction, PricePointRecord, StateChangeRecord, SubscriptionRecord,
    };

    #[test]
    fn phone_finance_switches_sub_tabs_filters_transactions_and_reviews_candidates() {
        use nsh_core::element::Root;
        use nsh_engine::EngineConfig;
        use nsh_testing::Headless;

        let mut dash = FinanceDashboardProjection::default();
        dash.summary = FinanceDashboardSummary {
            income_cents: 420_000,
            personal_spending_cents: 185_000,
            currency: "EUR".into(),
        };
        dash.categories = vec!["Food".into(), "Housing".into()];
        dash.planning = Some(FinancePlanningView {
            spend_goals: vec![FinanceSpendGoal {
                id: "goal-food".into(),
                label: "Groceries & Dining".into(),
                start: "2026-11-01".into(),
                end: "2026-11-30".into(),
                currency: "EUR".into(),
                target_cents: 60_000,
                spent_cents: 42_000,
                remaining_cents: 18_000,
                days_left: 11,
                needed_per_day_cents: 1_636,
                met: true,
            }],
        });
        dash.transactions = vec![FinanceTransaction {
            id: "tx-1".into(),
            date: "2026-11-18".into(),
            description: "Organic Market Berlin".into(),
            kind: "expense".into(),
            account: "Checking".into(),
            category: "Food".into(),
            amount_cents: -4_250,
            currency: "EUR".into(),
            source_id: None,
            trip_id: None,
            purpose: None,
            shared_cents: 0,
            cash_amount_cents: 0,
            reimbursement_for: None,
        }];
        let subs = vec![SubscriptionRecord {
            id: "sub-1".into(),
            name: "Cloud Storage Pro".into(),
            category: Some("Tech".into()),
            value_rating: None,
            prices: vec![PricePointRecord {
                valid_from: "2026-01-01".into(),
                amount_cents: 999,
                currency: "EUR".into(),
                cycle: "monthly".into(),
                plan: None,
            }],
            states: vec![StateChangeRecord {
                effective: "2026-01-01".into(),
                state: "active".into(),
                note: String::new(),
            }],
        }];
        let candidates = vec![FinanceCandidate {
            id: "cand:1".into(),
            fingerprint: "fp-1".into(),
            booked_at: "2026-11-18".into(),
            description: "Bakery Checkpoint".into(),
            amount_cents: -850,
            currency: "EUR".into(),
            source_account: "Checking".into(),
            source_reference: None,
            proposed_account: "expenses:food".into(),
            confidence_basis_points: 9500,
            state: "pending".into(),
            transfer_match_ids: Vec::new(),
        }];
        set_snapshot(dash, subs, candidates);

        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(Root::of::<FinanceScreen>())
        })
        .expect("FinanceScreen mounts in headless NSH");

        app.wait_for("text:Finance").unwrap();
        app.wait_for("text:Groceries & Dining").unwrap();
        app.wait_for("text:Organic Market Berlin").unwrap();

        app.press("text:Transactions").unwrap();
        app.wait_for("text:Showing 1 of 1 transactions").unwrap();

        app.press("text:Subscriptions").unwrap();
        app.wait_for("text:Cloud Storage Pro").unwrap();

        app.press("text:Review (1)").unwrap();
        app.wait_for("text:Bakery Checkpoint").unwrap();
        app.press("text:Confirm").unwrap();
        app.wait_for("text:Review queue clear").unwrap();
    }
}
