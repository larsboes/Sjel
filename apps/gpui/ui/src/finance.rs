//! Native Finance page (`/finance`): personal ledger, transaction search,
//! account breakdowns, and recurring subscription tracking.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, AnyElement, App, Context, Entity, FocusHandle,
    FontWeight, KeyBinding, SharedString, Subscription, Window,
};
use sjel_gpui_core::context::format_money;
use sjel_gpui_core::finance::{
    calculate_monthly_burn, filter_transactions, SubscriptionSummary, TransactionFilter,
};
use sjel_gpui_core::shell::{
    self, FinanceDashboardProjection, FinanceTransaction, ShellError, SubscriptionRecord,
};

use crate::data::{Remote, Repo};
use crate::theme::Palette;

actions!(
    finance_page,
    [
        /// Show the overview tab.
        OverviewView,
        /// Show the transactions ledger tab.
        TransactionsView,
        /// Show the subscriptions tab.
        SubscriptionsView,
        /// Focus the search field.
        FocusSearch,
        /// Reload data from the finance capability.
        Refresh,
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("1", OverviewView, Some("FinancePage")),
        KeyBinding::new("2", TransactionsView, Some("FinancePage")),
        KeyBinding::new("3", SubscriptionsView, Some("FinancePage")),
        KeyBinding::new("/", FocusSearch, Some("FinancePage")),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Overview,
    Transactions,
    Subscriptions,
}

pub struct FinancePage {
    repo: Repo,
    palette: Option<Palette>,
    pub view: View,
    pub dashboard: Remote<FinanceDashboardProjection>,
    pub subscriptions: Remote<Vec<SubscriptionRecord>>,
    pub search_query: String,
    pub selected_category: Option<String>,
    search_input: Option<(Entity<InputState>, Subscription)>,
    focus: FocusHandle,
    focus_next: bool,
}

impl FinancePage {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self {
            repo,
            palette: None,
            view: View::Overview,
            dashboard: Remote::default(),
            subscriptions: Remote::default(),
            search_query: String::new(),
            selected_category: None,
            search_input: None,
            focus: cx.focus_handle(),
            focus_next: true,
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn activate(&mut self, cx: &mut Context<Self>) {
        self.focus_next = true;
        self.load(cx);
    }

    pub fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        if self.view != view {
            self.view = view;
            cx.notify();
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };

        self.dashboard.loading = true;
        self.subscriptions.loading = true;
        cx.notify();

        // 1. Fetch Finance Dashboard projection
        let shell_dash = shell.clone();
        let dash_work = shell::run(async move { shell_dash.finance_dashboard(None, None).await });
        cx.spawn(async move |this, cx| {
            let res = dash_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.dashboard.settle(res);
                cx.notify();
            });
        })
        .detach();

        // 2. Fetch Subscriptions
        let shell_subs = shell;
        let subs_work = shell::run(async move { shell_subs.subscriptions().await });
        cx.spawn(async move |this, cx| {
            let res = subs_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.subscriptions.settle(res);
                cx.notify();
            });
        })
        .detach();
    }

    fn search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((input, _)) = &self.search_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search transactions…"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                view.search_query = input.read(cx).value().trim().to_lowercase();
                cx.notify();
            }
        });
        self.search_input = Some((input.clone(), subscription));
        input
    }

    fn on_overview_view(&mut self, _: &OverviewView, _: &mut Window, cx: &mut Context<Self>) {
        self.set_view(View::Overview, cx);
    }

    fn on_transactions_view(
        &mut self,
        _: &TransactionsView,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_view(View::Transactions, cx);
    }

    fn on_subscriptions_view(
        &mut self,
        _: &SubscriptionsView,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_view(View::Subscriptions, cx);
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_view(View::Transactions, cx);
        let field = self.search_field(window, cx);
        field.update(cx, |input, cx| input.focus(window, cx));
    }

    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_search(window, cx);
    }

    fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh(cx);
    }
}

