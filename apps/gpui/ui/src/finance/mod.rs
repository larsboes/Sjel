//! Native Finance page (`/finance`): personal ledger, transaction search,
//! account breakdowns, and recurring subscription tracking.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{
    actions, div, prelude::*, px, App, Context, Entity, FocusHandle, KeyBinding, ScrollHandle,
    SharedString, Subscription, Window,
};
use sjel_gpui_core::finance::ReviewQueueMode;
use sjel_gpui_core::shell::{
    self, BalanceCoverage, FinanceCandidate, FinanceCsvMappingProfile, FinanceCsvPreview,
    FinanceDashboardProjection, ShellError, SubscriptionRecord,
};

use crate::data::{Remote, Repo};
use crate::primitives::key_bar;
use crate::theme::{Palette, ThemeContext};

actions!(
    finance_page,
    [
        /// Show the overview tab.
        OverviewView,
        /// Show the transactions ledger tab.
        TransactionsView,
        /// Show the subscriptions tab.
        SubscriptionsView,
        /// Show CSV import and candidate review.
        ImportReviewView,
        /// Show investment ideas.
        IdeasView,
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
        KeyBinding::new("4", ImportReviewView, Some("FinancePage")),
        KeyBinding::new("5", IdeasView, Some("FinancePage")),
        KeyBinding::new("/", FocusSearch, Some("FinancePage")),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Overview,
    Transactions,
    Subscriptions,
    ImportReview,
    Ideas,
}

mod ideas;
mod import;
mod overview;
mod subscriptions;
mod transactions;

use import::{default_csv_mapping, CsvMappingField, ImportDerived};
use overview::BalanceDraft;
use subscriptions::SubsDerived;
use transactions::TxDerived;

pub struct FinancePage {
    pub(super) repo: Repo,
    pub(super) palette: Option<Palette>,
    pub view: View,
    pub dashboard: Remote<FinanceDashboardProjection>,
    pub subscriptions: Remote<Vec<SubscriptionRecord>>,
    pub(super) trip_plans: Remote<Vec<sjel_gpui_core::travel::Plan>>,
    pub(super) candidates: Remote<Vec<FinanceCandidate>>,
    pub(super) csv_mappings: Remote<Vec<FinanceCsvMappingProfile>>,
    pub(super) notice: Option<String>,
    pub(super) csv_file: Option<(String, String)>,
    pub(super) csv_preview: Option<FinanceCsvPreview>,
    pub(super) selected_mapping: usize,
    pub(super) manual_csv_mapping: serde_json::Value,
    pub(super) csv_mapping_fields: Option<Vec<CsvMappingField>>,
    pub(super) selected_reclass_candidate: Option<String>,
    pub(super) reclass_account: String,
    pub(super) reclass_input: Option<(Entity<InputState>, Subscription)>,
    pub(super) queue_mode: ReviewQueueMode,
    pub(super) review_trip_id: String,
    pub(super) selected_expense: Option<String>,
    pub(super) allocation_purpose: String,
    pub(super) personal_amount: String,
    pub(super) personal_input: Option<(Entity<InputState>, Subscription)>,
    pub(super) selected_trip_id: String,
    pub(super) selected_inflow: Option<String>,
    pub(super) reimbursement_target: Option<String>,
    pub(super) pending_limit: usize,
    pub(super) category_limit: usize,
    pub(super) expense_limit: usize,
    pub(super) inflow_limit: usize,
    pub(super) receivable_limit: usize,
    pub(super) balance_editor_open: bool,
    pub(super) balance_as_of: String,
    pub(super) balance_currency: String,
    pub(super) balance_coverage: BalanceCoverage,
    pub(super) balance_date_input: Option<(Entity<InputState>, Subscription)>,
    pub(super) balance_currency_input: Option<(Entity<InputState>, Subscription)>,
    pub(super) balance_fields: Vec<BalanceDraft>,
    pub search_query: String,
    pub selected_category: Option<String>,
    pub(super) filter_rev: u64,
    pub(super) review_rev: u64,
    pub(super) tx_derived: TxDerived,
    pub(super) tx_list: gpui::ListState,
    pub(super) subs_derived: SubsDerived,
    pub(super) import_derived: ImportDerived,
    pub overview_scroll: ScrollHandle,
    pub subscriptions_scroll: ScrollHandle,
    pub import_scroll: ScrollHandle,
    pub(super) search_input: Option<(Entity<InputState>, Subscription)>,
    pub(super) focus: FocusHandle,
    pub(super) focus_next: bool,
    pub(super) ideas: Remote<Vec<shell::FinanceIdea>>,
    pub(super) selected_idea: Option<String>,
    pub(super) idea_inputs: Option<ideas::IdeaInputs>,
    pub(super) thesis_for: Option<String>,
    pub(super) new_idea_stance: &'static str,
    pub(super) level_direction: &'static str,
    pub ideas_scroll: ScrollHandle,
}

impl FinancePage {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self {
            repo,
            palette: None,
            view: View::Overview,
            dashboard: Remote::default(),
            subscriptions: Remote::default(),
            trip_plans: Remote::default(),
            candidates: Remote::default(),
            csv_mappings: Remote::default(),
            notice: None,
            csv_file: None,
            csv_preview: None,
            selected_mapping: 0,
            manual_csv_mapping: default_csv_mapping(),
            csv_mapping_fields: None,
            selected_reclass_candidate: None,
            reclass_account: String::new(),
            reclass_input: None,
            queue_mode: ReviewQueueMode::Unreviewed,
            review_trip_id: String::new(),
            selected_expense: None,
            allocation_purpose: "day_to_day".into(),
            personal_amount: String::new(),
            personal_input: None,
            selected_trip_id: String::new(),
            selected_inflow: None,
            reimbursement_target: None,
            pending_limit: 24,
            category_limit: 16,
            expense_limit: 16,
            inflow_limit: 8,
            receivable_limit: 8,
            balance_editor_open: false,
            balance_as_of: String::new(),
            balance_currency: "EUR".into(),
            balance_coverage: BalanceCoverage::Partial,
            balance_date_input: None,
            balance_currency_input: None,
            balance_fields: Vec::new(),
            search_query: String::new(),
            selected_category: None,
            filter_rev: 0,
            review_rev: 0,
            tx_derived: TxDerived::default(),
            tx_list: gpui::ListState::new(0, gpui::ListAlignment::Top, px(600.)),
            subs_derived: SubsDerived::default(),
            import_derived: ImportDerived::default(),
            overview_scroll: ScrollHandle::new(),
            subscriptions_scroll: ScrollHandle::new(),
            import_scroll: ScrollHandle::new(),
            search_input: None,
            focus: cx.focus_handle(),
            focus_next: true,
            ideas: Remote::default(),
            selected_idea: None,
            idea_inputs: None,
            thesis_for: None,
            new_idea_stance: "watch",
            level_direction: "above",
            ideas_scroll: ScrollHandle::new(),
        }
    }

    pub fn active_scroll(&self) -> Option<&ScrollHandle> {
        match self.view {
            View::Overview => Some(&self.overview_scroll),
            View::Transactions => None,
            View::Subscriptions => Some(&self.subscriptions_scroll),
            View::ImportReview => Some(&self.import_scroll),
            View::Ideas => Some(&self.ideas_scroll),
        }
    }

    pub fn bench_scroll(&mut self, fraction: f32, cx: &mut Context<Self>) {
        if self.view == View::Transactions {
            self.sync_tx_derived();
            let rows = self.tx_derived.indices.len();
            let item_ix = (rows.saturating_sub(1) as f32 * fraction).round() as usize;
            self.tx_list.scroll_to(gpui::ListOffset {
                item_ix,
                offset_in_item: px(0.),
            });
        } else if let Some(scroll) = self.active_scroll() {
            let max = scroll.max_offset().y;
            scroll.set_offset(gpui_kit::point(px(0.), -max * fraction));
        }
        cx.notify();
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
        if self.ideas.value.is_some() {
            self.load_ideas(cx);
        }

        self.dashboard.loading = true;
        self.subscriptions.loading = true;
        self.trip_plans.loading = true;
        self.candidates.loading = true;
        self.csv_mappings.loading = true;
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
        let shell_candidates = shell.clone();
        let candidates_work =
            shell::run(async move { shell_candidates.finance_candidates().await });
        cx.spawn(async move |this, cx| {
            let res = candidates_work
                .await
                .unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.candidates.settle(res);
                cx.notify();
            });
        })
        .detach();

        let shell_mappings = shell.clone();
        let mappings_work = shell::run(async move { shell_mappings.finance_csv_mappings().await });
        cx.spawn(async move |this, cx| {
            let res = mappings_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.csv_mappings.settle(res);
                cx.notify();
            });
        })
        .detach();

        let shell_subs = shell;
        let shell_plans = shell_subs.clone();
        let plans_work = shell::run(async move {
            match shell_plans.travel_plans().await {
                Ok(plans) => Ok(plans),
                Err(_) => {
                    shell_plans.set_running("trips", true).await?;
                    shell_plans.travel_plans().await
                }
            }
        });
        cx.spawn(async move |this, cx| {
            let res = plans_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.trip_plans.settle(res);
                cx.notify();
            });
        })
        .detach();
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
                view.filter_rev = view.filter_rev.wrapping_add(1);
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

    fn on_ideas_view(&mut self, _: &IdeasView, _: &mut Window, cx: &mut Context<Self>) {
        self.set_view(View::Ideas, cx);
    }

    fn on_import_review_view(
        &mut self,
        _: &ImportReviewView,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_view(View::ImportReview, cx);
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_view(View::Transactions, cx);
        let field = self.search_field(window, cx);
        field.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn open_date(&mut self, date: String, window: &mut Window, cx: &mut Context<Self>) {
        self.set_view(View::Transactions, cx);
        self.search_query = date.clone();
        self.filter_rev = self.filter_rev.wrapping_add(1);
        let field = self.search_field(window, cx);
        field.update(cx, |input, cx| input.set_value(date, window, cx));
        cx.notify();
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

        let palette = self
            .palette
            .unwrap_or_else(|| ThemeContext::of(window, cx).palette);

        let view_btn = |label: &'static str, target: View, cx: &mut Context<Self>| {
            Button::new(SharedString::from(format!("fin-view-{label}")))
                .small()
                .label(label)
                .when(self.view == target, |b| b.primary())
                .when(self.view != target, |b| b.ghost())
                .on_click(cx.listener(move |page, _, _, cx| page.set_view(target, cx)))
        };

        // Header with clean sub-tabs (overview/transactions/subscriptions/import)
        let header = div().flex().items_center().justify_between().gap_4().child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(view_btn("Overview", View::Overview, cx))
                .child(view_btn("Transactions", View::Transactions, cx))
                .child(view_btn("Subscriptions", View::Subscriptions, cx))
                .child(view_btn("Import & Review", View::ImportReview, cx))
                .child(view_btn("Ideas", View::Ideas, cx)),
        );

        // Body per Tab
        let content = match self.view {
            View::Overview => self.render_overview(palette, cx),
            View::Transactions => self.render_transactions(window, palette, cx),
            View::Subscriptions => self.render_subscriptions(palette),
            View::ImportReview => self.render_import_review(palette, window, cx),
            View::Ideas => self.render_ideas(window, palette, cx),
        };

        div()
            .key_context("FinancePage")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_overview_view))
            .on_action(cx.listener(Self::on_transactions_view))
            .on_action(cx.listener(Self::on_subscriptions_view))
            .on_action(cx.listener(Self::on_import_review_view))
            .on_action(cx.listener(Self::on_ideas_view))
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_refresh))
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .size_full()
                    .overflow_hidden()
                    .child(content),
            )
            .child(key_bar(
                &[("1 – 5", "switch tab"), ("/", "search"), ("⌘r", "refresh")],
                palette,
            ))
    }
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
