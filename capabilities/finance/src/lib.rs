//! Finance: the decision layer above the ledger.
//!
//! A private plaintext journal is canonical and the shared SQLite file is its
//! rebuildable index (PRD Q45). The [`AccountingEngine`](accounting::AccountingEngine)
//! boundary is implemented in-process by [`journal`] since PRD Q50 (2026-08-28); the
//! journal FORMAT stays hledger-compatible, which is what the boundary was really
//! protecting. This capability owns reviewed import, analytics, subscriptions and the
//! local UI contract.
//!
//! Spec, phases and the decision record: the principal's `Projects/Ledger` note.

pub mod accounting;
pub mod allocation;
pub mod analytics;
pub mod balance;
pub mod clock;
pub mod config;
pub mod decision;
pub mod import;
pub mod investment;
pub mod journal;
pub mod links;
pub mod money;
pub mod obsidian;
pub mod planning;
pub mod portfolio;
pub mod price;
pub mod risk;
pub mod store;
pub mod subscription;

pub use accounting::{AccountingEngine, JournalEngine};
pub use config::Config;
pub use money::{burn_in_eur, to_eur, EurAmount, EurBurn, NoRate, DECLARED_CURRENCY};
pub use obsidian::{scan, seed_from_note, ScanError, ScannedNote, WriteBack};
pub use store::FinanceStore;
pub use subscription::{
    burn_at, burn_by_currency, BillingCycle, Burn, BurnByCurrency, CurrencyBurn, PricePoint, State,
    StateChange, Subscription,
};
