//! Private manual balance snapshots and tracked net-worth composition.
//!
//! Transaction imports describe movement, not the current value of accounts. A
//! reviewed snapshot supplies that missing point-in-time state without turning
//! inferred transaction totals into an account balance.

use crate::investment::{DecimalValue, PortfolioValuation};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BalanceCoverage {
    Complete,
    #[default]
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BalanceKind {
    Asset,
    Liability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualBalance {
    pub id: String,
    pub label: String,
    pub kind: BalanceKind,
    pub amount_cents: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualBalanceSnapshot {
    pub schema_version: u32,
    pub as_of: String,
    pub updated_at: String,
    pub currency: String,
    #[serde(default)]
    pub coverage: BalanceCoverage,
    pub balances: Vec<ManualBalance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ManualBalanceUpdate {
    pub as_of: String,
    pub currency: String,
    #[serde(default)]
    pub coverage: BalanceCoverage,
    pub balances: Vec<ManualBalance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrackedNetWorth {
    pub currency: String,
    pub value: DecimalValue,
    pub manual_balance_cents: i64,
    pub portfolio_included: bool,
    pub complete: bool,
}

pub fn snapshot_from_update(
    update: ManualBalanceUpdate,
    updated_at: String,
) -> Result<ManualBalanceSnapshot, String> {
    let snapshot = ManualBalanceSnapshot {
        schema_version: 1,
        as_of: update.as_of,
        updated_at,
        currency: update.currency,
        coverage: update.coverage,
        balances: update.balances,
    };
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

pub fn read_snapshot(path: &Path) -> Result<Option<ManualBalanceSnapshot>, String> {
    let body = match std::fs::read_to_string(path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("balance snapshot could not be read: {error}")),
    };
    let snapshot: ManualBalanceSnapshot = serde_json::from_str(&body)
        .map_err(|_| "balance snapshot has an unsupported shape".to_string())?;
    validate_snapshot(&snapshot)?;
    Ok(Some(snapshot))
}

pub fn write_snapshot(path: &Path, snapshot: &ManualBalanceSnapshot) -> Result<(), String> {
    validate_snapshot(snapshot)?;
    let parent = path
        .parent()
        .ok_or_else(|| "balance snapshot has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("balance snapshot directory could not be created: {error}"))?;
    let body = serde_json::to_vec_pretty(snapshot)
        .map_err(|_| "balance snapshot could not be serialized".to_string())?;
    let temporary = parent.join(format!(
        ".balance-{}-{}.tmp",
        std::process::id(),
        snapshot.updated_at.replace([':', '.'], "-")
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| {
                format!("balance snapshot temporary file could not be created: {error}")
            })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|error| {
                    format!("balance snapshot permissions could not be set: {error}")
                })?;
        }
        file.write_all(&body)
            .and_then(|_| file.write_all(b"\n"))
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("balance snapshot could not be written: {error}"))?;
        std::fs::rename(&temporary, path)
            .map_err(|error| format!("balance snapshot could not be replaced: {error}"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

pub fn tracked_net_worth(
    snapshot: &ManualBalanceSnapshot,
    portfolio: Option<&PortfolioValuation>,
    portfolio_complete: bool,
) -> Result<TrackedNetWorth, String> {
    validate_snapshot(snapshot)?;
    let manual_balance_cents = snapshot.balances.iter().try_fold(0_i64, |total, balance| {
        let signed = match balance.kind {
            BalanceKind::Asset => Some(balance.amount_cents),
            BalanceKind::Liability => balance.amount_cents.checked_neg(),
        }
        .ok_or_else(|| "balance total is outside the supported range".to_string())?;
        total
            .checked_add(signed)
            .ok_or_else(|| "balance total is outside the supported range".to_string())
    })?;
    let mut value = DecimalValue {
        mantissa: i128::from(manual_balance_cents),
        scale: 2,
    };
    let portfolio_included = portfolio.is_some_and(|value| value.currency == snapshot.currency);
    if let Some(portfolio) = portfolio.filter(|value| value.currency == snapshot.currency) {
        value = add_decimal(&value, &portfolio.value)?;
    }
    Ok(TrackedNetWorth {
        currency: snapshot.currency.clone(),
        value,
        manual_balance_cents,
        portfolio_included,
        complete: snapshot.coverage == BalanceCoverage::Complete
            && portfolio_included
            && portfolio_complete,
    })
}

fn validate_snapshot(snapshot: &ManualBalanceSnapshot) -> Result<(), String> {
    if snapshot.schema_version != 1 {
        return Err("balance snapshot schema version is unsupported".into());
    }
    if !valid_iso_date(&snapshot.as_of) {
        return Err("balance snapshot as_of must be a real date in YYYY-MM-DD form".into());
    }
    if snapshot.updated_at.trim().is_empty() {
        return Err("balance snapshot updated_at is required".into());
    }
    if snapshot.currency.len() != 3
        || !snapshot
            .currency
            .bytes()
            .all(|byte| byte.is_ascii_uppercase())
    {
        return Err("balance snapshot currency must be a three-letter uppercase code".into());
    }
    if snapshot.balances.is_empty() {
        return Err("balance snapshot requires at least one balance".into());
    }
    let mut ids = HashSet::new();
    for balance in &snapshot.balances {
        if balance.id.is_empty()
            || !balance
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err("balance ids must be symbolic names".into());
        }
        if !ids.insert(balance.id.as_str()) {
            return Err("balance ids must be unique".into());
        }
        let label = balance.label.trim();
        if label.is_empty() || label.len() > 100 || label.chars().any(char::is_control) {
            return Err("balance labels must be short, non-blank text".into());
        }
        if balance.amount_cents < 0 {
            return Err("balance amounts must not be negative".into());
        }
    }
    Ok(())
}

fn add_decimal(left: &DecimalValue, right: &DecimalValue) -> Result<DecimalValue, String> {
    let scale = left.scale.max(right.scale);
    let align = |value: &DecimalValue| {
        value
            .mantissa
            .checked_mul(10_i128.checked_pow(scale - value.scale)?)
    };
    let left = align(left)
        .ok_or_else(|| "tracked net worth is outside the supported range".to_string())?;
    let right = align(right)
        .ok_or_else(|| "tracked net worth is outside the supported range".to_string())?;
    Ok(DecimalValue {
        mantissa: left
            .checked_add(right)
            .ok_or_else(|| "tracked net worth is outside the supported range".to_string())?,
        scale,
    })
}

fn valid_iso_date(value: &str) -> bool {
    let Some((year, rest)) = value.split_once('-') else {
        return false;
    };
    let Some((month, day)) = rest.split_once('-') else {
        return false;
    };
    let (Ok(year), Ok(month), Ok(day)) = (
        year.parse::<u32>(),
        month.parse::<u32>(),
        day.parse::<u32>(),
    ) else {
        return false;
    };
    if value.len() != 10 {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let last_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=last_day).contains(&day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> ManualBalanceSnapshot {
        ManualBalanceSnapshot {
            schema_version: 1,
            as_of: "2026-08-09".into(),
            updated_at: "2026-08-09T12:00:00Z".into(),
            currency: "EUR".into(),
            coverage: BalanceCoverage::Partial,
            balances: vec![
                ManualBalance {
                    id: "cash-one".into(),
                    label: "Synthetic cash".into(),
                    kind: BalanceKind::Asset,
                    amount_cents: 10_000,
                },
                ManualBalance {
                    id: "card-one".into(),
                    label: "Synthetic card".into(),
                    kind: BalanceKind::Liability,
                    amount_cents: 2_500,
                },
            ],
        }
    }

    #[test]
    fn assets_and_liabilities_contribute_with_opposite_signs() {
        let tracked = tracked_net_worth(&snapshot(), None, false).unwrap();
        assert_eq!(tracked.manual_balance_cents, 7_500);
        assert_eq!(tracked.value.mantissa, 7_500);
        assert_eq!(tracked.value.scale, 2);
        assert!(!tracked.complete);
    }

    #[test]
    fn portfolio_precision_is_preserved_when_values_are_combined() {
        let portfolio = PortfolioValuation {
            currency: "EUR".into(),
            value: DecimalValue {
                mantissa: 1_234_567,
                scale: 3,
            },
            priced_holdings: 1,
            unpriced_holdings: 0,
        };
        let tracked = tracked_net_worth(&snapshot(), Some(&portfolio), true).unwrap();
        assert_eq!(tracked.value.mantissa, 1_309_567);
        assert_eq!(tracked.value.scale, 3);
        assert!(tracked.portfolio_included);
        assert!(!tracked.complete);
    }

    #[test]
    fn snapshot_round_trip_keeps_private_state_out_of_the_database() {
        let path = std::env::temp_dir().join(format!(
            "axon-balance-{}-{}.json",
            std::process::id(),
            snapshot().updated_at.replace(':', "-")
        ));
        write_snapshot(&path, &snapshot()).unwrap();
        assert_eq!(read_snapshot(&path).unwrap(), Some(snapshot()));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn invalid_or_ambiguous_manual_values_fail_closed() {
        let mut invalid = snapshot();
        invalid.balances[1].id = invalid.balances[0].id.clone();
        invalid.balances[1].amount_cents = -1;
        assert!(validate_snapshot(&invalid).is_err());
    }
}
