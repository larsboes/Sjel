//! Financial overview, net worth, assets/liabilities balance snapshots.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    AnyElement, Context, Entity, FontWeight, SharedString, Window, div, prelude::*, px, rgb, rgba,
};
use sjel_gpui_core::context::format_money;
use sjel_gpui_core::finance::{format_exact_money, parse_personal_cents};
use sjel_gpui_core::shell::{
    self, BalanceCoverage, BalanceKind, FinancePortfolio, FinancePosition, FinanceTrendPoint,
    ManualBalance, ManualBalanceUpdate, ShellError,
};

use crate::primitives::{mono_num, stat_card, stat_strip, tone_chip};
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

    pub(super) fn render_overview(
        &mut self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_subs_derived();
        let monthly_burn = self.subs_derived.monthly_burn;
        let active_subs = self.subs_derived.active_count;
        let projection = self.dashboard.value.as_ref();
        let curr = projection
            .map(|p| p.summary.currency.clone())
            .filter(|currency| !currency.is_empty())
            .unwrap_or_else(|| "EUR".to_owned());
        let trend = projection.map(|p| p.trend.as_slice()).unwrap_or_default();
        // The summary covers every imported row; the cards show one month.
        let latest = trend.last();
        let month_label = latest
            .map(|point| month_name(&point.month, "%b %Y"))
            .unwrap_or_else(|| "no data".into());
        let income = latest.map_or(0, |point| point.income_cents);
        let spend = latest.map_or(0, |point| point.personal_spending_cents);
        let net = income - spend;
        let portfolio = projection.and_then(|p| p.portfolio.as_ref());

        let mut cards = vec![
            stat_card(
                format!("SPENDING · {month_label}"),
                format_money(-spend, &curr),
                Some("Personal expenses".into()),
                palette.text,
                palette,
            ),
            stat_card(
                format!("INCOME · {month_label}"),
                format_money(income, &curr),
                Some("Inflows & earnings".into()),
                palette.text,
                palette,
            ),
            stat_card(
                format!("NET · {month_label}"),
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
        ];
        if let Some(portfolio) = portfolio {
            let fresh = portfolio
                .positions
                .iter()
                .filter(|position| position.price_freshness == "fresh")
                .count();
            let total = portfolio.positions.len();
            cards.push(stat_card(
                "PORTFOLIO",
                format_exact_money(
                    &portfolio.total.mantissa,
                    portfolio.total.scale,
                    &portfolio.currency,
                )
                .unwrap_or_else(|| "—".into()),
                Some(format!("{fresh} of {total} prices fresh").into()),
                if fresh == total {
                    palette.text
                } else {
                    palette.amber
                },
                palette,
            ));
        }
        // A spend goal, such as a card's welcome offer: spent against target, and what the
        // remaining days need (`planning.spend_goals`, crates/finance/src/planning.rs).
        for goal in projection
            .and_then(|p| p.planning.as_ref())
            .map(|planning| planning.spend_goals.as_slice())
            .unwrap_or_default()
        {
            let detail = if goal.met {
                "Target reached".to_owned()
            } else if goal.days_left == 0 {
                format!(
                    "Window closed · {} short",
                    format_money(goal.remaining_cents, &goal.currency)
                )
            } else {
                format!(
                    "{} left · {}/day · until {}",
                    format_money(goal.remaining_cents, &goal.currency),
                    format_money(goal.needed_per_day_cents, &goal.currency),
                    goal.end
                )
            };
            cards.push(stat_card(
                goal.label.to_uppercase(),
                format!(
                    "{} / {}",
                    format_money(goal.spent_cents, &goal.currency),
                    format_money(goal.target_cents, &goal.currency)
                ),
                Some(detail.into()),
                if goal.met {
                    palette.green
                } else if goal.days_left == 0 {
                    palette.red
                } else {
                    palette.text
                },
                palette,
            ));
        }
        let stat_cards = stat_strip(cards);
        let trend_panel = render_trend(trend, &curr, palette);
        let holdings_panel = portfolio
            .map(|portfolio| render_holdings(portfolio, palette))
            .unwrap_or_else(|| div().into_any_element());

        let date_w = self.col_width("tx-date", 88.0);
        let cat_w = self.col_width("tx-cat", 120.0);
        let acct_w = self.col_width("tx-acct", 120.0);
        let amt_w = self.col_width("tx-amt", 110.0);
        let recent_rows: Vec<AnyElement> = projection
            .map(|p| {
                p.transactions
                    .iter()
                    .take(8)
                    .map(|tx| render_tx_row(tx, date_w, cat_w, acct_w, amt_w, palette))
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
            .child(trend_panel)
            .child(holdings_panel)
            .child(balance_panel)
            .child(balance_editor)
            .child(recent_preview)
            .into_any_element()
    }

    pub(super) fn render_balance_editor(
        &mut self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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

const TREND_MONTHS: usize = 6;
const TREND_BAR_HEIGHT: f32 = 96.;

/// The six calendar months ending at the newest month in `trend`, oldest first.
/// A month with no imported rows is `None`, not zero.
fn trend_window(trend: &[FinanceTrendPoint]) -> Vec<(String, Option<&FinanceTrendPoint>)> {
    let Some(last) = trend.last() else {
        return Vec::new();
    };
    let Some((year, month)) = last
        .month
        .split_once('-')
        .and_then(|(y, m)| Some((y.parse::<i32>().ok()?, m.parse::<i32>().ok()?)))
    else {
        return Vec::new();
    };
    (0..TREND_MONTHS as i32)
        .rev()
        .map(|back| {
            let index = year * 12 + (month - 1) - back;
            let key = format!(
                "{:04}-{:02}",
                index.div_euclid(12),
                index.rem_euclid(12) + 1
            );
            let point = trend.iter().find(|point| point.month == key);
            (key, point)
        })
        .collect()
}

fn month_name(month: &str, format: &str) -> String {
    chrono::NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d")
        .map(|date| date.format(format).to_string())
        .unwrap_or_else(|_| month.to_owned())
}

fn panel(palette: Palette) -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded(px(8.))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgba(palette.surface))
}

fn panel_title(text: impl Into<SharedString>, palette: Palette) -> gpui_kit::Div {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette.muted))
        .child(text.into())
}

