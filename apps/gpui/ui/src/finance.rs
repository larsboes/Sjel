//! Native Finance page (`/finance`): personal ledger, transaction search,
//! account breakdowns, and recurring subscription tracking.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, AnyElement, App, ClickEvent, Context, Entity,
    FocusHandle, FontWeight, KeyBinding, PathPromptOptions, SharedString, Subscription, Window,
};
use sjel_gpui_core::context::format_money;
use sjel_gpui_core::finance::{
    calculate_monthly_burn, filter_transactions, format_exact_money, parse_personal_cents,
    preset_share_cents, prioritized_expense_queue, reviewable_expenses, selected_csv_mapping,
    ExpenseReview, ReviewQueueMode, ReviewScope, SubscriptionSummary, TransactionFilter,
};
use sjel_gpui_core::shell::{
    self, BalanceCoverage, BalanceKind, FinanceCandidate, FinanceCsvMappingProfile,
    FinanceCsvPreview, FinanceDashboardProjection, FinanceTransaction, ManualBalance,
    ManualBalanceUpdate, ShellError, SubscriptionRecord,
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
        /// Show CSV import and candidate review.
        ImportReviewView,
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
        KeyBinding::new("/", FocusSearch, Some("FinancePage")),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Overview,
    Transactions,
    Subscriptions,
    ImportReview,
}

struct BalanceDraft {
    id: String,
    label: Entity<InputState>,
    amount: Entity<InputState>,
    kind: BalanceKind,
}

struct CsvMappingField {
    label: &'static str,
    input: Entity<InputState>,
    _subscription: Subscription,
}

