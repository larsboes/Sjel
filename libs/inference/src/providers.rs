//! Reviewed cloud providers gating outbound model calls (PRD §6, ISC-24).
//!
//! A cloud model call is refused unless its provider is listed in `providers.toml`,
//! its review date is within the 12-month expiry window, and the payload does not
//! exceed `highest_data_class`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A reviewed provider entry from `providers.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewedProvider {
    pub highest_data_class: String,
    pub reviewed_at: String,
    #[serde(default)]
    pub why: Option<String>,
}

/// The collection of reviewed providers gating cloud calls.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewedProvidersList {
    pub providers: BTreeMap<String, ReviewedProvider>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderAdmissionError {
    #[error("provider {0:?} is not named in reviewed providers list (providers.toml)")]
    UnreviewedProvider(String),
    #[error("provider {provider:?} review expired on {expires_on} (reviewed {reviewed_at}, 12-month limit)")]
    ReviewExpired {
        provider: String,
        reviewed_at: String,
        expires_on: String,
    },
    #[error("provider {provider:?} allows up to {highest_allowed}, but request is {requested}")]
    DataClassExceeded {
        provider: String,
        highest_allowed: String,
        requested: String,
    },
    #[error("invalid date: {0}")]
    InvalidDate(String),
}

/// Normalizes provider name for robust matching (lowercase, trimmed, space to dash).
pub fn normalize_provider_name(name: &str) -> String {
    name.trim().to_lowercase().replace(' ', "-")
}

/// Computes the exact 12-month expiry date for an ISO 8601 YYYY-MM-DD date.
pub fn compute_expiry_date(reviewed_at: &str) -> Option<String> {
    if !crate::valid_iso_date(reviewed_at) {
        return None;
    }
    let year: u32 = reviewed_at[0..4].parse().ok()?;
    let month: u32 = reviewed_at[5..7].parse().ok()?;
    let day: u32 = reviewed_at[8..10].parse().ok()?;

    let next_year = year + 1;
    let leap = next_year % 4 == 0 && (next_year % 100 != 0 || next_year % 400 == 0);
    let max_days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    let next_day = day.min(max_days);
    Some(format!("{next_year:04}-{month:02}-{next_day:02}"))
}

/// Checks whether a given data class is permitted under the provider's ceiling.
pub fn class_admits(highest_allowed: &str, requested: &str) -> bool {
    match requested {
        "c0" => matches!(highest_allowed, "c0" | "c1"),
        "c1" => highest_allowed == "c1",
        _ => false,
    }
}