impl Render for FinancePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_next) {
            window.focus(&self.focus, cx);
        }

        let palette = self.palette.unwrap_or_else(Palette::dark);
        let projection = self.dashboard.value.clone();
        let subs_records = self.subscriptions.value.clone().unwrap_or_default();

        let view_btn = |label: &'static str, target: View, cx: &mut Context<Self>| {
            Button::new(SharedString::from(format!("fin-view-{label}")))
                .small()
                .label(label)
                .when(self.view == target, |b| b.primary())
                .when(self.view != target, |b| b.ghost())
                .on_click(cx.listener(move |page, _, _, cx| page.set_view(target, cx)))
        };

        // Header with Tabs
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child("Finance"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(view_btn("Overview", View::Overview, cx))
                            .child(view_btn("Transactions", View::Transactions, cx))
                            .child(view_btn("Subscriptions", View::Subscriptions, cx)),
                    ),
            )
            .child(
                Button::new("finance-refresh")
                    .small()
                    .ghost()
                    .label("↻ Refresh")
                    .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
            );

        // Body per Tab
        let content = match self.view {
            View::Overview => self.render_overview(projection.as_ref(), &subs_records, palette, cx),
            View::Transactions => {
                self.render_transactions(projection.as_ref(), window, palette, cx)
            }
            View::Subscriptions => self.render_subscriptions(&subs_records, palette),
        };

        div()
            .key_context("FinancePage")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_overview_view))
            .on_action(cx.listener(Self::on_transactions_view))
            .on_action(cx.listener(Self::on_subscriptions_view))
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_refresh))
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .child(header)
            .child(div().flex_1().min_h_0().child(content))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .pt_2()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(shortcut_hint("1 / 2 / 3", "switch tab", palette))
                    .child(shortcut_hint("/", "search", palette))
                    .child(shortcut_hint("⌘r", "refresh", palette)),
            )
    }
}

impl FinancePage {
    fn render_overview(
        &self,
        projection: Option<&FinanceDashboardProjection>,
        subs_records: &[SubscriptionRecord],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (income, spend, curr) = match projection {
            Some(p) => (
                p.summary.income_cents,
                p.summary.personal_spending_cents,
                p.summary.currency.as_str(),
            ),
            None => (0, 0, "EUR"),
        };

        let net = income - spend;
        let monthly_burn = calculate_monthly_burn(subs_records);
        let active_subs = subs_records
            .iter()
            .map(SubscriptionSummary::from_record)
            .filter(|s| s.status == "active" || s.status == "trial")
            .count();

        let stat_cards = div()
            .flex()
            .gap_4()
            .child(stat_card(
                "MONTHLY SPENDING",
                format_money(-spend, curr),
                "Personal expenses",
                palette,
            ))
            .child(stat_card(
                "MONTHLY INCOME",
                format_money(income, curr),
                "Inflows & earnings",
                palette,
            ))
            .child(stat_card(
                "NET SAVINGS",
                format_money(net, curr),
                if net >= 0 {
                    "Positive net"
                } else {
                    "Negative net"
                },
                palette,
            ))
            .child(stat_card(
                "RECURRING BURN",
                format_money(-monthly_burn, curr),
                format!("{active_subs} active subscriptions"),
                palette,
            ));

        let txs = projection.map(|p| p.transactions.as_slice()).unwrap_or(&[]);

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
            .child(if txs.is_empty() {
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
                let mut list = div().flex().flex_col().gap_1();
                for tx in txs.iter().take(8) {
                    list = list.child(render_tx_row(tx, palette));
                }
                list.into_any_element()
            });

        div()
            .id("finance-overview-scroll")
            .flex()
            .flex_col()
            .gap_4()
            .overflow_y_scroll()
            .child(stat_cards)
            .child(recent_preview)
            .into_any_element()
    }

