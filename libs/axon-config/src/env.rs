//! Environment and `deployment.env` settings under the Sjel name, with the Axon name as fallback.
//!
//! The product was renamed from Axon to Sjel on 2026-09-26. Every setting is read under its
//! `SJEL_` name first. When that is absent, the `AXON_` name it had before is read, so an overlay,
//! a launchd unit or a shell profile written before the rename keeps working unchanged.

use std::ffi::OsString;

const PREFIX: &str = "SJEL_";
const LEGACY_PREFIX: &str = "AXON_";

/// The pre-rename name of a `SJEL_` setting, or `None` for any other name.
pub fn legacy_name(name: &str) -> Option<String> {
    name.strip_prefix(PREFIX)
        .map(|rest| format!("{LEGACY_PREFIX}{rest}"))
}

/// [`std::env::var`] for a `SJEL_` name, falling back to its `AXON_` name.
///
pub fn env_var(name: &str) -> Result<String, std::env::VarError> {
    match std::env::var(name) {
        Err(std::env::VarError::NotPresent) => match legacy_name(name) {
            Some(legacy) => std::env::var(legacy),
            None => Err(std::env::VarError::NotPresent),
        },
        other => other,
    }
}

/// [`std::env::var_os`] for a `SJEL_` name, falling back to its `AXON_` name.
pub fn env_var_os(name: &str) -> Option<OsString> {
    std::env::var_os(name).or_else(|| legacy_name(name).and_then(std::env::var_os))
}

/// The value of `name` in a `KEY=value` body such as `deployment.env`, trimmed and non-empty.
/// A line with the `SJEL_` name wins over one with the `AXON_` name, wherever each appears.
pub fn deployment_value(body: &str, name: &str) -> Option<String> {
    let find = |key: &str| {
        let prefix = format!("{key}=");
        body.lines().find_map(|line| {
            line.strip_prefix(prefix.as_str())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
    };
    find(name).or_else(|| legacy_name(name).and_then(|legacy| find(&legacy)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sjel_name_wins_and_the_axon_name_is_the_fallback() {
        assert_eq!(legacy_name("SJEL_DB_PATH").as_deref(), Some("AXON_DB_PATH"));
        assert_eq!(legacy_name("HOME"), None);
        let body = "AXON_LAN_PORT=8443\nSJEL_HOME_TIMEZONE=Europe/Berlin\nAXON_HOME_TIMEZONE=UTC\n";
        assert_eq!(
            deployment_value(body, "SJEL_LAN_PORT").as_deref(),
            Some("8443")
        );
        assert_eq!(
            deployment_value(body, "SJEL_HOME_TIMEZONE").as_deref(),
            Some("Europe/Berlin")
        );
        assert_eq!(deployment_value(body, "SJEL_ABSENT"), None);
        assert_eq!(deployment_value("SJEL_EMPTY=\n", "SJEL_EMPTY"), None);
    }

    #[test]
    fn a_variable_set_under_either_name_is_read() {
        // Names unique to this test, so a parallel test cannot race on them.
        std::env::set_var("AXON_ENV_COMPAT_ONLY_OLD", "old");
        assert_eq!(env_var("SJEL_ENV_COMPAT_ONLY_OLD").as_deref(), Ok("old"));
        std::env::set_var("SJEL_ENV_COMPAT_BOTH", "new");
        std::env::set_var("AXON_ENV_COMPAT_BOTH", "old");
        assert_eq!(env_var("SJEL_ENV_COMPAT_BOTH").as_deref(), Ok("new"));
        assert!(env_var("SJEL_ENV_COMPAT_NEITHER").is_err());
        assert_eq!(env_var_os("SJEL_ENV_COMPAT_ONLY_OLD"), Some("old".into()));
    }
}
