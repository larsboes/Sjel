//! The accounting-engine boundary.
//!
//! Sjel owns these DTOs and reads the journal itself. The boundary stayed after
//! the engine changed: callers still ask for `check` and `transactions` and
//! still receive Sjel's own types, so replacing the engine under them changed
//! no importer, no analytic and no route.
//!
//! Until 2026-08-28 the only implementation shelled out to hledger and parsed
//! its JSON reports. PRD Q50 replaced it with [`crate::journal`], which reads
//! the plaintext file directly. The journal FORMAT is unchanged and stays
//! hledger-compatible by design (Principle 8) -- it is the runtime dependency
//! that is gone, not the portability of the file.
//!
//! The trait is two methods because two is what production called. The shell-out
//! also carried register, balance, cash-flow, budget and ROI adapters; measured
//! 2026-08-28, no route, analytic or tool invoked any of them, so they retired
//! with the process boundary that was their only reason for existing.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub type AccountingResult<T> = Result<T, AccountingError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountingError {
    pub operation: &'static str,
    pub message: String,
}

impl std::fmt::Display for AccountingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.message)
    }
}

impl std::error::Error for AccountingError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Amount {
    pub commodity: String,
    pub mantissa: i64,
    pub scale: u32,
}

impl Amount {
    pub fn minor_units(&self, scale: u32) -> Option<i64> {
        if self.scale > scale {
            let divisor = 10_i64.checked_pow(self.scale - scale)?;
            (self.mantissa % divisor == 0).then_some(self.mantissa / divisor)
        } else {
            self.mantissa
                .checked_mul(10_i64.checked_pow(scale - self.scale)?)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Posting {
    pub account: String,
    pub amounts: Vec<Amount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalTransaction {
    pub index: u64,
    pub date: String,
    pub description: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
    pub postings: Vec<Posting>,
}

pub trait AccountingEngine: Send + Sync {
    fn check(&self) -> AccountingResult<()>;
    fn transactions(&self) -> AccountingResult<Vec<JournalTransaction>>;
}

/// Reads a plaintext journal with Sjel's own parser.
#[derive(Debug, Clone)]
pub struct JournalEngine {
    journal: PathBuf,
}

impl JournalEngine {
    pub fn new(journal: impl Into<PathBuf>) -> Self {
        Self {
            journal: journal.into(),
        }
    }

    pub fn journal(&self) -> &Path {
        &self.journal
    }

    fn read(&self, operation: &'static str) -> AccountingResult<String> {
        std::fs::read_to_string(&self.journal).map_err(|error| AccountingError {
            // The path is the operator's own and already configured; the OS
            // error says whether it is missing, unreadable or not UTF-8.
            operation,
            message: format!("journal could not be read: {error}"),
        })
    }
}

impl AccountingEngine for JournalEngine {
    fn check(&self) -> AccountingResult<()> {
        crate::journal::validate(&self.read("check")?).map_err(|error| AccountingError {
            operation: "check",
            // The refusal names its line number and a bounded reason. It never
            // echoes a description, so a diagnostic cannot carry a payee out of
            // the journal the way the previous engine's raw output could.
            message: error.to_string(),
        })
    }

    fn transactions(&self) -> AccountingResult<Vec<JournalTransaction>> {
        crate::journal::parse(&self.read("transactions")?).map_err(|error| AccountingError {
            operation: "transactions",
            message: error.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/finance-journal.example")
    }

    #[test]
    fn decimal_amounts_convert_without_floating_point() {
        assert_eq!(
            Amount {
                commodity: "EUR".into(),
                mantissa: 1234,
                scale: 2
            }
            .minor_units(2),
            Some(1234)
        );
        assert_eq!(
            Amount {
                commodity: "EUR".into(),
                mantissa: 12,
                scale: 0
            }
            .minor_units(2),
            Some(1200)
        );
        assert_eq!(
            Amount {
                commodity: "EUR".into(),
                mantissa: 123,
                scale: 3
            }
            .minor_units(2),
            None
        );
    }

    /// The engine reads the published fixture end to end. This used to be
    /// skipped unless hledger happened to be installed -- which, measured on
    /// this machine, it was not -- so the boundary now has a test that always
    /// runs.
    #[test]
    fn the_engine_checks_and_reads_the_published_fixture() {
        let engine = JournalEngine::new(fixture());
        engine.check().unwrap();
        let transactions = engine.transactions().unwrap();
        assert_eq!(transactions.len(), 7);
        assert!(transactions.iter().any(|transaction| transaction
            .postings
            .iter()
            .any(|posting| posting.account == "income:salary")));
    }

    #[test]
    fn a_missing_journal_is_an_error_naming_the_operation() {
        let engine = JournalEngine::new(fixture().join("absent.journal"));
        let error = engine.check().unwrap_err();
        assert_eq!(error.operation, "check");
        assert!(error.message.contains("could not be read"), "{error}");
    }

    /// A refusal reaches the caller with its line number, which is what makes an
    /// unparseable journal fixable rather than merely rejected.
    #[test]
    fn a_broken_journal_reports_the_line_that_broke_it() {
        let path = std::env::temp_dir().join(format!(
            "axon-finance-accounting-{}-broken.journal",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "decimal-mark .\n\n2026-01-02 * skewed\n    assets:bank:a  10.00 EUR\n    expenses:food  3.00 EUR\n",
        )
        .unwrap();
        let error = JournalEngine::new(&path).check().unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(error.operation, "check");
        assert!(error.message.contains("line 3"), "{error}");
        assert!(error.message.contains("does not balance"), "{error}");
    }
}