fn render_trend(trend: &[FinanceTrendPoint], currency: &str, palette: Palette) -> AnyElement {
    let window = trend_window(trend);
    if window.is_empty() {
        return div().into_any_element();
    }
    let peak = window
        .iter()
        .filter_map(|(_, point)| *point)
        .map(|point| point.income_cents.max(point.personal_spending_cents))
        .max()
        .unwrap_or(0)
        .max(1);
    let bar = |cents: i64, tone: u32| {
        let height = (cents.max(0) as f32 / peak as f32 * TREND_BAR_HEIGHT).max(2.);
        div()
            .w(px(14.))
            .h(px(height))
            .rounded_t(px(3.))
            .bg(rgb(tone))
    };
    let columns = window.into_iter().map(|(month, point)| {
        let short = month_name(&month, "%b");
        let (bars, net) = match point {
            Some(point) => {
                let net = point.income_cents - point.personal_spending_cents;
                (
                    div()
                        .flex()
                        .items_end()
                        .gap_1()
                        .h(px(TREND_BAR_HEIGHT))
                        .child(bar(point.income_cents, palette.green))
                        .child(bar(point.personal_spending_cents, palette.red))
                        .into_any_element(),
                    mono_num(
                        format_money(net, currency),
                        if net >= 0 { palette.green } else { palette.red },
                    )
                    .text_xs()
                    .into_any_element(),
                )
            }
            None => (
                div()
                    .h(px(TREND_BAR_HEIGHT))
                    .flex()
                    .items_end()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("no rows")
                    .into_any_element(),
                div().text_xs().child(" ").into_any_element(),
            ),
        };
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .child(bars)
            .child(div().text_xs().text_color(rgb(palette.muted)).child(short))
            .child(net)
    });
    let legend = |text: &'static str, tone: u32| {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(div().size(px(8.)).rounded(px(2.)).bg(rgb(tone)))
            .child(text)
    };
    panel(palette)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(panel_title("LAST 6 MONTHS", palette))
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(legend("Income", palette.green))
                        .child(legend("Spending", palette.red)),
                ),
        )
        .child(div().flex().items_end().gap_2().children(columns))
        .into_any_element()
}

