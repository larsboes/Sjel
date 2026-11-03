//! Narrow cloud admission for the shared agent loop.
//!
//! An arbitrary agent transcript is not a reviewed cloud derivative. This module admits only
//! explicitly selected c0 conversations, with no extensions and only tools that attest c0
//! results, and reserves every provider request against a private, cross-process daily ledger.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sjel_inference::ResolvedRole;

const LOCK_WAIT: Duration = Duration::from_secs(2);
const LOCK_RETRY: Duration = Duration::from_millis(20);

pub(crate) fn admit_and_reserve(
    role: &ResolvedRole,
    body: &serde_json::Value,
    data_class: &str,
    tools_attest_public: bool,
) -> Result<(), String> {
    if data_class != "c0" {
        return Err("cloud agent mode currently admits public c0 content only".into());
    }
    if body.get("tools").is_some() && !tools_attest_public {
        return Err("cloud agent request contains tool definitions that do not attest c0".into());
    }
    if !role.has_cloud_policy() {
        return Err("cloud role is missing a complete HTTPS provider and billing policy".into());
    }
    let date = utc_date_today()?;
    if !role.billing_active_on(&date) {
        return Err("cloud role billing authorization is inactive for the current UTC date".into());
    }
    let provider = role
        .provider_name
        .as_deref()
        .ok_or_else(|| "cloud role has no provider name".to_owned())?;
    let reviewed = sjel_inference::ReviewedProvidersList::load();
    reviewed
        .check_admission(provider, "c0", &date)
        .map_err(|error| error.to_string())?;
    if !role.dispatch_ready() {
        return Err("cloud role credentials are not configured".into());
    }
    let max_bytes = role
        .max_input_tokens
        .ok_or_else(|| "cloud role has no input limit".to_owned())? as usize;
    let input_bytes = body
        .get(if role.uses_responses_api() {
            "input"
        } else {
            "messages"
        })
        .map_or(0, |messages| messages.to_string().len())
        .saturating_add(256);
    if input_bytes > max_bytes {
        return Err(format!(
            "cloud request has {input_bytes} input bytes; role limit is {max_bytes}"
        ));
    }
    let limit = role
        .max_requests_per_day
        .ok_or_else(|| "cloud role has no daily request ceiling".to_owned())?;
    let ledger_path = sjel_config::overlay_data_dir("agent")
        .ok_or_else(|| {
            "cloud requests require the private Sjel overlay for their usage ledger".to_owned()
        })?
        .join("cloud-usage.json");
    reserve(&ledger_path, &date, &role.cache_key(), limit)
}

fn reserve(path: &Path, date: &str, provider: &str, limit: u32) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "cloud usage ledger has no parent directory".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| format!("create cloud usage directory: {error}"))?;
    let lock_path = path.with_extension("lock");
    let start = SystemTime::now();
    let lock = loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(file) => break (file, LockFile(lock_path.clone())),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if start.elapsed().unwrap_or_default() >= LOCK_WAIT {
                    return Err("cloud usage ledger is locked; refusing provider request".into());
                }
                thread::sleep(LOCK_RETRY);
            }
            Err(error) => return Err(format!("lock cloud usage ledger: {error}")),
        }
    };
    let (_lock_file, _lock_guard) = lock;

    let mut counts = if path.exists() {
        let contents =
            fs::read(path).map_err(|error| format!("read cloud usage ledger: {error}"))?;
        serde_json::from_slice::<BTreeMap<String, u32>>(&contents).map_err(|error| {
            format!("cloud usage ledger is invalid; refusing provider request: {error}")
        })?
    } else {
        BTreeMap::new()
    };
    let key = format!("{date}:{provider}");
    let count = counts.get(&key).copied().unwrap_or_default();
    if count >= limit {
        return Err(format!(
            "cloud provider daily request ceiling reached ({limit})"
        ));
    }
    counts.insert(key, count + 1);
    let encoded = serde_json::to_vec(&counts)
        .map_err(|error| format!("encode cloud usage ledger: {error}"))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = path.with_extension(format!("{}.{}.tmp", std::process::id(), nonce));
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&encoded)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    write_result.map_err(|error| format!("write cloud usage ledger: {error}"))
}

struct LockFile(PathBuf);

impl Drop for LockFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn utc_date_today() -> Result<String, String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))?
        .as_secs();
    let days = (seconds / 86_400) as i64;
    Ok(civil_date(days))
}

fn civil_date(days_since_epoch: i64) -> String {
    let z = days_since_epoch + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_utc_calendar_dates() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(10_957), "2000-01-01");
        assert_eq!(civil_date(20_000), "2024-10-04");
    }

    #[test]
    fn daily_budget_is_durable_and_fails_closed_at_the_limit() {
        let dir = std::env::temp_dir().join(format!(
            "sjel-agent-cloud-budget-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("usage.json");
        reserve(&path, "2026-10-22", "test-provider", 2).unwrap();
        reserve(&path, "2026-10-22", "test-provider", 2).unwrap();
        assert!(reserve(&path, "2026-10-22", "test-provider", 2).is_err());
        assert!(reserve(&path, "2026-10-23", "test-provider", 2).is_ok());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_requests_cannot_overspend_the_daily_ceiling() {
        let dir = std::env::temp_dir().join(format!(
            "sjel-agent-cloud-budget-race-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("usage.json");
        let handles = (0..12)
            .map(|_| {
                let path = path.clone();
                thread::spawn(move || reserve(&path, "2026-10-22", "provider:model", 4).is_ok())
            })
            .collect::<Vec<_>>();
        let admitted = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|admitted| *admitted)
            .count();
        assert_eq!(admitted, 4);
        fs::remove_dir_all(dir).unwrap();
    }
}
