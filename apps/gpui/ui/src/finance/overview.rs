//! Financial overview, net worth, assets/liabilities balance snapshots.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, FontWeight, SharedString, Window,
};
use sjel_gpui_core::context::format_money;
use sjel_gpui_core::finance::{format_exact_money, parse_personal_cents};
use sjel_gpui_core::shell::{self, BalanceCoverage, BalanceKind, ManualBalance, ManualBalanceUpdate, ShellError};

use crate::primitives::{stat_card, stat_strip};
use crate::theme::Palette;

use super::transactions::render_tx_row;
use super::{FinancePage, View};

pub(crate) struct BalanceDraft {
    pub(super) id: String,
    pub(super) label: Entity<InputState>,
    pub(super) amount: Entity<InputState>,
    pub(super) kind: BalanceKind,
}


impl FinancePage {
    pub(super) fn balance_draft(
        id: String,
        kind: BalanceKind,
        label: String,
        amount: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> BalanceDraft {
        let label = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Account label");
            input.set_value(label, window, cx);
            input
        });
        let amount = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("0,00");
            input.set_value(amount, window, cx);
            input
        });
        BalanceDraft {
            id,
            label,
            amount,
            kind,
        }
    }

    pub(super) fn open_balance_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.balance_fields.clear();
        if let Some(snapshot) = self
            .dashboard
            .value
            .as_ref()
            .and_then(|dashboard| dashboard.balance_snapshot.as_ref())
        {
            self.balance_as_of = snapshot.as_of.clone();
            self.balance_currency = snapshot.currency.clone();
            self.balance_coverage = snapshot.coverage;
            for balance in &snapshot.balances {
                self.balance_fields.push(Self::balance_draft(
                    balance.id.clone(),
                    balance.kind,
                    balance.label.clone(),
                    format!(
                        "{},{:02}",
                        balance.amount_cents / 100,
                        balance.amount_cents % 100
                    ),
                    window,
                    cx,
                ));
            }
        } else {
            self.balance_as_of = chrono::Local::now().format("%Y-%m-%d").to_string();
            self.balance_currency = "EUR".into();
            self.balance_coverage = BalanceCoverage::Partial;
        }
        let date = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("YYYY-MM-DD");
            input.set_value(self.balance_as_of.clone(), window, cx);
            input
        });
        let subscription = cx.subscribe_in(&date, window, |page, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                page.balance_as_of = input.read(cx).value().to_string();
                cx.notify();
            }
        });
        self.balance_date_input = Some((date, subscription));
        let currency = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("EUR");
            input.set_value(self.balance_currency.clone(), window, cx);
            input
        });
        let currency_subscription =
            cx.subscribe_in(&currency, window, |page, input, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    page.balance_currency = input.read(cx).value().trim().to_ascii_uppercase();
                    cx.notify();
                }
            });
        self.balance_currency_input = Some((currency, currency_subscription));
        self.balance_editor_open = true;
        cx.notify();
    }

    pub(super) fn add_balance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = format!(
            "manual-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        );
        self.balance_fields.push(Self::balance_draft(
            id,
            BalanceKind::Asset,
            String::new(),
            "0,00".into(),
            window,
            cx,
        ));
        cx.notify();
    }

    pub(super) fn save_balance_snapshot(&mut self, cx: &mut Context<Self>) {
        let mut balances = Vec::with_capacity(self.balance_fields.len());
        for draft in &self.balance_fields {
            let label = draft.label.read(cx).value().trim().to_string();
            let Some(amount_cents) = parse_personal_cents(draft.amount.read(cx).value().as_ref())
            else {
                self.notice =
                    Some("Enter non-negative amounts with at most two decimal places.".into());
                cx.notify();
                return;
            };
            if label.is_empty() {
                self.notice = Some("Every balance needs a label.".into());
                cx.notify();
                return;
            }
            balances.push(ManualBalance {
                id: draft.id.clone(),
                label,
                kind: draft.kind,
                amount_cents,
            });
        }
        if balances.is_empty()
            || self.balance_as_of.len() != 10
            || self.balance_currency.trim().is_empty()
        {
            self.notice = Some("Add at least one balance and a valid as-of date.".into());
            cx.notify();
            return;
        }
        let update = ManualBalanceUpdate {
            as_of: self.balance_as_of.clone(),
            currency: self.balance_currency.clone(),
            coverage: self.balance_coverage,
            balances,
        };
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let work = shell::run(async move { shell.finance_update_balance_snapshot(update).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                match result {
                    Ok(_) => {
                        page.balance_editor_open = false;
                        page.notice = Some("Manual balances saved.".into());
                    }
                    Err(error) => page.notice = Some(error.to_string()),
                }
                page.load(cx);
                cx.notify();
            });
        })
        .detach();
    }


    pub(super) fn render_overview(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        self.sync_subs_derived();
        let monthly_burn = self.subs_derived.monthly_burn;
        let active_subs = self.subs_derived.active_count;
        let projection = self.dashboard.value.as_ref();
        let (income, spend, curr) = match projection {
            Some(p) => (
                p.summary.income_cents,
                p.summary.personal_spending_cents,
                p.summary.currency.clone(),
            ),
            None => (0, 0, "EUR".to_owned()),
        };

        let net = income - spend;

        let stat_cards = stat_strip(vec![
            stat_card(
                "MONTHLY SPENDING",
                format_money(-spend, &curr),
                Some("Personal expenses".into()),
                palette.text,
                palette,
            ),
            stat_card(
                "MONTHLY INCOME",
                format_money(income, &curr),
                Some("Inflows & earnings".into()),
                palette.text,
                palette,
            ),
            stat_card(
                "NET SAVINGS",
                format_money(net, &curr),
                Some(if net >= 0 {
                    "Positive net".into()
                } else {
                    "Negative net".into()
                }),
                if net >= 0 { palette.green } else { palette.red },
                palette,
            ),
            stat_card(
                "RECURRING BURN",
                format_money(-monthly_burn, &curr),
                Some(format!("{active_subs} active subscriptions").into()),
                palette.text,
                palette,
            ),
        ]);

        let recent_rows: Vec<AnyElement> = projection
            .map(|p| {
                p.transactions
                    .iter()
                    .take(8)
                    .map(|tx| render_tx_row(tx, palette))
                    .collect()
            })
            .unwrap_or_default();

        let recent_preview = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("RECENT TRANSACTIONS"),
                    )
                    .child(
                        Button::new("goto-transactions")
                            .xsmall()
                            .ghost()
                            .label("View all →")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.set_view(View::Transactions, cx);
                            })),
                    ),
            )
            .child(if recent_rows.is_empty() {
                div()
                    .p_6()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .bg(rgba(palette.surface))
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("No transactions recorded.")
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(recent_rows)
                    .into_any_element()
            });

        let balance_snapshot = projection.and_then(|value| value.balance_snapshot.clone());
        let tracked_net_worth = projection.and_then(|value| value.tracked_net_worth.clone());
        let balance_panel = if let Some(snapshot) = balance_snapshot.as_ref() {
            let mut balances = div().flex().flex_col().gap_1();
            for balance in &snapshot.balances {
                let sign = if balance.kind == shell::BalanceKind::Liability {
                    -1
                } else {
                    1
                };
                balances = balances.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_sm()
                        .child(balance.label.clone())
                        .child(format_money(
                            balance.amount_cents * sign,
                            &snapshot.currency,
                        )),
                );
            }
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_4()
                .rounded(px(8.))
                .border_1()
                .border_color(rgba(palette.border))
                .bg(rgba(palette.surface))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.muted))
                                .child("MANUAL BALANCES"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!(
                                    "As of {} · {}",
                                    snapshot.as_of,
                                    match snapshot.coverage {
                                        shell::BalanceCoverage::Complete => "complete",
                                        shell::BalanceCoverage::Partial => "partial",
                                    }
                                )),
                        ),
                )
                .child(balances)
                .child(if let Some(net_worth) = tracked_net_worth.as_ref() {
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "Tracked net worth {} · {} · portfolio {}",
                            format_exact_money(
                                &net_worth.value.mantissa,
                                net_worth.value.scale,
                                &net_worth.currency
                            )
                            .unwrap_or_else(|| "—".into()),
                            if net_worth.complete {
                                "complete"
                            } else {
                                "incomplete"
                            },
                            if net_worth.portfolio_included {
                                "included"
                            } else {
                                "not included"
                            },
                        ))
                        .into_any_element()
                } else {
                    div().into_any_element()
                })
                .child(
                    Button::new("finance-edit-balances")
                        .small()
                        .ghost()
                        .label("Edit balances")
                        .on_click(
                            cx.listener(|page, _, window, cx| page.open_balance_editor(window, cx)),
                        ),
                )
                .into_any_element()
        } else {
            div()
                .p_4()
                .rounded(px(8.))
                .border_1()
                .border_color(rgba(palette.border))
                .bg(rgba(palette.surface))
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("No manual balance snapshot configured.")
                .child(
                    Button::new("finance-add-balances")
                        .small()
                        .ghost()
                        .label("Add balances")
                        .on_click(
                            cx.listener(|page, _, window, cx| page.open_balance_editor(window, cx)),
                        ),
                )
                .into_any_element()
        };

        let balance_editor = if self.balance_editor_open {
            self.render_balance_editor(palette, cx)
        } else {
            div().into_any_element()
        };

        div()
            .id("finance-overview-scroll")
            .size_full()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_4()
            .overflow_y_scroll()
            .track_scroll(&self.overview_scroll)
            .vertical_scrollbar(&self.overview_scroll)
            .pb_6()
            .child(stat_cards)
            .child(balance_panel)
            .child(balance_editor)
            .child(recent_preview)
            .into_any_element()
    }

    pub(super) fn render_balance_editor(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = div().flex().flex_col().gap_2();
        for draft in &self.balance_fields {
            let id = draft.id.clone();
            let asset_id = id.clone();
            let liability_id = id.clone();
            let remove_id = id.clone();
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(170.)).child(Input::new(&draft.label).small()))
                    .child(div().w(px(100.)).child(Input::new(&draft.amount).small()))
                    .child(
                        Button::new(SharedString::from(format!("balance-asset-{id}")))
                            .xsmall()
                            .label("Asset")
                            .when(draft.kind == BalanceKind::Asset, |button| button.primary())
                            .when(draft.kind != BalanceKind::Asset, |button| button.ghost())
                            .on_click(cx.listener(move |page, _, _, cx| {
                                if let Some(balance) = page
                                    .balance_fields
                                    .iter_mut()
                                    .find(|balance| balance.id == asset_id)
                                {
                                    balance.kind = BalanceKind::Asset;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("balance-liability-{id}")))
                            .xsmall()
                            .label("Liability")
                            .when(draft.kind == BalanceKind::Liability, |button| {
                                button.primary()
                            })
                            .when(draft.kind != BalanceKind::Liability, |button| {
                                button.ghost()
                            })
                            .on_click(cx.listener(move |page, _, _, cx| {
                                if let Some(balance) = page
                                    .balance_fields
                                    .iter_mut()
                                    .find(|balance| balance.id == liability_id)
                                {
                                    balance.kind = BalanceKind::Liability;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("balance-remove-{id}")))
                            .xsmall()
                            .ghost()
                            .label("Remove")
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.balance_fields
                                    .retain(|balance| balance.id != remove_id);
                                cx.notify();
                            })),
                    ),
            );
        }
        let Some((date, _)) = &self.balance_date_input else {
            return div().into_any_element();
        };
        let Some((currency, _)) = &self.balance_currency_input else {
            return div().into_any_element();
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child("Edit manual balance snapshot"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("As of"),
                    )
                    .child(div().w(px(150.)).child(Input::new(date).small()))
                    .child(div().w(px(90.)).child(Input::new(currency).small())),
            )
            .child(rows)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("balance-add-row")
                            .small()
                            .ghost()
                            .label("Add balance")
                            .on_click(
                                cx.listener(|page, _, window, cx| page.add_balance(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("balance-coverage-partial")
                            .small()
                            .label("Incomplete")
                            .when(
                                self.balance_coverage == BalanceCoverage::Partial,
                                |button| button.primary(),
                            )
                            .when(
                                self.balance_coverage != BalanceCoverage::Partial,
                                |button| button.ghost(),
                            )
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.balance_coverage = BalanceCoverage::Partial;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("balance-coverage-complete")
                            .small()
                            .label("Complete")
                            .when(
                                self.balance_coverage == BalanceCoverage::Complete,
                                |button| button.primary(),
                            )
                            .when(
                                self.balance_coverage != BalanceCoverage::Complete,
                                |button| button.ghost(),
                            )
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.balance_coverage = BalanceCoverage::Complete;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("balance-save")
                            .small()
                            .primary()
                            .label("Save snapshot")
                            .when(self.balance_fields.is_empty(), |button| {
                                button.disabled(true)
                            })
                            .on_click(cx.listener(|page, _, _, cx| page.save_balance_snapshot(cx))),
                    )
                    .child(
                        Button::new("balance-cancel")
                            .small()
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.balance_editor_open = false;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

}
