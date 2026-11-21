//! Phone finance source and candidate decisions through the paired Mac.

use std::{collections::HashSet, sync::OnceLock};

use sjel_gpui_core::{
    context::format_money,
    device::Source,
    shell::{self, FinanceCandidate, FinanceDashboardProjection, SubscriptionRecord},
};
use tokio::sync::watch;

use crate::phone_feed::{SourceFeed, SourceState};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FinanceTab {
    #[default]
    Overview,
    Transactions,
    Subscriptions,
    Review,
}

impl FinanceTab {
    pub const ALL: [Self; 4] = [
        Self::Overview,
        Self::Transactions,
        Self::Subscriptions,
        Self::Review,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Transactions => "Transactions",
            Self::Subscriptions => "Subscriptions",
            Self::Review => "Review",
        }
    }
}

#[derive(Clone)]
pub struct Snapshot {
    pub dashboard: FinanceDashboardProjection,
    pub subscriptions: Vec<SubscriptionRecord>,
    pub candidates: Vec<FinanceCandidate>,
    pub(super) reviewing: HashSet<String>,
    pub status: String,
    pub loading: bool,
    pub error: Option<String>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            dashboard: FinanceDashboardProjection::default(),
            subscriptions: Vec::new(),
            candidates: Vec::new(),
            reviewing: HashSet::new(),
            status: "Connect to the paired Mac to read finance records.".into(),
            loading: false,
            error: None,
        }
    }
}

fn feed() -> &'static SourceFeed<Snapshot> {
    static FEED: OnceLock<SourceFeed<Snapshot>> = OnceLock::new();
    FEED.get_or_init(SourceFeed::new)
}

fn state() -> std::sync::MutexGuard<'static, SourceState<Snapshot>> {
    feed().state()
}

fn publish(s: &SourceState<Snapshot>) {
    feed().publish(s);
}

pub(super) fn snapshots() -> &'static watch::Sender<Snapshot> {
    feed().snapshots()
}

/// Connects the phone Finance screen to the Mac's shell or paired device client.
pub fn connect(source: Source) {
    {
        let mut s = state();
        s.source = Some(source);
    }
    load();
}

/// Replaces the current finance snapshot directly (used by offline caching and tests).
pub fn set_snapshot(
    dashboard: FinanceDashboardProjection,
    subscriptions: Vec<SubscriptionRecord>,
    candidates: Vec<FinanceCandidate>,
) {
    let mut s = state();
    let tx_count = dashboard.transactions.len();
    let sub_count = subscriptions.len();
    let pending_count = candidates.iter().filter(|c| c.state == "pending").count();
    s.snap.dashboard = dashboard;
    s.snap.subscriptions = subscriptions;
    s.snap.candidates = candidates;
    s.snap.loading = false;
    s.snap.error = None;
    s.snap.status =
        format!("{tx_count} transactions · {sub_count} subscriptions · {pending_count} to review");
    publish(&s);
}

/// Refreshes dashboard, subscriptions, and pending candidates from the connected source.
pub fn load() {
    let source = {
        let mut s = state();
        let Some(source) = s.source.clone() else {
            return;
        };
        s.snap.loading = true;
        s.snap.error = None;
        s.snap.status = "Reading finance records from the Mac…".into();
        publish(&s);
        source
    };
    shell::spawn(async move {
        let (dash_res, subs_res, cand_res) = futures::join!(
            source.finance_dashboard(),
            source.finance_subscriptions(),
            source.finance_candidates(),
        );
        let mut s = state();
        s.snap.loading = false;
        match dash_res {
            Ok(dashboard) => {
                s.snap.dashboard = dashboard;
                if let Ok(subs) = subs_res {
                    s.snap.subscriptions = subs;
                }
                if let Ok(cands) = cand_res {
                    s.snap.candidates = cands;
                }
                let pending = s
                    .snap
                    .candidates
                    .iter()
                    .filter(|c| c.state == "pending")
                    .count();
                s.snap.status = format!(
                    "{} transactions · {} subscriptions · {pending} to review",
                    s.snap.dashboard.transactions.len(),
                    s.snap.subscriptions.len()
                );
            }
            Err(err) => {
                s.snap.error = Some(err);
            }
        }
        publish(&s);
    });
}

pub(super) fn review_candidate(id: String, decision: &'static str) {
    let source = {
        let mut s = state();
        if !s
            .snap
            .candidates
            .iter()
            .any(|row| row.id == id && row.state == "pending")
        {
            return;
        }
        let Some(source) = s.source.clone() else {
            #[cfg(test)]
            record_decision(&mut s.snap, &id, decision);
            #[cfg(not(test))]
            {
                s.snap.error = Some("Connect to the paired Mac to review candidates.".into());
            }
            publish(&s);
            return;
        };
        if !s.snap.reviewing.insert(id.clone()) {
            return;
        }
        s.snap.error = None;
        publish(&s);
        source
    };
    shell::spawn(async move {
        let result = source.finance_review_candidate(&id, decision).await;
        {
            let mut s = state();
            s.snap.reviewing.remove(&id);
            match &result {
                Ok(_) => record_decision(&mut s.snap, &id, decision),
                Err(error) => s.snap.error = Some(error.clone()),
            }
            publish(&s);
        }
        if result.is_ok() {
            load();
        }
    });
}

fn record_decision(snap: &mut Snapshot, id: &str, decision: &str) {
    if let Some(row) = snap.candidates.iter_mut().find(|row| row.id == id) {
        row.state = if decision == "confirm" {
            "confirmed"
        } else {
            "rejected"
        }
        .into();
    }
    let pending = snap
        .candidates
        .iter()
        .filter(|row| row.state == "pending")
        .count();
    snap.status = format!("Recorded {decision} · {pending} candidates left to review");
}

pub(super) fn format_signed_cents(cents: i64, currency: &str) -> String {
    let code = if currency.trim().is_empty() {
        "EUR"
    } else {
        currency
    };
    format_money(cents, code)
}

mod view;
pub use view::PhoneFinance;
