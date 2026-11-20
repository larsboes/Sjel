//! Native Finance page (`/finance`): personal ledger, transaction search,
//! account breakdowns, and recurring subscription tracking.

use crate::ely::forms::{InputEvent, TextInput};
use crate::ely::theme::Density as TableDensity;
use gpui_kit::{
    actions, div, prelude::*, px, App, Context, Entity, FocusHandle, Focusable, KeyBinding,
    ScrollHandle, SharedString, Subscription, Window,
};
use sjel_gpui_core::finance::ReviewQueueMode;
use sjel_gpui_core::shell::{
    self, BalanceCoverage, FinanceCandidate, FinanceCsvMappingProfile, FinanceCsvPreview,
    FinanceDashboardProjection, ShellError, SubscriptionRecord,
};

use crate::data::{Remote, Repo};
use crate::primitives::{column_width, key_bar, sub_tabs};
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverviewSection {
    #[default]
    CashFlow,
    Holdings,
    Balances,
    Recent,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImportSubTab {
    #[default]
    Candidates,
    Categorize,
    SharedTrips,
    Reimbursements,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IdeaDetailTab {
    #[default]
    ThesisLevels,
    ReviewResearch,
    NotesHistory,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TxPeriod {
    #[default]
    All,
    ThisMonth,
    LastMonth,
    ThreeMonths,
}

impl TxPeriod {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::ThisMonth => "This month",
            Self::LastMonth => "Last month",
            Self::ThreeMonths => "3M",
        }
    }
}

mod ideas;
mod import;
mod overview;
mod subscriptions;
mod transactions;

use import::{default_csv_mapping, CsvMappingField, ImportDerived};
use overview::BalanceDraft;
use subscriptions::SubsDerived;

pub struct FinancePage {
    pub(super) repo: Repo,
    pub(super) palette: Option<Palette>,
    pub view: View,
    pub overview_section: OverviewSection,
    pub import_tab: ImportSubTab,
    pub csv_sheet_open: bool,
    pub tx_hidden: Vec<SharedString>,
    pub tx_density: TableDensity,
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
    pub(super) reclass_input: Option<(Entity<TextInput>, Subscription)>,
    pub(super) queue_mode: ReviewQueueMode,
    pub(super) review_trip_id: String,
    pub(super) selected_expense: Option<String>,
    pub(super) allocation_purpose: String,
    pub(super) personal_amount: String,
    pub(super) personal_input: Option<(Entity<TextInput>, Subscription)>,
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
    pub(super) balance_date_input: Option<(Entity<crate::ely::forms::TextInput>, Subscription)>,
    pub(super) balance_currency_input: Option<(Entity<crate::ely::forms::TextInput>, Subscription)>,
    pub(super) balance_fields: Vec<BalanceDraft>,
    pub search_query: String,
    pub selected_category: Option<String>,
    pub selected_account: Option<String>,
    pub tx_period: TxPeriod,
    pub tx_charts_open: bool,
    pub sub_status_filter: Option<&'static str>,
    pub(super) import_rows: std::rc::Rc<Vec<crate::ely::tables::Row>>,
    pub(super) import_rows_key: Option<(Option<std::time::Instant>, u64, usize, Option<String>)>,
    pub(super) col_widths: std::collections::HashMap<&'static str, f32>,
    pub(super) filter_rev: u64,
    pub(super) review_rev: u64,
    pub(super) tx_rows: std::rc::Rc<Vec<crate::ely::tables::Row>>,
    pub(super) tx_scroll: gpui_kit::UniformListScrollHandle,
    pub(super) tx_rows_key: Option<(Option<std::time::Instant>, u64)>,
    pub(super) subs_derived: SubsDerived,
    pub(super) import_derived: ImportDerived,
    pub overview_scroll: ScrollHandle,
    pub subscriptions_scroll: ScrollHandle,
    pub import_scroll: ScrollHandle,
    pub(super) search_input: Option<(Entity<TextInput>, Subscription)>,
    pub(super) focus: FocusHandle,
    pub(super) focus_next: bool,
    pub(super) ideas: Remote<Vec<shell::FinanceIdea>>,
    pub(super) selected_idea: Option<String>,
    pub(super) idea_inputs: Option<ideas::IdeaInputs>,
    pub(super) thesis_for: Option<String>,
    pub(super) new_idea_stance: &'static str,
    pub(super) level_direction: &'static str,
    pub ideas_scroll: ScrollHandle,
    pub(super) idea_detail_tab: IdeaDetailTab,
    pub(super) new_idea_open: bool,
    pub(super) reviewing: bool,
    pub(super) reclass_highlight: usize,
}

impl FinancePage {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self {
            repo,
            palette: None,
            view: View::Overview,
            overview_section: OverviewSection::default(),
            import_tab: ImportSubTab::default(),
            csv_sheet_open: false,
            tx_hidden: Vec::new(),
            tx_density: TableDensity::Standard,
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
            selected_account: None,
            tx_period: TxPeriod::All,
            tx_charts_open: true,
            sub_status_filter: None,
            import_rows: Default::default(),
            import_rows_key: None,
            col_widths: Default::default(),
            filter_rev: 0,
            review_rev: 0,
            tx_rows: Default::default(),
            tx_scroll: gpui_kit::UniformListScrollHandle::new(),
            tx_rows_key: None,
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
            idea_detail_tab: IdeaDetailTab::default(),
            new_idea_open: false,
            reviewing: false,
            reclass_highlight: 0,
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
            let rows = self.tx_rows.len();
            let row_ix = (rows.saturating_sub(1) as f32 * fraction).round() as usize;
            self.tx_scroll
                .scroll_to_item(row_ix, gpui_kit::ScrollStrategy::Top);
            cx.notify();
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

    pub fn badge_counts(&self) -> (Option<usize>, Option<usize>, Option<usize>, Option<usize>) {
        let tx_count = self
            .dashboard
            .value
            .as_ref()
            .map(|p| p.transactions.len());
        let subs_count = Some(self.subs_derived.active_count);
        let import_count = Some(
            self.import_derived.pending_indices.len() + self.import_derived.uncategorized.len(),
        );
        let ideas_count = self.ideas.value.as_ref().map(|ideas| {
            ideas
                .iter()
                .filter(|idea| idea.current.status == "open")
                .count()
        });
        (tx_count, subs_count, import_count, ideas_count)
    }

    pub fn open_csv_import(&mut self, cx: &mut Context<Self>) {
        self.view = View::ImportReview;
        self.csv_sheet_open = true;
        cx.notify();
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

    pub(super) fn col_width(&self, id: &'static str, default: f32) -> f32 {
        column_width(&self.col_widths, id, default)
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

    fn search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<TextInput> {
        if let Some((input, _)) = &self.search_input {
            return input.clone();
        }
        let input = cx.new(|cx| TextInput::new(window, cx).placeholder("Search transactions…"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Changed) {
                view.search_query = input.read(cx).text().trim().to_lowercase();
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
        field.update(cx, |input, cx| window.focus(&input.focus_handle(cx), cx));
    }

    pub fn open_date(&mut self, date: String, window: &mut Window, cx: &mut Context<Self>) {
        self.set_view(View::Transactions, cx);
        self.search_query = date.clone();
        self.filter_rev = self.filter_rev.wrapping_add(1);
        let field = self.search_field(window, cx);
        field.update(cx, |input, cx| input.set_text(date, cx));
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

        self.sync_subs_derived();
        self.sync_import_derived();
        let show_inline_tabs = cfg!(test);
        let inline_header = show_inline_tabs.then(|| {
            let (tx_count, subs_count, import_count, ideas_count) = self.badge_counts();
            let entity = cx.entity();
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_4()
                .child(sub_tabs(
                    "finance",
                    vec![
                        ("Overview", None, self.view == View::Overview),
                        ("Transactions", tx_count, self.view == View::Transactions),
                        ("Subscriptions", subs_count, self.view == View::Subscriptions),
                        (
                            "Import & Review",
                            import_count,
                            self.view == View::ImportReview,
                        ),
                        ("Ideas", ideas_count, self.view == View::Ideas),
                    ],
                    move |idx, _, cx| {
                        entity.update(cx, |page, cx| {
                            let target = match idx {
                                0 => View::Overview,
                                1 => View::Transactions,
                                2 => View::Subscriptions,
                                3 => View::ImportReview,
                                _ => View::Ideas,
                            };
                            page.set_view(target, cx);
                        });
                    },
                ))
        });

        // Body per Tab
        let content = match self.view {
            View::Overview => self.render_overview(palette, cx),
            View::Transactions => self.render_transactions(window, cx),
            View::Subscriptions => self.render_subscriptions(palette, cx),
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
            .children(inline_header)
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
    fn transaction_search_uses_ely_input_events_and_survives_navigation(cx: &mut TestAppContext) {
        let (page, cx) = crate::test_support::mount_root(cx, None, |_, cx| {
            let mut page = FinancePage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
            page.set_view(View::Transactions, cx);
            page
        });
        cx.update(|window, cx| page.update(cx, |page, cx| page.focus_search(window, cx)));
        cx.simulate_input("  Grocery  ");
        cx.run_until_parked();
        page.read_with(cx, |page, _| assert_eq!(page.search_query, "grocery"));
        page.update(cx, |page, cx| page.set_view(View::Overview, cx));
        cx.run_until_parked();
        page.update(cx, |page, cx| page.set_view(View::Transactions, cx));
        cx.run_until_parked();
        page.read_with(cx, |page, cx| {
            assert_eq!(
                page.search_input.as_ref().unwrap().0.read(cx).text(),
                "  Grocery  "
            )
        });
    }

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

    #[gpui::test]
    fn finance_sub_tabs_switch_overview_and_import_sections(cx: &mut TestAppContext) {
        let (page, cx) = crate::test_support::mount_root(cx, None, |_, cx| {
            let mut page = FinancePage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
            page.overview_section = OverviewSection::Holdings;
            page
        });
        let overview_tabs = cx
            .debug_bounds("finance-overview-section-tabs")
            .expect("overview sub-tabs render");
        cx.simulate_click(
            overview_tabs.origin + gpui_kit::point(px(12.), px(10.)),
            gpui_kit::Modifiers::none(),
        );
        cx.run_until_parked();
        page.read_with(cx, |page, _| {
            assert_eq!(page.overview_section, OverviewSection::CashFlow);
        });
        page.update(cx, |page, cx| {
            page.import_tab = ImportSubTab::Categorize;
            page.set_view(View::ImportReview, cx);
        });
        cx.run_until_parked();
        let import_tabs = cx
            .debug_bounds("finance-import-tabs")
            .expect("import sub-tabs render");
        cx.simulate_click(
            import_tabs.origin + gpui_kit::point(px(12.), px(10.)),
            gpui_kit::Modifiers::none(),
        );
        cx.run_until_parked();
        page.read_with(cx, |page, _| {
            assert_eq!(page.import_tab, ImportSubTab::Candidates);
        });
    }
}
