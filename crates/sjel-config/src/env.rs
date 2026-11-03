//! Environment and `deployment.env` settings under the Sjel name.
//!
//! The product was renamed from Axon to Sjel on 2026-09-26. Settings are read under their
//! `SJEL_` name only. The `AXON_` names that existed before the rename were read as a fallback
//! until that compatibility layer was retired on 2026-09-30: a setting that is only present
//! under the old name is now absent, so a half-migrated overlay fails loudly instead of
//! silently answering from a name nothing sets any more.

use std::ffi::OsString;

/// [`std::env::var`] for a setting in the environment.
pub fn env_var(name: &str) -> Result<String, std::env::VarError> {
    std::env::var(name)
}

/// [`std::env::var_os`] for a setting in the environment.
pub fn env_var_os(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// The value of `name` in a `KEY=value` body such as `deployment.env`, trimmed and non-empty.
pub fn deployment_value(body: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    body.lines().find_map(|line| {
        line.strip_prefix(prefix.as_str())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setting_is_read_under_its_own_name() {
        let body = "SJEL_HOME_TIMEZONE=Europe/Berlin\nSJEL_LAN_PORT=8443\nSJEL_EMPTY=\n";
        assert_eq!(
            deployment_value(body, "SJEL_HOME_TIMEZONE").as_deref(),
            Some("Europe/Berlin")
        );
        assert_eq!(
            deployment_value(body, "SJEL_LAN_PORT").as_deref(),
            Some("8443")
        );
        assert_eq!(deployment_value(body, "SJEL_ABSENT"), None);
        assert_eq!(deployment_value("SJEL_EMPTY=\n", "SJEL_EMPTY"), None);
    }

    #[test]
    fn the_pre_rename_axon_name_is_no_longer_read() {
        // The compatibility layer was retired on 2026-09-30 (see CONTRIBUTING.md). An AXON_
        // name must not answer for a SJEL_ one in either direction: a body that carries only
        // the old name yields nothing, and an empty SJEL_ line is not filled from it either.
        let body = "AXON_LAN_PORT=8443\nAXON_HOME_TIMEZONE=UTC\n";
        assert_eq!(deployment_value(body, "SJEL_LAN_PORT"), None);
        assert_eq!(deployment_value(body, "SJEL_HOME_TIMEZONE"), None);

        let mixed = "AXON_HOME_TIMEZONE=UTC\nSJEL_HOME_TIMEZONE=\n";
        assert_eq!(deployment_value(mixed, "SJEL_HOME_TIMEZONE"), None);

        // The environment behaves the same way, under a name unique to this test so a
        // parallel test cannot race on it.
        std::env::set_var("AXON_ENV_COMPAT_RETIRED", "old");
        assert!(env_var("SJEL_ENV_COMPAT_RETIRED").is_err());
        assert!(env_var_os("SJEL_ENV_COMPAT_RETIRED").is_none());
        std::env::remove_var("AXON_ENV_COMPAT_RETIRED");
    }
}