/// Where a position's price came from, in one line a person can act on.
fn price_provenance(position: &FinancePosition) -> (String, bool) {
    let source = position.market_price_source.as_deref().unwrap_or("none");
    let date = position.market_price_observed_on.as_deref().unwrap_or("");
    let stale = position.price_freshness != "fresh";
    let text = match (source, &position.conversion) {
        ("broker", _) | ("none", _) => match date {
            "" => "No price".to_owned(),
            date => format!("Broker price from {date} · no market quote"),
        },
        (source, Some(conversion)) => format!(
            "{source} · {date} · {} @ {} ({})",
            format_exact_money(
                &conversion.quoted_price.mantissa,
                conversion.quoted_price.scale,
                &conversion.quoted_currency
            )
            .unwrap_or_default(),
            decimal_text(&conversion.rate.mantissa, conversion.rate.scale),
            conversion.rate_observed_on
        ),
        (source, None) => format!("{source} · {date}"),
    };
    let age = match position.price_age_days {
        Some(days) if stale && days > 0 => format!(" · {days} d old"),
        _ => String::new(),
    };
    (format!("{text}{age}"), stale)
}

fn decimal_text(mantissa: &str, scale: u32) -> String {
    let scale = scale as usize;
    let (sign, digits) = match mantissa.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", mantissa),
    };
    if scale == 0 {
        return format!("{sign}{digits}");
    }
    let padded = format!("{digits:0>width$}", width = scale + 1);
    let (whole, fraction) = padded.split_at(padded.len() - scale);
    format!("{sign}{whole},{}", fraction.trim_end_matches('0'))
        .trim_end_matches(',')
        .to_owned()
}

fn percent_bp(bp: i64) -> String {
    format!("{}{:.1} %", if bp > 0 { "+" } else { "" }, bp as f64 / 100.)
}

fn render_holdings(portfolio: &FinancePortfolio, palette: Palette) -> AnyElement {
    let mut positions: Vec<&FinancePosition> = portfolio.positions.iter().collect();
    positions.sort_by_key(|position| std::cmp::Reverse(position.share_bp));
    let rows = positions.into_iter().map(|position| {
        let (provenance, stale) = price_provenance(position);
        let change = position.change_since_review_bp.filter(|bp| *bp != 0);
        div()
            .flex()
            .items_center()
            .gap_3()
            .py_1()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_sm().child(position.label.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(if stale { palette.amber } else { palette.muted }))
                            .child(provenance),
                    ),
            )
            .child(match change {
                Some(bp) => tone_chip(
                    format!("{} vs last trade", percent_bp(bp)),
                    if bp >= 0 { palette.green } else { palette.red },
                ),
                None => div().into_any_element(),
            })
            .child(
                mono_num(
                    percent_bp(position.share_bp).replace('+', ""),
                    palette.muted,
                )
                .w(px(56.))
                .text_xs(),
            )
            .child(
                mono_num(
                    format_exact_money(
                        &position.value.mantissa,
                        position.value.scale,
                        &position.currency,
                    )
                    .unwrap_or_else(|| "—".into()),
                    palette.text,
                )
                .w(px(110.))
                .text_sm(),
            )
    });
    panel(palette)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(panel_title("HOLDINGS", palette))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("As of {}", portfolio.as_of)),
                ),
        )
        .child(div().flex().flex_col().children(rows))
        .children(portfolio.caveats.iter().map(|caveat| {
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(caveat.clone())
        }))
        .into_any_element()
}

#[cfg(test)]
mod overview_tests {
    use super::*;

    fn point(month: &str) -> FinanceTrendPoint {
        FinanceTrendPoint {
            month: month.into(),
            income_cents: 100,
            personal_spending_cents: 50,
        }
    }

    #[test]
    fn the_trend_window_is_six_calendar_months_across_a_year_with_gaps_kept() {
        let trend = [point("2025-09"), point("2026-01"), point("2026-02")];
        let window = trend_window(&trend);
        let months: Vec<_> = window.iter().map(|(month, _)| month.as_str()).collect();
        assert_eq!(
            months,
            [
                "2025-09", "2025-10", "2025-11", "2025-12", "2026-01", "2026-02"
            ]
        );
        assert!(window[1].1.is_none());
        assert!(window[5].1.is_some());
        assert!(trend_window(&[]).is_empty());
    }

    #[test]
    fn a_rate_is_written_with_its_significant_decimals() {
        assert_eq!(decimal_text("11224", 4), "1,1224");
        assert_eq!(decimal_text("83393", 5), "0,83393");
        assert_eq!(decimal_text("104365", 4), "10,4365");
        assert_eq!(decimal_text("10000", 4), "1");
    }
}
