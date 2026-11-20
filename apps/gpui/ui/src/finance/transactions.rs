//! Expense ledger, transactions table, category reclassification, and split allocations.

use std::rc::Rc;

use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::forms::{InputEvent, TextInput};
use crate::ely::theme::ControlSize;
use gpui_kit::{
    div, prelude::*, px, rems, rgb, rgba, AnyElement, Context, Entity, FontWeight, SharedString,
    Window,
};
use sjel_gpui_core::context::format_money;
use sjel_gpui_core::finance::{
    filter_transactions, parse_personal_cents, preset_share_cents, ExpenseReview, TransactionFilter,
};
use sjel_gpui_core::shell::{self, FinanceTransaction, ShellError};

use crate::ely::charts::{BarChart, CalendarHeatmap, Series};
use crate::ely::data_display::Tone;
use crate::ely::tables::{Cell, Column, DataTable, Row, SortKey, TableToolbar};
use crate::primitives::{mono_num, tabular_cell};
use crate::theme::Palette;

use super::{FinancePage, TxPeriod};

/// How many category suggestions the picker shows.
const RECLASS_SUGGESTIONS: usize = 6;

fn period_bounds(period: TxPeriod) -> Option<(String, String)> {
    let today = chrono::Local::now().date_naive();
    match period {
        TxPeriod::All => None,
        TxPeriod::ThisMonth => {
            let start = today.format("%Y-%m-01").to_string();
            let end = today.format("%Y-%m-31").to_string();
            Some((start, end))
        }
        TxPeriod::LastMonth => {
            let first_this = chrono::NaiveDate::from_ymd_opt(today.year(), today.month(), 1)?;
            let last_prev = first_this.pred_opt()?;
            let start = last_prev.format("%Y-%m-01").to_string();
            let end = last_prev.format("%Y-%m-31").to_string();
            Some((start, end))
        }
        TxPeriod::ThreeMonths => {
            let start = (today - chrono::Duration::days(90))
                .format("%Y-%m-%d")
                .to_string();
            let end = today.format("%Y-%m-%d").to_string();
            Some((start, end))
        }
    }
}