impl ReviewedProvidersList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, provider: ReviewedProvider) {
        self.providers
            .insert(normalize_provider_name(&name.into()), provider);
    }

    pub fn get(&self, name: &str) -> Option<&ReviewedProvider> {
        let normalized = normalize_provider_name(name);
        self.providers.get(&normalized)
    }

    pub fn parse_toml(toml_str: &str) -> Result<Self, String> {
        let parsed: BTreeMap<String, ReviewedProvider> =
            toml::from_str(toml_str).map_err(|e| format!("failed to parse providers.toml: {e}"))?;
        let mut list = Self::new();
        for (name, provider) in parsed {
            list.insert(name, provider);
        }
        Ok(list)
    }

    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let content = std::fs::read_to_string(path.as_ref()).map_err(|e| {
            format!(
                "could not read providers file {}: {e}",
                path.as_ref().display()
            )
        })?;
        Self::parse_toml(&content)
    }

    /// Load reviewed providers list from environment, overlay, or repository root.
    pub fn load() -> Self {
        if let Ok(env_path) = std::env::var("SJEL_PROVIDERS_PATH") {
            if let Ok(list) = Self::load_from_file(&env_path) {
                return list;
            }
        }
        if let Ok(repo_root) = std::env::var("SJEL_ROOT") {
            let p = PathBuf::from(repo_root).join("providers.toml");
            if let Ok(list) = Self::load_from_file(&p) {
                return list;
            }
        }
        // Check current directory and walk up
        let mut cur = std::env::current_dir().ok();
        while let Some(dir) = cur {
            let p = dir.join("providers.toml");
            if p.is_file() {
                if let Ok(list) = Self::load_from_file(&p) {
                    return list;
                }
            }
            cur = dir.parent().map(|p| p.to_path_buf());
        }
        Self::default()
    }

    /// Gate a cloud call: checks that provider exists, review has not expired, and data class is admitted.
    pub fn check_admission(
        &self,
        provider: &str,
        data_class: &str,
        current_utc_date: &str,
    ) -> Result<&ReviewedProvider, ProviderAdmissionError> {
        let Some(reviewed) = self.get(provider) else {
            return Err(ProviderAdmissionError::UnreviewedProvider(
                provider.to_string(),
            ));
        };

        if !crate::valid_iso_date(current_utc_date) {
            return Err(ProviderAdmissionError::InvalidDate(
                current_utc_date.to_string(),
            ));
        }

        let Some(expires_on) = compute_expiry_date(&reviewed.reviewed_at) else {
            return Err(ProviderAdmissionError::InvalidDate(
                reviewed.reviewed_at.clone(),
            ));
        };

        if current_utc_date > expires_on.as_str() {
            return Err(ProviderAdmissionError::ReviewExpired {
                provider: provider.to_string(),
                reviewed_at: reviewed.reviewed_at.clone(),
                expires_on,
            });
        }

        if !class_admits(&reviewed.highest_data_class, data_class) {
            return Err(ProviderAdmissionError::DataClassExceeded {
                provider: provider.to_string(),
                highest_allowed: reviewed.highest_data_class.clone(),
                requested: data_class.to_string(),
            });
        }

        Ok(reviewed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_validate_providers() {
        let sample = r#"
[nvidia-nim]
highest_data_class = "c0"
reviewed_at = "2026-08-30"
why = "NVIDIA NIM free-tier hosted inference for public content"

[openai]
highest_data_class = "c1"
reviewed_at = "2026-08-25"
why = "Zero-retention agreement"
"#;
        let list = ReviewedProvidersList::parse_toml(sample).expect("parsed");
        assert!(list.get("nvidia-nim").is_some());
        assert!(list.get("NVIDIA NIM").is_some());
        assert!(list.get("OpenAI").is_some());
        assert!(list.get("unreviewed-unknown").is_none());

        // c0 admitted for nvidia-nim within 12 months
        assert!(list
            .check_admission("nvidia-nim", "c0", "2026-09-29")
            .is_ok());

        // c1 refused for nvidia-nim (highest is c0)
        let err = list
            .check_admission("nvidia-nim", "c1", "2026-09-29")
            .unwrap_err();
        assert!(matches!(
            err,
            ProviderAdmissionError::DataClassExceeded { .. }
        ));

        // c1 admitted for openai
        assert!(list.check_admission("openai", "c1", "2026-09-29").is_ok());

        // c2 never admitted
        assert!(list.check_admission("openai", "c2", "2026-09-29").is_err());

        // Unreviewed provider refused (ISC-24 falsifier)
        let err = list
            .check_admission("unreviewed-provider", "c0", "2026-09-29")
            .unwrap_err();
        assert!(matches!(err, ProviderAdmissionError::UnreviewedProvider(_)));

        // Expiry after 12 months
        let expired_err = list
            .check_admission("nvidia-nim", "c0", "2027-09-01")
            .unwrap_err();
        assert!(matches!(
            expired_err,
            ProviderAdmissionError::ReviewExpired { .. }
        ));
    }

    #[test]
    fn expiry_date_computation_handles_leap_year() {
        assert_eq!(compute_expiry_date("2024-02-29"), Some("2025-02-28".into()));
        assert_eq!(compute_expiry_date("2026-08-30"), Some("2027-08-30".into()));
        assert_eq!(compute_expiry_date("invalid-date"), None);
    }
}