    fn render_transactions(
        &mut self,
        projection: Option<&FinanceDashboardProjection>,
        window: &mut Window,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let all_txs = projection.map(|p| p.transactions.as_slice()).unwrap_or(&[]);

        let filter = TransactionFilter {
            query: self.search_query.clone(),
            category: self.selected_category.clone(),
        };

        let filtered = filter_transactions(all_txs, &filter);

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
                                cx.notify();
                            }))
                    })),
            );

        let table_body = if filtered.is_empty() {
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
            let mut list = div().flex().flex_col().gap_1();
            for tx in filtered {
                list = list.child(render_tx_row(tx, palette));
            }
            list.into_any_element()
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
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(table_body),
            )
            .into_any_element()
    }

    fn render_subscriptions(
        &self,
        subs_records: &[SubscriptionRecord],
        palette: Palette,
    ) -> AnyElement {
        let monthly_burn = calculate_monthly_burn(subs_records);
        let summaries: Vec<_> = subs_records
            .iter()
            .map(SubscriptionSummary::from_record)
            .collect();

        let header_stat = div()
            .flex()
            .items_center()
            .justify_between()
            .p_4()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("TOTAL RECURRING SUBSCRIPTIONS"),
                    )
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child(format_money(-monthly_burn, "EUR")),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(format!("{} subscriptions tracked", summaries.len())),
            );

        let cards_grid = if summaries.is_empty() {
            div()
                .p_8()
                .rounded(px(8.))
                .border_1()
                .border_color(rgba(palette.border))
                .bg(rgba(palette.surface))
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("No subscriptions found.")
                .into_any_element()
        } else {
            let mut list = div().flex().flex_col().gap_2();
            for sub in summaries {
                let status_color = match sub.status.as_str() {
                    "active" => palette.green,
                    "trial" => palette.accent,
                    "paused" => palette.amber,
                    _ => palette.muted,
                };

                let card = div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p_3()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .bg(rgba(palette.surface))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .px_1()
                                    .py(px(1.))
                                    .rounded(px(3.))
                                    .bg(rgba((status_color & 0xffffff) | 0x22000000))
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(status_color))
                                    .child(sub.status.to_uppercase()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(palette.text))
                                            .child(sub.name),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(format!("{} · {}", sub.category, sub.cycle)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_end()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(palette.text))
                                    .child(format!(
                                        "{}/mo",
                                        format_money(-sub.monthly_equivalent_cents, &sub.currency)
                                    )),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!(
                                        "{} billed",
                                        format_money(-sub.amount_cents, &sub.currency)
                                    )),
                            ),
                    );
                list = list.child(card);
            }
            list.into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap_4()
            .size_full()
            .child(header_stat)
            .child(
                div()
                    .id("finance-subscriptions-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(cards_grid),
            )
            .into_any_element()
    }
}

fn stat_card(
    label: &'static str,
    main: impl Into<SharedString>,
    sub: impl Into<SharedString>,
    palette: Palette,
) -> AnyElement {
    div()
        .flex_1()
        .p_4()
        .rounded(px(8.))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgba(palette.surface))
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.muted))
                .child(label),
        )
        .child(
            div()
                .text_xl()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(palette.text))
                .child(main.into()),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(sub.into()),
        )
        .into_any_element()
}

fn render_tx_row(tx: &FinanceTransaction, palette: Palette) -> AnyElement {
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
                        .text_color(rgb(palette.muted))
                        .child(tx.date.clone()),
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
                .child(
                    div()
                        .px_2()
                        .py(px(2.))
                        .rounded(px(4.))
                        .bg(rgba((amount_color & 0xffffff) | 0x22000000))
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(amount_color))
                        .child(amount_text),
                ),
        )
        .into_any_element()
}

fn shortcut_hint(key: &'static str, desc: &'static str, palette: Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .px_1()
                .py(px(1.))
                .rounded(px(3.))
                .bg(rgba(palette.surface))
                .border_1()
                .border_color(rgba(palette.border))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.text))
                .child(key),
        )
        .child(div().child(desc))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn finance_tab_switching(cx: &mut TestAppContext) {
        let page = cx.new(|cx| FinancePage::new(Repo::new(Err(ShellError::NotSetUp)), cx));
        page.update(cx, |page, cx| {
            assert_eq!(page.view, View::Overview);
            page.set_view(View::Transactions, cx);
            assert_eq!(page.view, View::Transactions);
            page.set_view(View::Subscriptions, cx);
            assert_eq!(page.view, View::Subscriptions);
        });
    }
}