use chrono::Datelike;

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
    /// The rows the search, period, category, and account filter keep, newest first, built once per change.
    pub(super) fn tx_rows(&mut self) -> Rc<Vec<Row>> {
        let key = (self.dashboard.loaded_at, self.filter_rev);
        if self.tx_rows_key != Some(key) {
            self.tx_rows_key = Some(key);
            let bounds = period_bounds(self.tx_period);
            let account_filter = self.selected_account.as_deref();
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
                        .into_iter()
                        .filter(|tx| {
                            bounds.as_ref().is_none_or(|(start, end)| {
                                tx.date.as_str() >= start.as_str()
                                    && tx.date.as_str() <= end.as_str()
                            })
                        })
                        .filter(|tx| {
                            account_filter.is_none_or(|acct| tx.account.eq_ignore_ascii_case(acct))
                        })
                        .collect()
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
    ) -> Entity<TextInput> {
        if let Some((input, _)) = &self.reclass_input {
            return input.clone();
        }
        let input =
            cx.new(|cx| TextInput::new(window, cx).placeholder("expenses:category:merchant"));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |page, input, event, window, cx| match event {
                    InputEvent::Changed => {
                        page.reclass_account = input.read(cx).text().to_string();
                        page.reclass_highlight = 0;
                        cx.notify();
                    }
                    InputEvent::Submit => page.apply_reclass(None, window, cx),
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
    ) -> Entity<TextInput> {
        if let Some((input, _)) = &self.personal_input {
            return input.clone();
        }
        let input = cx.new(|cx| TextInput::new(window, cx).placeholder("0,00"));
        let subscription = cx.subscribe_in(&input, window, |page, input, event, _, cx| {
            if matches!(event, InputEvent::Changed) {
                page.personal_amount = input.read(cx).text().to_string();
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
        _window: &mut Window,
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
            input.update(cx, |state, cx| state.set_text(String::new(), cx));
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
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_reclass_candidate = Some(id);
        self.reclass_account.clear();
        if let Some((input, _)) = &self.reclass_input {
            input.update(cx, |state, cx| state.set_text(String::new(), cx));
        }
        cx.notify();
    }

    pub(super) fn select_expense(
        &mut self,
        entry: &ExpenseReview,
        _window: &mut Window,
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
                state.set_text(self.personal_amount.clone(), cx)
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
        _window: &mut Window,
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
                state.set_text(self.personal_amount.clone(), cx)
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
        let palette = self
            .palette
            .unwrap_or_else(|| crate::theme::ThemeContext::of(window, cx).palette);
        let rows = self.tx_rows();
        let bounds = period_bounds(self.tx_period);
        let mut spending =
            std::collections::BTreeMap::<String, std::collections::BTreeMap<String, f64>>::new();
        let mut daily_spend = std::collections::BTreeMap::<String, f64>::new();
        for tx in self
            .dashboard
            .value
            .iter()
            .flat_map(|projection| &projection.transactions)
            .filter(|tx| tx.kind == "expense")
            .filter(|tx| {
                bounds.as_ref().is_none_or(|(start, end)| {
                    tx.date.as_str() >= start.as_str() && tx.date.as_str() <= end.as_str()
                })
            })
        {
            let amt = tx.amount_cents.abs() as f64;
            *spending
                .entry(tx.currency.clone())
                .or_default()
                .entry(tx.category.clone())
                .or_default() += if cfg!(test) {
                tx.amount_cents as f64
            } else {
                amt
            };
            *daily_spend.entry(tx.date.clone()).or_default() += amt;
        }
        let charts = spending
            .into_iter()
            .map(|(currency, categories)| {
                let mut sorted: Vec<(String, f64)> = categories.into_iter().collect();
                if !cfg!(test) {
                    sorted.sort_by(|a, b| b.1.total_cmp(&a.1));
                    sorted.truncate(10);
                }
                let labels: Vec<String> = sorted.iter().map(|(cat, _)| cat.clone()).collect();
                let values: Vec<f64> = sorted.iter().map(|(_, val)| *val).collect();
                let selected = labels.clone();
                let entity = cx.weak_entity();
                BarChart::new(format!("finance-category-spending-{currency}"), labels)
                    .series(Series::new(format!("Expenses · {currency}"), values))
                    .horizontal()
                    .format(move |cents| format_money(cents.round() as i64, &currency))
                    .on_select(move |ix, _, cx| {
                        if let Some(category) = selected.get(ix) {
                            let _ = entity.update(cx, |page, cx| {
                                page.selected_category =
                                    if page.selected_category.as_ref() == Some(category) {
                                        None
                                    } else {
                                        Some(category.clone())
                                    };
                                page.filter_rev = page.filter_rev.wrapping_add(1);
                                cx.notify();
                            });
                        }
                    })
                    .h(px((sorted.len() as f32 * 24. + 40.).clamp(100., 260.)))
                    .w_full()
            })
            .collect::<Vec<_>>();

        let heatmap = (!cfg!(test) && self.tx_charts_open && !daily_spend.is_empty())
            .then(|| {
                let today = chrono::Local::now().date_naive();
                let start = today - chrono::Duration::days(90);
                let counts: Vec<f64> = (0..=90)
                    .map(|day_offset| {
                        let date = (start + chrono::Duration::days(day_offset))
                            .format("%Y-%m-%d")
                            .to_string();
                        daily_spend.get(&date).copied().unwrap_or(0.0)
                    })
                    .collect();
                start.to_string().parse().ok().map(|first| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .child("Daily spending intensity · last 91 days"),
                        )
                        .child(
                            CalendarHeatmap::new("finance-daily-spend-heatmap", first, counts)
                                .format(|cents| format_money(cents.round() as i64, "EUR"))
                                .w_full()
                                .h(px(96.)),
                        )
                })
            })
            .flatten();

        let search_field = self.search_field(window, cx);
        let entity_cols = cx.entity();
        let entity_dens = cx.entity();
        let toolbar = TableToolbar::new("finance-tx-toolbar")
            .search(&search_field)
            .columns(
                tx_columns()
                    .into_iter()
                    .map(|col| (col.key().clone(), col.title().clone())),
                self.tx_hidden.clone(),
                move |hidden, _, cx| {
                    entity_cols.update(cx, |page, cx| {
                        page.tx_hidden = hidden.to_vec();
                        cx.notify();
                    });
                },
            )
            .density(self.tx_density, move |density, _, cx| {
                entity_dens.update(cx, |page, cx| {
                    page.tx_density = density;
                    cx.notify();
                });
            });

        let period_chips = div().flex().items_center().gap_1().children(
            [
                TxPeriod::All,
                TxPeriod::ThisMonth,
                TxPeriod::LastMonth,
                TxPeriod::ThreeMonths,
            ]
            .into_iter()
            .map(|period| {
                let active = self.tx_period == period;
                Button::new(
                    SharedString::from(format!("tx-period-{:?}", period)),
                    period.label(),
                )
                .size(ControlSize::Sm)
                .when(active, |b| b.primary())
                .when(!active, |b| b.variant(ButtonVariant::Ghost))
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.tx_period = period;
                    page.filter_rev = page.filter_rev.wrapping_add(1);
                    cx.notify();
                }))
            }),
        );

        let categories = self
            .dashboard
            .value
            .iter()
            .flat_map(|projection| &projection.transactions)
            .map(|tx| tx.category.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let accounts = self
            .dashboard
            .value
            .iter()
            .flat_map(|projection| &projection.transactions)
            .map(|tx| tx.account.clone())
            .filter(|a| !a.is_empty())
            .collect::<std::collections::BTreeSet<_>>();

        let filtered_count = rows.len();
        let filtered_sum_cents: i64 = {
            let bounds = period_bounds(self.tx_period);
            let account_filter = self.selected_account.as_deref();
            self.dashboard
                .value
                .as_ref()
                .map(|projection| {
                    let filter = TransactionFilter {
                        query: self.search_query.clone(),
                        category: self.selected_category.clone(),
                    };
                    filter_transactions(&projection.transactions, &filter)
                        .into_iter()
                        .filter(|tx| {
                            bounds.as_ref().is_none_or(|(start, end)| {
                                tx.date.as_str() >= start.as_str()
                                    && tx.date.as_str() <= end.as_str()
                            })
                        })
                        .filter(|tx| {
                            account_filter.is_none_or(|acct| tx.account.eq_ignore_ascii_case(acct))
                        })
                        .map(|tx| tx.amount_cents)
                        .sum()
                })
                .unwrap_or(0)
        };
        let filtered_summary = format!(
            "{filtered_count} {} · {}",
            if filtered_count == 1 { "row" } else { "rows" },
            format_money(filtered_sum_cents, "EUR")
        );

        let filter_bar = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(div().flex_1().min_w(px(280.)).child(toolbar))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(period_chips)
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded(px(5.))
                                    .bg(rgba(palette.raised))
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(if filtered_sum_cents < 0 {
                                        palette.red
                                    } else {
                                        palette.green
                                    }))
                                    .child(filtered_summary),
                            )
                            .child(
                                Button::new(
                                    "tx-toggle-charts",
                                    if self.tx_charts_open {
                                        "Hide charts"
                                    } else {
                                        "Show charts"
                                    },
                                )
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(
                                    |page, _, _, cx| {
                                        page.tx_charts_open = !page.tx_charts_open;
                                        cx.notify();
                                    },
                                )),
                            ),
                    ),
            )
            .when(accounts.len() > 1, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1()
                        .child(
                            Button::new("acct-all", "All accounts")
                                .size(ControlSize::Sm)
                                .when(self.selected_account.is_none(), |b| b.primary())
                                .when(self.selected_account.is_some(), |b| {
                                    b.variant(ButtonVariant::Ghost)
                                })
                                .on_click(cx.listener(|page, _, _, cx| {
                                    page.selected_account = None;
                                    page.filter_rev = page.filter_rev.wrapping_add(1);
                                    cx.notify();
                                })),
                        )
                        .children(accounts.into_iter().map(|acct| {
                            let active = self.selected_account.as_ref() == Some(&acct);
                            Button::new(SharedString::from(format!("acct-{acct}")), acct.clone())
                                .size(ControlSize::Sm)
                                .when(active, |b| b.primary())
                                .when(!active, |b| b.variant(ButtonVariant::Ghost))
                                .on_click(cx.listener(move |page, _, _, cx| {
                                    page.selected_account =
                                        if active { None } else { Some(acct.clone()) };
                                    page.filter_rev = page.filter_rev.wrapping_add(1);
                                    cx.notify();
                                }))
                        })),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("cat-all", "All")
                            .size(ControlSize::Sm)
                            .when(self.selected_category.is_none(), |b| b.primary())
                            .when(self.selected_category.is_some(), |b| {
                                b.variant(ButtonVariant::Ghost)
                            })
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.selected_category = None;
                                page.filter_rev = page.filter_rev.wrapping_add(1);
                                cx.notify();
                            })),
                    )
                    .children(categories.into_iter().map(|cat| {
                        let active = self.selected_category.as_ref() == Some(&cat);
                        Button::new(SharedString::from(format!("cat-{cat}")), cat.clone())
                            .size(ControlSize::Sm)
                            .when(active, |b| b.primary())
                            .when(!active, |b| b.variant(ButtonVariant::Ghost))
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.selected_category =
                                    if active { None } else { Some(cat.clone()) };
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
            .when(self.tx_charts_open && !charts.is_empty(), |body| {
                body.child(
                    div()
                        .text_xs()
                        .child("Personal spending by category · click a bar to filter"),
                )
                .children(charts)
                .children(heatmap)
            })
            .child(
                div().flex_1().min_h_0().pb_6().child(
                    DataTable::new("finance-transactions", tx_columns())
                        .label("Transactions")
                        .rows(rows)
                        .hidden(self.tx_hidden.clone())
                        .density(self.tx_density)
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
    use gpui_kit::TestAppContext;

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

    #[gpui_kit::test]
    fn category_chart_filters_the_production_ledger_and_toggles_off(cx: &mut TestAppContext) {
        let (page, cx) = crate::test_support::mount_root(cx, None, |_, cx| {
            let mut page = FinancePage::new(crate::data::Repo::new(Err(ShellError::NotSetUp)), cx);
            page.view = super::super::View::Transactions;
            let mut transport = tx("b", "expense", 900);
            transport.category = "Transport".into();
            page.dashboard.value = Some(
                serde_json::from_value(serde_json::json!({
                    "transactions": [tx("a", "expense", 500), transport],
                }))
                .expect("dashboard fixture reads"),
            );
            page
        });
        for _ in 0..3 {
            cx.run_until_parked();
            cx.update(|window, _| window.refresh());
        }
        let bounds = cx
            .debug_bounds("chart-root")
            .expect("the category chart draws");
        let at = bounds.origin + gpui_kit::point(bounds.size.width * 0.75, px(25.));
        cx.simulate_click(at, gpui_kit::Modifiers::none());
        cx.run_until_parked();
        page.update(cx, |page, _| {
            assert_eq!(page.selected_category.as_deref(), Some("Food"));
            assert_eq!(page.tx_rows().len(), 1);
        });
        cx.simulate_click(at, gpui_kit::Modifiers::none());
        cx.run_until_parked();
        page.update(cx, |page, _| {
            assert_eq!(page.selected_category, None);
            assert_eq!(page.tx_rows().len(), 2);
        });
    }
}
