//! Financial overview, net worth, assets/liabilities balance snapshots.

use std::rc::Rc;

use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::charts::{BarChart, Bullet, BulletChart, LineChart, Series, Treemap};
use crate::ely::data_display::Tone;
use crate::ely::forms::{Input, InputEvent, TextInput};
use crate::ely::layout::Sheet;
use crate::ely::primitives::Place;
use crate::ely::tables::{Cell, Column, DataTable, Row, SortKey};
use crate::ely::theme::{ControlSize, Density};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{
    div, prelude::*, px, rems, rgb, rgba, AnyElement, Context, Entity, FontWeight, SharedString,
    Window,
};
use sjel_gpui_core::context::format_money;
use sjel_gpui_core::finance::{format_exact_money, parse_personal_cents};
use sjel_gpui_core::shell::{
    self, BalanceCoverage, BalanceKind, FinancePortfolio, FinancePosition, FinanceTrendPoint,
    ManualBalance, ManualBalanceUpdate, ShellError,
};

use crate::primitives::{mono_num, stat_card, stat_strip, sub_tabs};
use crate::theme::Palette;

use super::transactions::render_tx_row;
use super::{FinancePage, OverviewSection, View};

pub(crate) struct BalanceDraft {
    pub(super) id: String,
    pub(super) label: Entity<TextInput>,
    pub(super) amount: Entity<TextInput>,
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
            let mut input = TextInput::new(window, cx).placeholder("Account label");
            input.set_text(label, cx);
            input
        });
        let amount = cx.new(|cx| {
            let mut input = TextInput::new(window, cx).placeholder("0,00");
            input.set_text(amount, cx);
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
            let mut input = TextInput::new(window, cx).placeholder("YYYY-MM-DD");
            input.set_text(self.balance_as_of.clone(), cx);
            input
        });
        let subscription = cx.subscribe_in(&date, window, |page, input, event, _, cx| {
            if matches!(event, InputEvent::Changed) {
                page.balance_as_of = input.read(cx).text().to_string();
                cx.notify();
            }
        });
        self.balance_date_input = Some((date, subscription));
        let currency = cx.new(|cx| {
            let mut input = TextInput::new(window, cx).placeholder("EUR");
            input.set_text(self.balance_currency.clone(), cx);
            input
        });
        let currency_subscription =
            cx.subscribe_in(&currency, window, |page, input, event, _, cx| {
                if matches!(event, InputEvent::Changed) {
                    page.balance_currency = input.read(cx).text().trim().to_ascii_uppercase();
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
            let label = draft.label.read(cx).text().trim().to_string();
            let Some(amount_cents) = parse_personal_cents(draft.amount.read(cx).text()) else {
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
        let spend_goals = projection
            .and_then(|p| p.planning.as_ref())
            .map(|planning| planning.spend_goals.as_slice())
            .unwrap_or_default();
        let goals_panel = render_spend_goals_chart(spend_goals, palette);
        let trend_panel = render_trend(trend, &curr, palette, Some(cx.weak_entity()));
        let holdings_count = portfolio.map_or(0, |p| p.positions.len());
        let holdings_panel = portfolio
            .map(|portfolio| render_holdings(portfolio, palette))
            .unwrap_or_else(|| {
                panel(palette)
                    .child(panel_title("HOLDINGS", palette))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child("No portfolio positions loaded."),
                    )
                    .into_any_element()
            });

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
        let recent_count = recent_rows.len();

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
                        Button::new("goto-transactions", "View all →")
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
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
        let balances_count = balance_snapshot.as_ref().map_or(0, |s| s.balances.len());
        let tracked_net_worth = projection.and_then(|value| value.tracked_net_worth.clone());
        let balance_panel =
            if let Some(snapshot) = balance_snapshot.as_ref() {
                let mut rows = Vec::with_capacity(snapshot.balances.len());
                for balance in &snapshot.balances {
                    let is_liability = balance.kind == shell::BalanceKind::Liability;
                    let signed = if is_liability {
                        -balance.amount_cents
                    } else {
                        balance.amount_cents
                    };
                    rows.push(Row::new(
                        balance.id.clone(),
                        [
                            Cell::Text(balance.label.clone().into()),
                            Cell::Tag(
                                if is_liability { "Liability" } else { "Asset" }.into(),
                                if is_liability {
                                    Tone::Danger
                                } else {
                                    Tone::Success
                                },
                            ),
                            Cell::Figure(
                                format_money(signed, &snapshot.currency).into(),
                                signed as f64,
                                if is_liability {
                                    Tone::Danger
                                } else {
                                    Tone::Neutral
                                },
                            ),
                        ],
                    ));
                }
                let table = DataTable::new(
                    "finance-manual-balances",
                    [
                        Column::new("account", "ACCOUNT"),
                        Column::new("kind", "KIND").width(rems(8.)),
                        Column::new("balance", "BALANCE").width(rems(9.)).end(),
                    ],
                )
                .label("Manual balances")
                .rows(Rc::new(rows))
                .density(Density::Comfortable)
                .sorts([SortKey {
                    column: "balance".into(),
                    rising: false,
                }]);
                let assets_total: i64 = snapshot
                    .balances
                    .iter()
                    .filter(|b| b.kind == shell::BalanceKind::Asset)
                    .map(|b| b.amount_cents)
                    .sum();
                let liabilities_total: i64 = snapshot
                    .balances
                    .iter()
                    .filter(|b| b.kind == shell::BalanceKind::Liability)
                    .map(|b| b.amount_cents)
                    .sum();
                let balance_chart =
                    (assets_total > 0 || liabilities_total > 0).then(|| {
                        let curr = snapshot.currency.clone();
                        let max_bar = (assets_total.max(liabilities_total).max(1)) as f64;
                        BulletChart::new("finance-balances-breakdown")
                            .format(move |cents| format_money(cents.round() as i64, &curr))
                            .bullet(Bullet::new("Assets", assets_total as f64, max_bar).bands([
                                max_bar * 0.5,
                                max_bar * 0.85,
                                max_bar,
                            ]))
                            .bullet(
                                Bullet::new("Liabilities", liabilities_total as f64, max_bar)
                                    .bands([max_bar * 0.5, max_bar * 0.85, max_bar]),
                            )
                            .w_full()
                    });
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
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
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(div().text_xs().text_color(rgb(palette.muted)).child(
                                        format!(
                                            "As of {} · {}",
                                            snapshot.as_of,
                                            match snapshot.coverage {
                                                shell::BalanceCoverage::Complete => "complete",
                                                shell::BalanceCoverage::Partial => "partial",
                                            }
                                        ),
                                    ))
                                    .child(
                                        Button::new("finance-edit-balances", "Edit balances")
                                            .size(ControlSize::Sm)
                                            .variant(ButtonVariant::Ghost)
                                            .on_click(cx.listener(|page, _, window, cx| {
                                                page.open_balance_editor(window, cx)
                                            })),
                                    ),
                            ),
                    )
                    .children(balance_chart)
                    .child(table)
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
                    .into_any_element()
            } else {
                div()
                    .p_4()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .bg(rgba(palette.surface))
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("No manual balance snapshot configured.")
                    .child(
                        Button::new("finance-add-balances", "Add balances")
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(cx.listener(|page, _, window, cx| {
                                page.open_balance_editor(window, cx)
                            })),
                    )
                    .into_any_element()
            };

        let balance_editor = if self.balance_editor_open {
            let entity = cx.entity();
            let body = self.render_balance_editor(palette, cx);
            Sheet::new(
                "finance-balance-editor-sheet",
                Place::Right,
                "Manual Balance Snapshot",
                move |_, cx| {
                    entity.update(cx, |page, cx| {
                        page.balance_editor_open = false;
                        cx.notify();
                    });
                },
            )
            .child(body)
            .into_any_element()
        } else {
            div().into_any_element()
        };

        let entity = cx.entity();
        let section_nav = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(sub_tabs(
                "finance-overview-section",
                vec![
                    (
                        "Cash Flow & Trend",
                        None,
                        self.overview_section == OverviewSection::CashFlow,
                    ),
                    (
                        "Portfolio Holdings",
                        Some(holdings_count),
                        self.overview_section == OverviewSection::Holdings,
                    ),
                    (
                        "Balances & Net Worth",
                        Some(balances_count),
                        self.overview_section == OverviewSection::Balances,
                    ),
                    (
                        "Recent Activity",
                        Some(recent_count),
                        self.overview_section == OverviewSection::Recent,
                    ),
                ],
                move |idx, _, cx| {
                    entity.update(cx, |page, cx| {
                        page.overview_section = match idx {
                            0 => OverviewSection::CashFlow,
                            1 => OverviewSection::Holdings,
                            2 => OverviewSection::Balances,
                            _ => OverviewSection::Recent,
                        };
                        cx.notify();
                    });
                },
            ))
            .child(
                Button::new("finance-overview-edit-balances-top", "Manual Balances")
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(
                        cx.listener(|page, _, window, cx| page.open_balance_editor(window, cx)),
                    ),
            );

        let section_body = if cfg!(test) {
            match self.overview_section {
                OverviewSection::CashFlow => div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .children(goals_panel)
                    .child(trend_panel)
                    .child(recent_preview)
                    .into_any_element(),
                OverviewSection::Holdings => holdings_panel,
                OverviewSection::Balances => balance_panel,
                OverviewSection::Recent => recent_preview.into_any_element(),
            }
        } else {
            div()
                .flex()
                .flex_wrap()
                .items_start()
                .gap_4()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(420.))
                        .flex()
                        .flex_col()
                        .gap_4()
                        .children(goals_panel)
                        .child(trend_panel)
                        .child(recent_preview),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(380.))
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(holdings_panel)
                        .child(balance_panel),
                )
                .into_any_element()
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
            .when(cfg!(test), |el| el.child(section_nav))
            .when(!cfg!(test), |el| {
                el.child(
                    div().flex().items_center().justify_end().child(
                        Button::new("finance-overview-edit-balances-top", "Manual Balances")
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(cx.listener(|page, _, window, cx| {
                                page.open_balance_editor(window, cx)
                            })),
                    ),
                )
            })
            .child(section_body)
            .child(balance_editor)
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
                    .child(
                        div()
                            .w(px(170.))
                            .child(Input::new(&draft.label).size(ControlSize::Sm)),
                    )
                    .child(
                        div()
                            .w(px(100.))
                            .child(Input::new(&draft.amount).size(ControlSize::Sm)),
                    )
                    .child(
                        Button::new(SharedString::from(format!("balance-asset-{id}")), "Asset")
                            .size(ControlSize::Sm)
                            .when(draft.kind == BalanceKind::Asset, |button| button.primary())
                            .when(draft.kind != BalanceKind::Asset, |button| {
                                button.variant(ButtonVariant::Ghost)
                            })
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
                        Button::new(
                            SharedString::from(format!("balance-liability-{id}")),
                            "Liability",
                        )
                        .size(ControlSize::Sm)
                        .when(draft.kind == BalanceKind::Liability, |button| {
                            button.primary()
                        })
                        .when(draft.kind != BalanceKind::Liability, |button| {
                            button.variant(ButtonVariant::Ghost)
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
                        Button::new(SharedString::from(format!("balance-remove-{id}")), "Remove")
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
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
                    .child(
                        div()
                            .w(px(150.))
                            .child(Input::new(date).size(ControlSize::Sm)),
                    )
                    .child(
                        div()
                            .w(px(90.))
                            .child(Input::new(currency).size(ControlSize::Sm)),
                    ),
            )
            .child(rows)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("balance-add-row", "Add balance")
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(
                                cx.listener(|page, _, window, cx| page.add_balance(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("balance-coverage-partial", "Incomplete")
                            .size(ControlSize::Sm)
                            .when(
                                self.balance_coverage == BalanceCoverage::Partial,
                                |button| button.primary(),
                            )
                            .when(
                                self.balance_coverage != BalanceCoverage::Partial,
                                |button| button.variant(ButtonVariant::Ghost),
                            )
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.balance_coverage = BalanceCoverage::Partial;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("balance-coverage-complete", "Complete")
                            .size(ControlSize::Sm)
                            .when(
                                self.balance_coverage == BalanceCoverage::Complete,
                                |button| button.primary(),
                            )
                            .when(
                                self.balance_coverage != BalanceCoverage::Complete,
                                |button| button.variant(ButtonVariant::Ghost),
                            )
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.balance_coverage = BalanceCoverage::Complete;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("balance-save", "Save snapshot")
                            .size(ControlSize::Sm)
                            .primary()
                            .when(self.balance_fields.is_empty(), |button| {
                                button.disabled(true)
                            })
                            .on_click(cx.listener(|page, _, _, cx| page.save_balance_snapshot(cx))),
                    )
                    .child(
                        Button::new("balance-cancel", "Cancel")
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
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
    crate::primitives::surface_card(palette)
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
}

fn panel_title(text: impl Into<SharedString>, palette: Palette) -> gpui_kit::Div {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette.muted))
        .child(text.into())
}

fn render_trend(
    trend: &[FinanceTrendPoint],
    currency: &str,
    palette: Palette,
    page: Option<gpui_kit::WeakEntity<FinancePage>>,
) -> AnyElement {
    let window = trend_window(trend);
    if window.is_empty() {
        return div().into_any_element();
    }
    let observed: Vec<_> = window.iter().filter_map(|(_, point)| *point).collect();
    if observed.is_empty() {
        return div().into_any_element();
    }
    let currency_owned = currency.to_owned();
    let month_keys: Vec<String> = observed.iter().map(|point| point.month.clone()).collect();
    let mut chart = BarChart::new(
        "finance-monthly-trend",
        observed
            .iter()
            .map(|point| month_name(&point.month, "%b %Y")),
    )
    .series(Series::new(
        "Income",
        observed.iter().map(|point| point.income_cents as f64),
    ))
    .series(Series::new(
        "Spending",
        observed
            .iter()
            .map(|point| point.personal_spending_cents as f64),
    ))
    .format(move |cents| format_money(cents.round() as i64, &currency_owned))
    .rule(
        observed
            .iter()
            .map(|point| point.personal_spending_cents as f64)
            .sum::<f64>()
            / observed.len() as f64,
        "Average spending",
    )
    .h(px(220.))
    .w_full();
    if let Some(page) = page {
        chart = chart.on_select(move |ix, window, cx| {
            if let Some(month) = month_keys.get(ix).cloned() {
                let _ = page.update(cx, |page, cx| page.open_date(month, window, cx));
            }
        });
    }
    let net_line = (observed.len() >= 3).then(|| {
        let net_curr = currency.to_owned();
        LineChart::new(
            "finance-monthly-net-line",
            observed.iter().map(|point| month_name(&point.month, "%b")),
        )
        .series(Series::new(
            "Monthly net",
            observed
                .iter()
                .map(|point| (point.income_cents - point.personal_spending_cents) as f64),
        ))
        .format(move |cents| format_money(cents.round() as i64, &net_curr))
        .rule(0.0, "Break-even")
        .h(px(120.))
        .w_full()
    });
    let columns = window.into_iter().map(|(month, point)| {
        let short = month_name(&month, "%b");
        let net = match point {
            Some(point) => {
                let net = point.income_cents - point.personal_spending_cents;
                mono_num(
                    format_money(net, currency),
                    if net >= 0 { palette.green } else { palette.red },
                )
                .text_xs()
                .into_any_element()
            }
            None => div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("no rows")
                .into_any_element(),
        };
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .child(div().text_xs().text_color(rgb(palette.muted)).child(short))
            .child(net)
    });
    panel(palette)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(panel_title("LAST 6 MONTHS", palette))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("Click a month bar to filter transactions"),
                ),
        )
        .child(chart)
        .children(net_line)
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("Monthly net · chart and spending average use months with imported rows"),
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

fn render_spend_goals_chart(
    goals: &[sjel_gpui_core::shell::FinanceSpendGoal],
    palette: Palette,
) -> Option<AnyElement> {
    if goals.is_empty() {
        return None;
    }
    let currency = goals
        .first()
        .map(|g| g.currency.clone())
        .unwrap_or_else(|| "EUR".to_owned());
    let mut chart = BulletChart::new("finance-spend-goals")
        .format(move |cents| format_money(cents.round() as i64, &currency));
    for goal in goals {
        let target = (goal.target_cents.max(1)) as f64;
        let spent = (goal.spent_cents.max(0)) as f64;
        chart = chart.bullet(Bullet::new(goal.label.clone(), spent, target).bands([
            target * 0.5,
            target * 0.85,
            target,
        ]));
    }
    Some(
        panel(palette)
            .child(panel_title("SPEND TARGETS", palette))
            .child(chart.w_full())
            .into_any_element(),
    )
}

fn holdings_columns() -> [Column; 5] {
    [
        Column::new("position", "POSITION"),
        Column::new("vs_trade", "VS LAST TRADE").width(rems(8.)),
        Column::new("share", "SHARE")
            .width(rems(6.))
            .end()
            .decimals(1)
            .suffix(" %"),
        Column::new("freshness", "QUOTE").width(rems(6.)),
        Column::new("value", "VALUE").width(rems(8.)).end(),
    ]
}

fn holding_row(position: &FinancePosition) -> Row {
    let (provenance, stale) = price_provenance(position);
    let vs_trade = match position.change_since_review_bp.filter(|bp| *bp != 0) {
        Some(bp) => Cell::Tag(
            percent_bp(bp).into(),
            if bp >= 0 { Tone::Success } else { Tone::Danger },
        ),
        None => Cell::Empty,
    };
    let freshness = Cell::Tag(
        if stale { "Stale" } else { "Fresh" }.into(),
        if stale { Tone::Warning } else { Tone::Success },
    );
    let value_str = format_exact_money(
        &position.value.mantissa,
        position.value.scale,
        &position.currency,
    )
    .unwrap_or_else(|| "—".into());
    Row::new(
        position.label.clone(),
        [
            Cell::Lines(position.label.clone().into(), provenance.into()),
            vs_trade,
            Cell::Number(position.share_bp as f64 / 100.0),
            freshness,
            Cell::Figure(value_str.into(), position.share_bp as f64, Tone::Neutral),
        ],
    )
}

fn render_holdings(portfolio: &FinancePortfolio, palette: Palette) -> AnyElement {
    let mut positions: Vec<&FinancePosition> = portfolio.positions.iter().collect();
    positions.sort_by_key(|position| std::cmp::Reverse(position.share_bp));
    let allocation = positions
        .iter()
        .filter(|position| position.share_bp > 0)
        .fold(
            Treemap::new("finance-holdings-treemap"),
            |chart, position| chart.tile(position.label.clone(), position.share_bp as f64 / 100.0),
        )
        .format(|pct| format!("{pct:.1} %"))
        .h(px(190.))
        .w_full();
    let rows: Vec<Row> = positions.into_iter().map(holding_row).collect();
    let table = DataTable::new("finance-holdings-table", holdings_columns())
        .label("Portfolio holdings")
        .rows(Rc::new(rows))
        .density(Density::Comfortable)
        .sorts([SortKey {
            column: "share".into(),
            rising: false,
        }])
        .empty("No positions in portfolio.");
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
        .when(!portfolio.positions.is_empty(), |panel| {
            panel.child(allocation)
        })
        .child(table)
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
    use gpui_kit::{Render, TestAppContext};

    #[gpui_kit::test]
    fn balance_editor_reads_ely_input_events_and_keeps_invalid_amounts(cx: &mut TestAppContext) {
        use gpui_kit::Focusable;

        let (page, cx) = crate::test_support::mount_root(cx, None, |window, cx| {
            let mut page = FinancePage::new(crate::data::Repo::new(Err(ShellError::NotSetUp)), cx);
            page.open_balance_editor(window, cx);
            page.add_balance(window, cx);
            page
        });
        cx.update(|window, cx| {
            let input = &page.read(cx).balance_currency_input.as_ref().unwrap().0;
            input.read(cx).focus_handle(cx).focus(window, cx);
        });
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input("usd");
        cx.run_until_parked();
        page.read_with(cx, |page, _| assert_eq!(page.balance_currency, "USD"));
        cx.update(|window, cx| {
            let input = &page.read(cx).balance_fields[0].amount;
            input.read(cx).focus_handle(cx).focus(window, cx);
        });
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input("12,345");
        page.update(cx, |page, cx| page.save_balance_snapshot(cx));
        page.read_with(cx, |page, cx| {
            assert!(page.balance_editor_open);
            assert_eq!(page.balance_fields[0].amount.read(cx).text(), "12,345");
            assert_eq!(
                page.notice.as_deref(),
                Some("Enter non-negative amounts with at most two decimal places.")
            );
        });
    }

    struct TrendPreview(Vec<FinanceTrendPoint>);

    impl Render for TrendPreview {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(700.))
                .child(render_trend(&self.0, "EUR", Palette::light(), None))
        }
    }

    #[gpui_kit::test]
    fn the_finance_trend_draws_with_gaps_zero_values_and_negative_net(cx: &mut TestAppContext) {
        crate::test_support::init(cx);
        let (view, cx) = cx.add_window_view(|_, _| {
            TrendPreview(vec![
                point("2026-01"),
                FinanceTrendPoint {
                    month: "2026-03".into(),
                    income_cents: 0,
                    personal_spending_cents: 12345,
                },
            ])
        });
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds("chart-root")
            .expect("Ely draws the finance chart");
        assert!(bounds.size.width > px(0.));
        assert_eq!(bounds.size.height, px(220.));
        view.update(cx, |view, cx| {
            view.0.clear();
            cx.notify();
        });
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("chart-root").is_none(),
            "no trend draws no chart"
        );
    }

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
            ["2025-09", "2025-10", "2025-11", "2025-12", "2026-01", "2026-02"]
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