pub struct FinancePage {
    repo: Repo,
    palette: Option<Palette>,
    pub view: View,
    pub dashboard: Remote<FinanceDashboardProjection>,
    pub subscriptions: Remote<Vec<SubscriptionRecord>>,
    trip_plans: Remote<Vec<sjel_gpui_core::travel::Plan>>,
    candidates: Remote<Vec<FinanceCandidate>>,
    csv_mappings: Remote<Vec<FinanceCsvMappingProfile>>,
    notice: Option<String>,
    csv_file: Option<(String, String)>,
    csv_preview: Option<FinanceCsvPreview>,
    selected_mapping: usize,
    manual_csv_mapping: serde_json::Value,
    csv_mapping_fields: Option<Vec<CsvMappingField>>,
    selected_reclass_candidate: Option<String>,
    reclass_account: String,
    reclass_input: Option<(Entity<InputState>, Subscription)>,
    queue_mode: ReviewQueueMode,
    review_trip_id: String,
    selected_expense: Option<String>,
    allocation_purpose: String,
    personal_amount: String,
    personal_input: Option<(Entity<InputState>, Subscription)>,
    selected_trip_id: String,
    selected_inflow: Option<String>,
    reimbursement_target: Option<String>,
    pending_limit: usize,
    category_limit: usize,
    expense_limit: usize,
    inflow_limit: usize,
    receivable_limit: usize,
    balance_editor_open: bool,
    balance_as_of: String,
    balance_currency: String,
    balance_coverage: BalanceCoverage,
    balance_date_input: Option<(Entity<InputState>, Subscription)>,
    balance_currency_input: Option<(Entity<InputState>, Subscription)>,
    balance_fields: Vec<BalanceDraft>,
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
                cx.notify();
            }
        });
        self.search_input = Some((input.clone(), subscription));
        input
    }

    fn reclass_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
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

    fn personal_field(
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

    fn reclassify_selected(&mut self, cx: &mut Context<Self>) {
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

    fn select_reclass_candidate(
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

    fn select_expense(
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

    fn set_personal_share(&mut self, percent: u32, window: &mut Window, cx: &mut Context<Self>) {
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

    fn save_allocation(&mut self, cx: &mut Context<Self>) {
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

    fn link_reimbursement(&mut self, cx: &mut Context<Self>) {
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

    fn balance_draft(
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

    fn open_balance_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn add_balance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn save_balance_snapshot(&mut self, cx: &mut Context<Self>) {
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

    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_search(window, cx);
    }

    fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh(cx);
    }

    fn active_csv_mapping(&self) -> serde_json::Value {
        let profiles = self
            .csv_mappings
            .value
            .as_deref()
            .unwrap_or_default()
            .to_vec();
        profiles
            .get(self.selected_mapping)
            .map(|profile| profile.mapping.clone())
            .unwrap_or_else(|| self.manual_csv_mapping.clone())
    }

    fn ensure_csv_mapping_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.csv_mapping_fields.is_some() {
            return;
        }
        let fields = [
            ("delimiter", "Delimiter"),
            ("decimal_separator", "Decimal separator"),
            ("date_column", "Date column"),
            ("amount_column", "Amount column"),
            ("description_column", "Description column"),
            ("reference_column", "Reference column (optional)"),
            ("currency_column", "Currency column (optional)"),
            ("default_currency", "Default currency"),
            ("source_account", "Source account"),
            ("default_outflow_account", "Default outflow account"),
            ("default_inflow_account", "Default inflow account"),
        ];
        self.csv_mapping_fields = Some(
            fields
                .into_iter()
                .map(|(key, label)| {
                    let initial = self
                        .manual_csv_mapping
                        .get(key)
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let input = cx.new(|cx| {
                        let mut state = InputState::new(window, cx).placeholder(label);
                        state.set_value(initial, window, cx);
                        state
                    });
                    let subscription =
                        cx.subscribe_in(&input, window, move |page, input, event, _, cx| {
                            if matches!(event, InputEvent::Change) {
                                let value = input.read(cx).value().to_string();
                                if let Some(object) = page.manual_csv_mapping.as_object_mut() {
                                    object.insert(
                                        key.to_owned(),
                                        if matches!(key, "currency_column" | "reference_column")
                                            && value.trim().is_empty()
                                        {
                                            serde_json::Value::Null
                                        } else {
                                            serde_json::Value::String(value)
                                        },
                                    );
                                }
                                page.csv_preview = None;
                                cx.notify();
                            }
                        });
                    CsvMappingField {
                        label,
                        input,
                        _subscription: subscription,
                    }
                })
                .collect(),
        );
    }

    fn review_candidate(
        &mut self,
        id: String,
        decision: &'static str,
        source_account: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let account = self
            .candidates
            .value
            .as_ref()
            .and_then(|items| items.iter().find(|candidate| candidate.id == id))
            .map(|candidate| candidate.proposed_account.clone());
        let work = shell::run(async move {
            shell
                .finance_review_candidate(
                    &id,
                    decision,
                    account.as_deref(),
                    source_account.as_deref(),
                )
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                if let Err(error) = result {
                    page.notice = Some(error.to_string());
                }
                page.load(cx);
            });
        })
        .detach();
    }

    fn reconcile_candidate(&mut self, id: String, counterpart: String, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let work =
            shell::run(async move { shell.finance_reconcile_transfer(&id, &counterpart).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                if let Err(error) = result {
                    page.notice = Some(error.to_string());
                }
                page.load(cx);
            });
        })
        .detach();
    }

    fn choose_csv(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose a CSV export".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("CSV")
                .to_owned();
            let (tx, rx) = futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let _ = tx.send(std::fs::read_to_string(path));
            });
            let loaded = rx.await.ok().and_then(Result::ok);
            let _ = this.update(cx, |page, cx| {
                if let Some(content) = loaded {
                    page.csv_file = Some((name, content));
                    page.csv_preview = None;
                    page.notice = None;
                } else {
                    page.notice = Some("Could not read the selected file as UTF-8 text.".into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn preview_csv(&mut self, cx: &mut Context<Self>) {
        let Some((_, content)) = self.csv_file.clone() else {
            return;
        };
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let mapping = selected_csv_mapping(&self.active_csv_mapping());
        let work = shell::run(async move { shell.finance_preview_csv(content, mapping).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                match result {
                    Ok(preview) => {
                        page.csv_preview = Some(preview);
                        page.notice = None;
                    }
                    Err(error) => page.notice = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn stage_csv(&mut self, cx: &mut Context<Self>) {
        let (Some((_, content)), Some(preview)) = (self.csv_file.clone(), self.csv_preview.clone())
        else {
            return;
        };
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let mapping = selected_csv_mapping(&self.active_csv_mapping());
        let work = shell::run(async move {
            shell
                .finance_import_csv(content, mapping, preview.preview_id)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                match result {
                    Ok(value) => {
                        page.csv_file = None;
                        page.csv_preview = None;
                        page.notice = Some(format!(
                            "{} new candidates staged.",
                            value["created"].as_u64().unwrap_or_default()
                        ));
                        page.load(cx);
                    }
                    Err(error) => page.notice = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
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
                            .child(view_btn("Subscriptions", View::Subscriptions, cx))
                            .child(view_btn("Import & Review", View::ImportReview, cx)),
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
            View::ImportReview => self.render_import_review(palette, window, cx),
        };

        div()
            .key_context("FinancePage")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_overview_view))
            .on_action(cx.listener(Self::on_transactions_view))
            .on_action(cx.listener(Self::on_subscriptions_view))
            .on_action(cx.listener(Self::on_import_review_view))
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
    fn render_import_review(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let candidates = self
            .candidates
            .value
            .as_deref()
            .unwrap_or_default()
            .to_vec();
        let pending_count = candidates
            .iter()
            .filter(|candidate| candidate.state == "pending")
            .count();
        let pending = candidates
            .iter()
            .filter(|candidate| candidate.state == "pending")
            .take(self.pending_limit);
        let mut queue = div().flex().flex_col().gap_2();
        for candidate in pending {
            let id = candidate.id.clone();
            let reject_id = id.clone();
            let exact_transfer = candidate.transfer_match_ids.len() == 1;
            let transfer = exact_transfer.then(|| candidate.transfer_match_ids[0].clone());
            let is_mail = candidate
                .source_reference
                .as_deref()
                .is_some_and(|reference| reference.starts_with("gmail:"));
            let mail_source = self
                .csv_mappings
                .value
                .as_ref()
                .and_then(|profiles| profiles.get(self.selected_mapping))
                .and_then(|profile| profile.mapping.get("source_account"))
                .and_then(serde_json::Value::as_str)
                .filter(|account| {
                    account.starts_with("assets:") || account.starts_with("liabilities:")
                })
                .map(str::to_owned);
            let mut row = div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .p_3()
                .rounded(px(7.))
                .border_1()
                .border_color(rgba(palette.border))
                .bg(rgba(palette.surface))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(candidate.description.clone())
                        .child(format!(
                            "{} · {} · {}",
                            candidate.booked_at,
                            candidate.proposed_account,
                            format_money(candidate.amount_cents, &candidate.currency)
                        )),
                );
            if let Some(counterpart) = transfer {
                let reconcile_id = id.clone();
                row = row.child(
                    Button::new(SharedString::from(format!(
                        "finance-transfer-{}",
                        candidate.id
                    )))
                    .small()
                    .ghost()
                    .label("Reconcile transfer")
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.reconcile_candidate(reconcile_id.clone(), counterpart.clone(), cx)
                    })),
                );
            }
            if !exact_transfer && !is_mail {
                let confirm_id = id.clone();
                row = row.child(
                    Button::new(SharedString::from(format!(
                        "finance-confirm-{}",
                        candidate.id
                    )))
                    .small()
                    .primary()
                    .label("Confirm proposed account")
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.review_candidate(confirm_id.clone(), "confirm", None, cx)
                    })),
                );
            } else if !exact_transfer && is_mail {
                if let Some(source_account) = mail_source {
                    let confirm_id = id.clone();
                    row = row.child(
                        Button::new(SharedString::from(format!(
                            "finance-confirm-mail-{}",
                            candidate.id
                        )))
                        .small()
                        .primary()
                        .label(format!("Confirm from {source_account}"))
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.review_candidate(
                                confirm_id.clone(),
                                "confirm",
                                Some(source_account.clone()),
                                cx,
                            )
                        })),
                    );
                }
            }
            row = row.child(
                Button::new(SharedString::from(format!(
                    "finance-reject-{}",
                    candidate.id
                )))
                .small()
                .ghost()
                .label("Reject")
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.review_candidate(reject_id.clone(), "reject", None, cx)
                })),
            );
            queue = queue.child(row);
        }
        if pending_count > self.pending_limit {
            queue = queue.child(
                Button::new("finance-pending-more")
                    .small()
                    .ghost()
                    .label("Show 24 more candidates")
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.pending_limit += 24;
                        cx.notify();
                    })),
            );
        }

        let uncategorized: Vec<_> = candidates
            .iter()
            .filter(|candidate| {
                candidate.state == "confirmed"
                    && candidate.amount_cents < 0
                    && candidate
                        .proposed_account
                        .split(':')
                        .any(|part| part == "uncategorized")
            })
            .cloned()
            .collect();
        let mut category_rows = div().flex().flex_col().gap_1();
        for candidate in uncategorized.iter().take(self.category_limit) {
            let candidate_id = candidate.id.clone();
            category_rows = category_rows.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .p_2()
                    .rounded(px(5.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(candidate.description.clone())
                            .child(format!(
                                "{} · {}",
                                candidate.booked_at,
                                format_money(candidate.amount_cents, &candidate.currency)
                            )),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "finance-select-category-{}",
                            candidate.id
                        )))
                        .small()
                        .ghost()
                        .label(
                            if self.selected_reclass_candidate.as_deref() == Some(&candidate.id) {
                                "Selected"
                            } else {
                                "Choose"
                            },
                        )
                        .on_click(cx.listener(
                            move |page, _, window, cx| {
                                page.select_reclass_candidate(candidate_id.clone(), window, cx)
                            },
                        )),
                    ),
            );
        }
        let category_input = Input::new(&self.reclass_field(window, cx)).small();
        let categorize_panel = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(7.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child("Recategorize confirmed expenses"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(format!(
                        "{} confirmed expenses need a category",
                        uncategorized.len()
                    )),
            )
            .child(category_rows)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(category_input)
                    .child(
                        Button::new("finance-reclassify-selected")
                            .small()
                            .primary()
                            .label("Reclassify selected")
                            .when(
                                self.selected_reclass_candidate.is_none()
                                    || self.reclass_account.trim().is_empty(),
                                |button| button.disabled(true),
                            )
                            .on_click(cx.listener(|page, _, _, cx| page.reclassify_selected(cx))),
                    ),
            );
        let categorize_panel = if uncategorized.len() > self.category_limit {
            categorize_panel.child(
                Button::new("finance-category-more")
                    .small()
                    .ghost()
                    .label("Show more uncategorized")
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.category_limit += 16;
                        cx.notify();
                    })),
            )
        } else {
            categorize_panel
        };

        let transactions = self
            .dashboard
            .value
            .as_ref()
            .map(|dashboard| dashboard.transactions.clone())
            .unwrap_or_default();
        let eligible = reviewable_expenses(
            &candidates,
            &transactions,
            &ReviewScope {
                start: String::new(),
                end: "9999-12-31".into(),
                account: String::new(),
                category: String::new(),
            },
        );
        let focused_plan = self
            .trip_plans
            .value
            .as_ref()
            .and_then(|plans| plans.iter().find(|plan| plan.id == self.review_trip_id));
        let review_queue = prioritized_expense_queue(
            &eligible,
            self.queue_mode,
            focused_plan.map(|plan| (plan.date_start.as_str(), plan.date_end.as_str())),
        );
        let mut expense_rows = div().flex().flex_col().gap_1();
        for entry in review_queue.iter().take(self.expense_limit) {
            let selected = self.selected_expense.as_deref() == Some(&entry.candidate.id);
            let selected_entry = entry.clone();
            expense_rows = expense_rows.child(
                Button::new(SharedString::from(format!(
                    "finance-expense-{}",
                    entry.candidate.id
                )))
                .small()
                .when(selected, |button| button.primary())
                .when(!selected, |button| button.ghost())
                .label(format!(
                    "{} · {} · {}",
                    entry.candidate.booked_at,
                    entry.candidate.description,
                    format_money(entry.total_cents, &entry.candidate.currency)
                ))
                .on_click(cx.listener(move |page, _, window, cx| {
                    page.select_expense(&selected_entry, window, cx)
                })),
            );
        }
        let personal_input = Input::new(&self.personal_field(window, cx)).small();
        let share_is_valid = self
            .selected_expense
            .as_ref()
            .and_then(|id| candidates.iter().find(|candidate| &candidate.id == id))
            .and_then(|candidate| {
                parse_personal_cents(&self.personal_amount)
                    .map(|cents| cents <= candidate.amount_cents.saturating_abs())
            })
            .unwrap_or(false);
        let mut allocation = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(7.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child("Purpose and shared-cost review"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("finance-mode-unreviewed")
                            .xsmall()
                            .label("Needs purpose")
                            .when(self.queue_mode == ReviewQueueMode::Unreviewed, |button| {
                                button.primary()
                            })
                            .when(self.queue_mode != ReviewQueueMode::Unreviewed, |button| {
                                button.ghost()
                            })
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.queue_mode = ReviewQueueMode::Unreviewed;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("finance-mode-shared")
                            .xsmall()
                            .label("Shared costs")
                            .when(self.queue_mode == ReviewQueueMode::Shared, |button| {
                                button.primary()
                            })
                            .when(self.queue_mode != ReviewQueueMode::Shared, |button| {
                                button.ghost()
                            })
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.queue_mode = ReviewQueueMode::Shared;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("finance-mode-all")
                            .xsmall()
                            .label("All")
                            .when(self.queue_mode == ReviewQueueMode::All, |button| {
                                button.primary()
                            })
                            .when(self.queue_mode != ReviewQueueMode::All, |button| {
                                button.ghost()
                            })
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.queue_mode = ReviewQueueMode::All;
                                cx.notify();
                            })),
                    ),
            )
            .child(expense_rows);
        let plans = self.trip_plans.value.as_deref().unwrap_or_default();
        let mut trip_focus = div().flex().items_center().gap_1().child(
            Button::new("finance-trip-focus-all")
                .xsmall()
                .label("Any trip")
                .when(self.review_trip_id.is_empty(), |button| button.primary())
                .when(!self.review_trip_id.is_empty(), |button| button.ghost())
                .on_click(cx.listener(|page, _, _, cx| {
                    page.review_trip_id.clear();
                    cx.notify();
                })),
        );
        for plan in plans.iter().filter(|plan| plan.status != "archived") {
            let id = plan.id.clone();
            trip_focus = trip_focus.child(
                Button::new(SharedString::from(format!(
                    "finance-trip-focus-{}",
                    plan.id
                )))
                .xsmall()
                .label(plan.title.clone())
                .when(self.review_trip_id == plan.id, |button| button.primary())
                .when(self.review_trip_id != plan.id, |button| button.ghost())
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.review_trip_id = id.clone();
                    cx.notify();
                })),
            );
        }
        allocation = allocation.child(trip_focus);
        if review_queue.len() > self.expense_limit {
            allocation = allocation.child(
                Button::new("finance-expenses-more")
                    .small()
                    .ghost()
                    .label("Show more expenses")
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.expense_limit += 16;
                        cx.notify();
                    })),
            );
        }
        if self.selected_expense.is_some() {
            let purposes = [
                ("day_to_day", "Day to day"),
                ("trip", "Trip"),
                ("work", "Work"),
                ("housing", "Housing"),
                ("other", "Other"),
            ];
            let mut purpose_buttons = div().flex().items_center().gap_1();
            for (value, label) in purposes {
                let value_for_click = value.to_owned();
                purpose_buttons = purpose_buttons.child(
                    Button::new(SharedString::from(format!("finance-purpose-{value}")))
                        .xsmall()
                        .label(label)
                        .when(self.allocation_purpose == value, |button| button.primary())
                        .when(self.allocation_purpose != value, |button| button.ghost())
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.allocation_purpose = value_for_click.clone();
                            if page.allocation_purpose != "trip" {
                                page.selected_trip_id.clear();
                            }
                            cx.notify();
                        })),
                );
            }
            allocation = allocation
                .child(purpose_buttons)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("Your final cost"),
                        )
                        .child(personal_input),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            Button::new("finance-share-100")
                                .xsmall()
                                .ghost()
                                .label("All mine")
                                .on_click(cx.listener(|page, _, window, cx| {
                                    page.set_personal_share(100, window, cx)
                                })),
                        )
                        .child(
                            Button::new("finance-share-50")
                                .xsmall()
                                .ghost()
                                .label("Half")
                                .on_click(cx.listener(|page, _, window, cx| {
                                    page.set_personal_share(50, window, cx)
                                })),
                        )
                        .child(
                            Button::new("finance-share-25")
                                .xsmall()
                                .ghost()
                                .label("Quarter")
                                .on_click(cx.listener(|page, _, window, cx| {
                                    page.set_personal_share(25, window, cx)
                                })),
                        )
                        .child(
                            Button::new("finance-share-0")
                                .xsmall()
                                .ghost()
                                .label("Fronted all")
                                .on_click(cx.listener(|page, _, window, cx| {
                                    page.set_personal_share(0, window, cx)
                                })),
                        ),
                )
                .child(
                    Button::new("finance-save-allocation")
                        .small()
                        .primary()
                        .label("Save purpose and share")
                        .when(
                            !share_is_valid
                                || (self.allocation_purpose == "trip"
                                    && self.selected_trip_id.is_empty()),
                            |button| button.disabled(true),
                        )
                        .on_click(cx.listener(|page, _, _, cx| page.save_allocation(cx))),
                );
            if self.allocation_purpose == "trip" {
                let plans = self.trip_plans.value.as_deref().unwrap_or_default();
                let mut plan_buttons = div().flex().items_center().gap_1().child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("Trip:"),
                );
                for plan in plans.iter().filter(|plan| plan.status != "archived") {
                    let trip_id = plan.id.clone();
                    plan_buttons = plan_buttons.child(
                        Button::new(SharedString::from(format!("finance-trip-{}", plan.id)))
                            .xsmall()
                            .label(plan.title.clone())
                            .when(self.selected_trip_id == plan.id, |button| button.primary())
                            .when(self.selected_trip_id != plan.id, |button| button.ghost())
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.selected_trip_id = trip_id.clone();
                                cx.notify();
                            })),
                    );
                }
                allocation = allocation.child(plan_buttons);
            }
        }

        let open_receivables: Vec<_> = self
            .dashboard
            .value
            .as_ref()
            .map(|dashboard| {
                dashboard
                    .shared_expenses
                    .iter()
                    .filter(|expense| expense.outstanding_cents > 0)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let inflows: Vec<_> = candidates
            .iter()
            .filter(|candidate| {
                candidate.state == "confirmed"
                    && candidate.amount_cents > 0
                    && candidate.proposed_account != "assets:receivable:shared"
            })
            .cloned()
            .collect();
        let mut reimbursement = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(7.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child("Link a repayment"),
            );
        for inflow in inflows.iter().take(self.inflow_limit) {
            let id = inflow.id.clone();
            reimbursement = reimbursement.child(
                Button::new(SharedString::from(format!("finance-inflow-{}", inflow.id)))
                    .xsmall()
                    .label(format!(
                        "Inflow · {} · {}",
                        inflow.description,
                        format_money(inflow.amount_cents, &inflow.currency)
                    ))
                    .when(
                        self.selected_inflow.as_deref() == Some(&inflow.id),
                        |button| button.primary(),
                    )
                    .when(
                        self.selected_inflow.as_deref() != Some(&inflow.id),
                        |button| button.ghost(),
                    )
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.selected_inflow = Some(id.clone());
                        cx.notify();
                    })),
            );
        }
        if inflows.len() > self.inflow_limit {
            reimbursement = reimbursement.child(
                Button::new("finance-inflows-more")
                    .small()
                    .ghost()
                    .label("Show more inflows")
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.inflow_limit += 8;
                        cx.notify();
                    })),
            );
        }
        for expense in open_receivables.iter().take(self.receivable_limit) {
            let id = expense.candidate_id.clone();
            reimbursement = reimbursement.child(
                Button::new(SharedString::from(format!(
                    "finance-receivable-{}",
                    expense.candidate_id
                )))
                .xsmall()
                .label(format!(
                    "Expense · {} · {} open",
                    expense.description,
                    format_money(expense.outstanding_cents, &expense.currency)
                ))
                .when(
                    self.reimbursement_target.as_deref() == Some(&expense.candidate_id),
                    |button| button.primary(),
                )
                .when(
                    self.reimbursement_target.as_deref() != Some(&expense.candidate_id),
                    |button| button.ghost(),
                )
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.reimbursement_target = Some(id.clone());
                    cx.notify();
                })),
            );
        }
        if open_receivables.len() > self.receivable_limit {
            reimbursement = reimbursement.child(
                Button::new("finance-receivables-more")
                    .small()
                    .ghost()
                    .label("Show more receivables")
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.receivable_limit += 8;
                        cx.notify();
                    })),
            );
        }
        reimbursement = reimbursement.child(
            Button::new("finance-link-reimbursement")
                .small()
                .primary()
                .label("Link selected inflow")
                .when(
                    self.selected_inflow.is_none() || self.reimbursement_target.is_none(),
                    |button| button.disabled(true),
                )
                .on_click(cx.listener(|page, _, _, cx| page.link_reimbursement(cx))),
        );
        let profiles = self
            .csv_mappings
            .value
            .as_deref()
            .unwrap_or_default()
            .to_vec();
        let manual_selected = self.selected_mapping >= profiles.len();
        if manual_selected {
            self.ensure_csv_mapping_fields(window, cx);
        }
        let profile_name = if manual_selected {
            "Manual entry".to_owned()
        } else {
            profiles[self.selected_mapping].label.clone()
        };
        let mapping = self.active_csv_mapping();
        let mapping_detail = format!(
            "Date: {} · Amount: {} · Description: {} · Source: {}",
            mapping["date_column"].as_str().unwrap_or("—"),
            mapping["amount_column"].as_str().unwrap_or("—"),
            mapping["description_column"].as_str().unwrap_or("—"),
            mapping["source_account"].as_str().unwrap_or("—"),
        );
        let mut import_card = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(7.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child("CSV import"),
            )
            .child(
                div().text_xs().text_color(rgb(palette.muted)).child(
                    self.csv_file
                        .as_ref()
                        .map(|(name, _)| name.clone())
                        .unwrap_or_else(|| {
                            "Choose a CSV file. It is read locally and sent for preview.".into()
                        }),
                ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(mapping_detail),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("finance-choose-csv")
                            .small()
                            .ghost()
                            .label("Choose CSV")
                            .on_click(cx.listener(Self::choose_csv)),
                    )
                    .child(
                        Button::new("finance-mapping-profile")
                            .small()
                            .ghost()
                            .label(format!("Mapping: {profile_name}"))
                            .on_click(cx.listener(|page, _, _, cx| {
                                let count =
                                    page.csv_mappings.value.as_ref().map_or(0, Vec::len) + 1;
                                page.selected_mapping = (page.selected_mapping + 1) % count;
                                page.csv_preview = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("finance-preview-csv")
                            .small()
                            .primary()
                            .label("Preview")
                            .when(self.csv_file.is_none(), |button| button.disabled(true))
                            .on_click(cx.listener(|page, _, _, cx| page.preview_csv(cx))),
                    ),
            );
        if manual_selected {
            let mut fields = div().flex().flex_col().gap_2();
            if let Some(mapping_fields) = &self.csv_mapping_fields {
                for field in mapping_fields {
                    fields = fields.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(150.))
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(field.label),
                            )
                            .child(div().w(px(260.)).child(Input::new(&field.input).small())),
                    );
                }
            }
            let date_format = self.manual_csv_mapping["date_formats"][0]
                .as_str()
                .unwrap_or("day_month_year_dots");
            let amount_sign = self.manual_csv_mapping["amount_sign"]
                .as_str()
                .unwrap_or("as_provided");
            let row_policy = self.manual_csv_mapping["row_policy"]
                .as_str()
                .unwrap_or("strict");
            fields = fields
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(150.))
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("Date format"),
                        )
                        .child(
                            Button::new("finance-manual-date-format")
                                .xsmall()
                                .ghost()
                                .label(date_format)
                                .on_click(cx.listener(|page, _, _, cx| {
                                    let next = match page.manual_csv_mapping["date_formats"][0]
                                        .as_str()
                                        .unwrap_or("day_month_year_dots")
                                    {
                                        "day_month_year_dots" => "day_month_year_slashes",
                                        "day_month_year_slashes" => "iso_year_month_day",
                                        _ => "day_month_year_dots",
                                    };
                                    page.manual_csv_mapping["date_formats"] =
                                        serde_json::json!([next]);
                                    page.csv_preview = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(150.))
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("Amount sign"),
                        )
                        .child(
                            Button::new("finance-manual-amount-sign")
                                .xsmall()
                                .ghost()
                                .label(amount_sign)
                                .on_click(cx.listener(|page, _, _, cx| {
                                    page.manual_csv_mapping["amount_sign"] = serde_json::json!(
                                        if page.manual_csv_mapping["amount_sign"] == "invert" {
                                            "as_provided"
                                        } else {
                                            "invert"
                                        }
                                    );
                                    page.csv_preview = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(150.))
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("Row policy"),
                        )
                        .child(
                            Button::new("finance-manual-row-policy")
                                .xsmall()
                                .ghost()
                                .label(row_policy)
                                .on_click(cx.listener(|page, _, _, cx| {
                                    page.manual_csv_mapping["row_policy"] =
                                        serde_json::json!(if page.manual_csv_mapping["row_policy"]
                                            == "strict"
                                        {
                                            "required_fields"
                                        } else {
                                            "strict"
                                        });
                                    page.csv_preview = None;
                                    cx.notify();
                                })),
                        ),
                );
            import_card = import_card.child(fields);
        }
        if let Some(preview) = &self.csv_preview {
            import_card = import_card.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                            "{} candidates · {} outflows · {} inflows · {} duplicates · {} ignored",
                            preview.candidate_count,
                            preview.outflow_count,
                            preview.inflow_count,
                            preview.duplicate_rows,
                            preview.ignored_non_transaction_rows
                        )),
                    )
                    .child(
                        Button::new("finance-stage-csv")
                            .small()
                            .primary()
                            .label("Stage preview")
                            .on_click(cx.listener(|page, _, _, cx| page.stage_csv(cx))),
                    ),
            );
        }
        let mut view = div()
            .id("finance-import-review-scroll")
            .flex()
            .flex_col()
            .gap_3()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child("Import and review"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} pending candidates",
                                candidates
                                    .iter()
                                    .filter(|candidate| candidate.state == "pending")
                                    .count()
                            )),
                    ),
            )
            .child(import_card)
            .when_some(self.notice.clone(), |view, notice| {
                view.child(div().text_sm().text_color(rgb(palette.red)).child(notice))
            })
            .child(queue)
            .child(categorize_panel)
            .child(allocation)
            .child(reimbursement);
        if self.csv_mappings.loading {
            view = view.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("Loading mapping profiles…"),
            );
        }
        view.into_any_element()
    }

    fn render_overview(
        &mut self,
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

        let balance_panel =
            if let Some(snapshot) = projection.and_then(|value| value.balance_snapshot.as_ref()) {
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
                    .child(
                        if let Some(net_worth) =
                            projection.and_then(|value| value.tracked_net_worth.as_ref())
                        {
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
                        },
                    )
                    .child(
                        Button::new("finance-edit-balances")
                            .small()
                            .ghost()
                            .label("Edit balances")
                            .on_click(cx.listener(|page, _, window, cx| {
                                page.open_balance_editor(window, cx)
                            })),
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
                            .on_click(cx.listener(|page, _, window, cx| {
                                page.open_balance_editor(window, cx)
                            })),
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
            .flex()
            .flex_col()
            .gap_4()
            .overflow_y_scroll()
            .child(stat_cards)
            .child(balance_panel)
            .child(balance_editor)
            .child(recent_preview)
            .into_any_element()
    }

    fn render_balance_editor(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
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

fn default_csv_mapping() -> serde_json::Value {
    serde_json::json!({
        "delimiter": ";",
        "decimal_separator": ",",
        "date_column": "Buchung",
        "amount_column": "Betrag",
        "description_column": "Verwendungszweck",
        "categorization_columns": [],
        "reference_column": null,
        "currency_column": "Währung",
        "default_currency": "EUR",
        "source_account": "assets:bank:checking",
        "default_outflow_account": "expenses:uncategorized",
        "default_inflow_account": "income:uncategorized",
        "categorization_rules": [],
        "row_filter": null,
        "amount_sign": "as_provided",
        "amount_rounding": "reject",
        "date_formats": ["day_month_year_dots"],
        "row_policy": "strict",
        "location_columns": null,
    })
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
