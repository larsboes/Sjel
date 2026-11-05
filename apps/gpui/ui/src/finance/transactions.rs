//! Expense ledger, transactions table, category reclassification, and split allocations.

use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, FontWeight, SharedString, Window,
};
use sjel_gpui_core::context::format_money;
use sjel_gpui_core::finance::{
    filter_transactions, parse_personal_cents, preset_share_cents, ExpenseReview,
    TransactionFilter,
};
use sjel_gpui_core::shell::{self, FinanceTransaction, ShellError};

use crate::primitives::{mono_num, tone_chip, virtual_list};
use crate::theme::Palette;

use super::FinancePage;

#[derive(Default)]
pub(crate) struct TxDerived {
    pub(super) key: Option<(Option<Instant>, u64)>,
    pub(super) indices: Vec<usize>,
}

pub(crate) fn render_tx_row(tx: &FinanceTransaction, palette: Palette) -> AnyElement {
    let is_expense = tx.kind == "expense" || tx.amount_cents < 0;
    let amount_color = if is_expense {
        palette.red
    } else if tx.kind == "income" {
        palette.green
    } else {
        palette.muted
    };

    let amount_text = format_money(tx.amount_cents, &tx.currency);

    div()
        .flex()
        .items_center()
        .justify_between()
        .p_2p5()
        .rounded(px(6.))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgba(palette.surface))
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .min_w_0()
                .child(
                    div()
                        .w(px(80.))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .child(mono_num(tx.date.clone(), palette.muted)),
                )
                .child(
                    div()
                        .px_1()
                        .py(px(1.))
                        .rounded(px(3.))
                        .bg(rgba(palette.raised))
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(tx.category.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(palette.text))
                        .child(tx.description.clone()),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(tx.account.clone()),
                )
                .child(tone_chip(amount_text, amount_color)),
        )
        .into_any_element()
}


impl FinancePage {
    pub(super) fn sync_tx_derived(&mut self) {
        let key = (self.dashboard.loaded_at, self.filter_rev);
        if self.tx_derived.key == Some(key) {
            return;
        }
        let mut indices = Vec::new();
        if let Some(projection) = self.dashboard.value.as_ref() {
            let filter = TransactionFilter {
                query: self.search_query.clone(),
                category: self.selected_category.clone(),
            };
            let filtered = filter_transactions(&projection.transactions, &filter);
            let base = projection.transactions.as_ptr() as usize;
            let stride = std::mem::size_of::<FinanceTransaction>();
            if stride > 0 {
                indices = filtered
                    .into_iter()
                    .map(|tx| ((tx as *const FinanceTransaction as usize) - base) / stride)
                    .collect();
            }
        }
        self.tx_list.reset(indices.len());
        self.tx_derived = TxDerived {
            key: Some(key),
            indices,
        };
    }


    pub(super) fn reclass_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((input, _)) = &self.reclass_input {
            return input.clone();
        }
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("expenses:category:merchant"));
        let subscription = cx.subscribe_in(&input, window, |page, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                page.reclass_account = input.read(cx).value().to_string();
                cx.notify();
            }
        });
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

    pub(super) fn reclassify_selected(&mut self, cx: &mut Context<Self>) {
        let (Some(id), account) = (
            self.selected_reclass_candidate.clone(),
            self.reclass_account.trim().to_owned(),
        ) else {
            return;
        };
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
                        ))
                    }
                    Err(error) => page.notice = Some(error.to_string()),
                }
                page.selected_reclass_candidate = None;
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

    pub(super) fn set_personal_share(&mut self, percent: u32, window: &mut Window, cx: &mut Context<Self>) {
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
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_tx_derived();
        let is_empty = self.tx_derived.indices.is_empty();

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

        let table_body = if is_empty {
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .py_12()
                .rounded(px(8.))
                .border_1()
                .border_color(rgba(palette.border))
                .bg(rgba(palette.surface))
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("No transactions match the current filter.")
                .into_any_element()
        } else {
            let entity = cx.entity();
            virtual_list(
                "finance-transactions-list",
                &self.tx_list,
                move |ix, _window, cx| {
                    entity.update(cx, |page, _cx| {
                        let Some(&tx_ix) = page.tx_derived.indices.get(ix) else {
                            return div().into_any_element();
                        };
                        let Some(tx) = page
                            .dashboard
                            .value
                            .as_ref()
                            .and_then(|d| d.transactions.get(tx_ix))
                        else {
                            return div().into_any_element();
                        };
                        div()
                            .pb_1()
                            .child(render_tx_row(tx, palette))
                            .into_any_element()
                    })
                },
            )
        };

        div()
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            .child(filter_bar)
            .child(
                div()
                    .id("finance-transactions-scroll")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .pb_6()
                    .child(table_body),
            )
            .into_any_element()
    }

}
