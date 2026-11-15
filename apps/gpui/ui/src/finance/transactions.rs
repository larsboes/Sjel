//! Expense ledger, transactions table, category reclassification, and split allocations.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rems, rgb, rgba, AnyElement, Context, Entity, FontWeight, SharedString,
    Window,
};
use sjel_gpui_core::context::format_money;
use sjel_gpui_core::finance::{
    filter_transactions, parse_personal_cents, preset_share_cents, ExpenseReview, TransactionFilter,
};
use sjel_gpui_core::shell::{self, FinanceTransaction, ShellError};

use crate::ely::data_display::Tone;
use crate::ely::tables::{Cell, Column, DataTable, Row, SortKey};
use crate::primitives::{mono_num, tabular_cell};
use crate::theme::Palette;

use super::FinancePage;

/// How many category suggestions the picker shows.
const RECLASS_SUGGESTIONS: usize = 6;

fn amount_color(tx: &FinanceTransaction, palette: Palette) -> u32 {
    if tx.kind == "expense" || tx.amount_cents < 0 {
        palette.red
    } else if tx.kind == "income" {
        palette.green
    } else {
        palette.muted
    }
}

/// The transactions table's columns. Widths in rems follow the app's text size.
fn tx_columns() -> [Column; 5] {
    [
        Column::new("date", "DATE").width(rems(6.)).muted(),
        Column::new("description", "DESCRIPTION"),
        Column::new("category", "CATEGORY").width(rems(8.)),
        Column::new("account", "ACCOUNT").width(rems(10.)).muted(),
        Column::new("amount", "AMOUNT").width(rems(7.)).end(),
    ]
}

/// One table row for `tx`. The amount keeps Sjel's money format and sorts by cents.
fn tx_row(tx: &FinanceTransaction) -> Row {
    let tone = if tx.kind == "expense" || tx.amount_cents < 0 {
        Tone::Danger
    } else if tx.kind == "income" {
        Tone::Success
    } else {
        Tone::Neutral
    };
    Row::new(
        tx.id.clone(),
        [
            Cell::Text(tx.date.clone().into()),
            Cell::Text(tx.description.clone().into()),
            Cell::Tag(tx.category.clone().into(), Tone::Neutral),
            Cell::Text(tx.account.clone().into()),
            Cell::Figure(
                format_money(tx.amount_cents, &tx.currency).into(),
                tx.amount_cents as f64,
                tone,
            ),
        ],
    )
}

pub(crate) fn render_tx_row(
    tx: &FinanceTransaction,
    date_w: f32,
    cat_w: f32,
    acct_w: f32,
    amt_w: f32,
    palette: Palette,
) -> AnyElement {
    let amount_color = amount_color(tx, palette);
    let amount_text = format_money(tx.amount_cents, &tx.currency);

    div()
        .flex()
        .items_center()
        .gap_3()
        .px_3()
        .py_2()
        .rounded(px(6.))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgba(palette.surface))
        .child(
            div()
                .w(px(date_w))
                .flex_none()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .child(mono_num(tx.date.clone(), palette.muted)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.text))
                .child(tx.description.clone()),
        )
        .child(
            div().w(px(cat_w)).flex_none().flex().items_center().child(
                div()
                    .max_w_full()
                    .truncate()
                    .px_1p5()
                    .py(px(1.))
                    .rounded(px(3.))
                    .bg(rgba(palette.raised))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(tx.category.clone()),
            ),
        )
        .child(
            div()
                .w(px(acct_w))
                .flex_none()
                .truncate()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(tx.account.clone()),
        )
        .child(tabular_cell(amount_text, px(amt_w), amount_color))
        .into_any_element()
}

impl FinancePage {
    /// The rows the search and category filter keep, newest first, built once per change.
    pub(super) fn tx_rows(&mut self) -> Rc<Vec<Row>> {
        let key = (self.dashboard.loaded_at, self.filter_rev);
        if self.tx_rows_key != Some(key) {
            self.tx_rows_key = Some(key);
            let mut kept: Vec<&FinanceTransaction> = self
                .dashboard
                .value
                .as_ref()
                .map(|projection| {
                    let filter = TransactionFilter {
                        query: self.search_query.clone(),
                        category: self.selected_category.clone(),
                    };
                    filter_transactions(&projection.transactions, &filter)
                })
                .unwrap_or_default();
            kept.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.id.cmp(&b.id)));
            self.tx_rows = Rc::new(kept.into_iter().map(tx_row).collect());
        }
        self.tx_rows.clone()
    }

    pub(super) fn reclass_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.reclass_input {
            return input.clone();
        }
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("expenses:category:merchant"));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |page, input, event, window, cx| match event {
                    InputEvent::Change => {
                        page.reclass_account = input.read(cx).value().to_string();
                        page.reclass_highlight = 0;
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => page.apply_reclass(None, window, cx),
                    _ => {}
                },
            );
        self.reclass_input = Some((input.clone(), subscription));
        input
    }

    pub(super) fn personal_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.personal_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("0,00"));
        let subscription = cx.subscribe_in(&input, window, |page, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                page.personal_amount = input.read(cx).value().to_string();
                cx.notify();
            }
        });
        self.personal_input = Some((input.clone(), subscription));
        input
    }

    /// Known expense accounts matching the typed text, best first.
    pub(super) fn reclass_suggestions(&self) -> Vec<String> {
        let categories = self
            .dashboard
            .value
            .as_ref()
            .map(|dashboard| dashboard.categories.as_slice())
            .unwrap_or_default();
        sjel_gpui_core::finance::category_suggestions(
            categories,
            &self.reclass_account,
            RECLASS_SUGGESTIONS,
        )
    }

    /// Moves the highlighted suggestion by `step`, staying inside the list.
    pub(super) fn move_reclass_highlight(&mut self, step: isize, cx: &mut Context<Self>) {
        let count = self.reclass_suggestions().len();
        if count > 0 {
            self.reclass_highlight = self
                .reclass_highlight
                .saturating_add_signed(step)
                .min(count - 1);
            cx.notify();
        }
    }

    /// Applies `account`, else the highlighted suggestion, else the typed text, to the selected
    /// expense (the first uncategorized one when none is selected), clears the field and moves
    /// to the next expense.
    pub(super) fn apply_reclass(
        &mut self,
        account: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let account = account
            .or_else(|| {
                self.reclass_suggestions()
                    .get(self.reclass_highlight)
                    .cloned()
            })
            .unwrap_or_else(|| self.reclass_account.trim().to_owned());
        let id = self.selected_reclass_candidate.clone().or_else(|| {
            self.import_derived
                .uncategorized
                .first()
                .map(|candidate| candidate.id.clone())
        });
        let Some(id) = id else {
            return;
        };
        if let Some((input, _)) = &self.reclass_input {
            input.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        }
        self.reclass_highlight = 0;
        self.reclassify(id, account, cx);
    }

    pub(super) fn reclassify_selected(&mut self, cx: &mut Context<Self>) {
        let (Some(id), account) = (
            self.selected_reclass_candidate.clone(),
            self.reclass_account.trim().to_owned(),
        ) else {
            return;
        };
        self.reclassify(id, account, cx);
    }

    fn reclassify(&mut self, id: String, account: String, cx: &mut Context<Self>) {
        let uncategorized = &self.import_derived.uncategorized;
        let next = uncategorized
            .iter()
            .position(|candidate| candidate.id == id)
            .and_then(|index| uncategorized.get(index + 1))
            .map(|candidate| candidate.id.clone());
        if !account.starts_with("expenses:")
            || account.split(':').any(|segment| segment == "uncategorized")
        {
            self.notice = Some("Choose a categorized expenses account.".into());
            cx.notify();
            return;
        }
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let work = shell::run(async move {
            shell
                .finance_reclassify_candidates(vec![(id, account)])
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                match result {
                    Ok(value) => {
                        page.notice = Some(format!(
                            "{} candidate reclassified.",
                            value["reclassified"].as_u64().unwrap_or_default()
                        ));
                        page.selected_reclass_candidate = next;
                    }
                    Err(error) => {
                        page.notice = Some(error.to_string());
                        page.selected_reclass_candidate = None;
                    }
                }
                page.reclass_account.clear();
                page.load(cx);
            });
        })
        .detach();
    }

    pub(super) fn select_reclass_candidate(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_reclass_candidate = Some(id);
        self.reclass_account.clear();
        if let Some((input, _)) = &self.reclass_input {
            input.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        }
        cx.notify();
    }

    pub(super) fn select_expense(
        &mut self,
        entry: &ExpenseReview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_expense = Some(entry.candidate.id.clone());
        let initial_share = entry
            .transaction
            .as_ref()
            .map_or(entry.total_cents, |transaction| {
                transaction.amount_cents.saturating_abs()
            });
        self.personal_amount =
            format!("{},{:02}", initial_share / 100, (initial_share % 100).abs());
        if let Some((input, _)) = &self.personal_input {
            input.update(cx, |state, cx| {
                state.set_value(self.personal_amount.clone(), window, cx)
            });
        }
        if let Some(transaction) = &entry.transaction {
            self.allocation_purpose = transaction
                .purpose
                .clone()
                .unwrap_or_else(|| "day_to_day".into());
            self.selected_trip_id = transaction.trip_id.clone().unwrap_or_default();
        } else {
            self.allocation_purpose = "day_to_day".into();
            self.selected_trip_id.clear();
        }
        cx.notify();
    }

    pub(super) fn set_personal_share(
        &mut self,
        percent: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.selected_expense.as_deref() else {
            return;
        };
        let Some(candidate) = self
            .candidates
            .value
            .as_ref()
            .and_then(|items| items.iter().find(|candidate| candidate.id == id))
        else {
            return;
        };
        let cents = preset_share_cents(candidate.amount_cents.saturating_abs(), f64::from(percent));
        self.personal_amount = format!("{},{:02}", cents / 100, (cents % 100).abs());
        if let Some((input, _)) = &self.personal_input {
            input.update(cx, |state, cx| {
                state.set_value(self.personal_amount.clone(), window, cx)
            });
        }
        cx.notify();
    }

    pub(super) fn save_allocation(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_expense.clone() else {
            return;
        };
        let Some(cents) = parse_personal_cents(&self.personal_amount) else {
            self.notice = Some("Enter a non-negative personal share.".into());
            cx.notify();
            return;
        };
        let Some(candidate) = self
            .candidates
            .value
            .as_ref()
            .and_then(|items| items.iter().find(|candidate| candidate.id == id))
        else {
            return;
        };
        let total = candidate.amount_cents.saturating_abs();
        if cents > total
            || (self.allocation_purpose == "trip" && self.selected_trip_id.trim().is_empty())
        {
            self.notice = Some(
                "Personal share must be between zero and the total; trip purpose needs a plan."
                    .into(),
            );
            cx.notify();
            return;
        }
        let purpose = self.allocation_purpose.clone();
        let trip = (purpose == "trip").then(|| self.selected_trip_id.trim().to_owned());
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let work = shell::run(async move {
            shell
                .finance_allocate_expense(&id, cents, &purpose, trip.as_deref())
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                match result {
                    Ok(_) => page.notice = Some("Expense purpose and personal share saved.".into()),
                    Err(error) => page.notice = Some(error.to_string()),
                }
                page.selected_expense = None;
                page.load(cx);
            });
        })
        .detach();
    }

    pub(super) fn link_reimbursement(&mut self, cx: &mut Context<Self>) {
        let (Some(inflow), Some(target)) = (
            self.selected_inflow.clone(),
            self.reimbursement_target.clone(),
        ) else {
            return;
        };
        let candidates = self.candidates.value.as_deref().unwrap_or_default();
        let Some(income) = candidates.iter().find(|candidate| candidate.id == inflow) else {
            return;
        };
        let Some(expense) = self.dashboard.value.as_ref().and_then(|dashboard| {
            dashboard
                .shared_expenses
                .iter()
                .find(|expense| expense.candidate_id == target)
        }) else {
            return;
        };
        if income.currency != expense.currency || income.amount_cents > expense.outstanding_cents {
            self.notice =
                Some("Repayment must use the same currency and not exceed the open amount.".into());
            cx.notify();
            return;
        }
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let work =
            shell::run(async move { shell.finance_link_reimbursement(&inflow, &target).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                match result {
                    Ok(_) => page.notice = Some("Repayment linked to the shared expense.".into()),
                    Err(error) => page.notice = Some(error.to_string()),
                }
                page.selected_inflow = None;
                page.reimbursement_target = None;
                page.load(cx);
            });
        })
        .detach();
    }

    pub(super) fn render_transactions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = self.tx_rows();

        let search_box = div()
            .w(px(280.))
            .child(Input::new(&self.search_field(window, cx)).small());

        // Category filter buttons
        let categories = ["Food", "Transport", "Shopping", "Tech", "Travel", "Health"];
        let filter_bar = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(search_box)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("cat-all")
                            .xsmall()
                            .label("All")
                            .when(self.selected_category.is_none(), |b| b.primary())
                            .when(self.selected_category.is_some(), |b| b.ghost())
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.selected_category = None;
                                page.filter_rev = page.filter_rev.wrapping_add(1);
                                cx.notify();
                            })),
                    )
                    .children(categories.map(|cat| {
                        let active = self.selected_category.as_deref() == Some(cat);
                        Button::new(SharedString::from(format!("cat-{cat}")))
                            .xsmall()
                            .label(cat)
                            .when(active, |b| b.primary())
                            .when(!active, |b| b.ghost())
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.selected_category =
                                    if active { None } else { Some(cat.to_string()) };
                                page.filter_rev = page.filter_rev.wrapping_add(1);
                                cx.notify();
                            }))
                    })),
            );

        div()
            .flex()
            .flex_col()
            .gap_2()
            .size_full()
            .child(filter_bar)
            .child(
                div().flex_1().min_h_0().pb_6().child(
                    DataTable::new("finance-transactions", tx_columns())
                        .rows(rows)
                        .sorts([SortKey {
                            column: "date".into(),
                            rising: false,
                        }])
                        .virtualized()
                        .track_scroll(&self.tx_scroll)
                        .empty("No transactions match the current filter.")
                        .size_full(),
                ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, VisualTestContext};

    fn tx(id: &str, kind: &str, cents: i64) -> FinanceTransaction {
        serde_json::from_value(serde_json::json!({
            "id": id, "date": "2026-11-14", "description": "Coffee", "kind": kind,
            "account": "Giro", "category": "Food", "amount_cents": cents, "currency": "EUR",
        }))
        .expect("a transaction reads")
    }

    #[test]
    fn a_row_keeps_sjels_money_format_sorts_by_cents_and_tones_by_kind() {
        for (kind, cents, tone) in [
            ("expense", 1234, Tone::Danger),
            ("income", 1234, Tone::Success),
            ("transfer", 1234, Tone::Neutral),
            ("transfer", -1234, Tone::Danger),
        ] {
            let expected = Row::new(
                "t1",
                [
                    Cell::Text("2026-11-14".into()),
                    Cell::Text("Coffee".into()),
                    Cell::Tag("Food".into(), Tone::Neutral),
                    Cell::Text("Giro".into()),
                    Cell::Figure(format_money(cents, "EUR").into(), cents as f64, tone),
                ],
            );
            assert_eq!(tx_row(&tx("t1", kind, cents)), expected, "{kind} {cents}");
        }
    }

    struct Ledger(Rc<Vec<Row>>);

    impl gpui_kit::Render for Ledger {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            DataTable::new("finance-transactions", tx_columns())
                .rows(self.0.clone())
                .virtualized()
                .empty("No transactions match the current filter.")
                .size_full()
        }
    }

    #[gpui_kit::test]
    fn the_ledger_draws_on_elys_table(cx: &mut TestAppContext) {
        cx.update(crate::ely::init_for_tests);
        let rows = Rc::new(vec![
            tx_row(&tx("a", "expense", -500)),
            tx_row(&tx("b", "income", 900)),
        ]);
        let (_, cx): (_, &mut VisualTestContext) = cx.add_window_view(|_, _| Ledger(rows));
        cx.run_until_parked();
    }
}
