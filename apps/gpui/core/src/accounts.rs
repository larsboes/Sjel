//! Settings › Accounts: the credentials capability manifests declare, and connecting a Google
//! grant on this Mac.
//!
//! The list comes from `GET /api/sjel-status/credentials` (presence only). Connecting runs in
//! this process (`libs/sjel-credentials/src/google.rs`): the client JSON is read from the
//! dropped file into a zeroizing buffer, the browser opens for consent, and the grant goes to
//! the OS keychain. No secret crosses an HTTP route.

use serde::Deserialize;

/// One declared credential, as sjel-status reports it.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct CredentialRow {
    pub capability: String,
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub env_fallback: Option<String>,
    #[serde(default)]
    pub in_keychain: bool,
    #[serde(default)]
    pub env_file: bool,
    /// A manifest whose `[[credentials]]` did not parse.
    #[serde(default)]
    pub error: Option<String>,
}

impl CredentialRow {
    /// Where this machine holds the grant, in the words the page shows.
    pub fn state(&self) -> &'static str {
        match (self.in_keychain, self.env_file) {
            (true, _) => "In the keychain",
            (false, true) => "Old env file only — connect to move it to the keychain",
            (false, false) => "Not connected",
        }
    }
}

/// Connects `row`'s grant from the dropped client JSON file
/// (`sjel_credentials::google::connect_from_file`, the path `sjel credentials connect` takes too).
/// Blocks until the browser returns (up to five minutes); run it off the UI thread.
pub fn connect_google(row: &CredentialRow, client_file: &std::path::Path) -> Result<(), String> {
    use sjel_credentials::{google, CredentialId};

    if row.kind != google::KIND {
        return Err(format!("{} is not a Google grant", row.label));
    }
    let id = CredentialId::parse(row.id.as_str()).map_err(|e| e.to_string())?;
    let catalog = sjel_config::overlay_config("credentials.toml")
        .ok_or("no private overlay is configured on this Mac")?;
    google::connect_from_file(
        &catalog,
        &id,
        &row.label,
        &row.capability,
        &row.scopes,
        client_file,
        |url| {
            let _ = std::process::Command::new("open").arg(url).spawn();
        },
    )
}

/// Moves `row`'s grant from its old overlay env file into the keychain, with no browser
/// (`sjel_credentials::google::move_env_grant`: store, prove, then strip the env file).
pub fn move_to_keychain(row: &CredentialRow) -> Result<(), String> {
    use sjel_credentials::{google, CredentialId};

    if row.kind != google::KIND {
        return Err(format!("{} is not a Google grant", row.label));
    }
    let env = row
        .env_fallback
        .as_deref()
        .and_then(sjel_config::overlay_config)
        .filter(|p| p.is_file())
        .ok_or("there is no old env file to move")?;
    let id = CredentialId::parse(row.id.as_str()).map_err(|e| e.to_string())?;
    let catalog = sjel_config::overlay_config("credentials.toml")
        .ok_or("no private overlay is configured on this Mac")?;
    google::move_env_grant(
        &catalog,
        &id,
        &row.label,
        vec![row.capability.clone()],
        &row.scopes,
        &env,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_parses_from_the_status_answer_and_names_its_state() {
        let row: CredentialRow = serde_json::from_str(
            r#"{"capability":"comms","id":"google-comms","type":"google-oauth","label":"G",
                "scopes":["gmail.modify"],"steps":["a"],"env_fallback":"comms.env",
                "in_keychain":false,"env_file":true}"#,
        )
        .unwrap();
        assert_eq!(row.kind, "google-oauth");
        assert!(row.state().starts_with("Old env file"));
        let broken: CredentialRow =
            serde_json::from_str(r#"{"capability":"x","error":"invalid"}"#).unwrap();
        assert_eq!(broken.error.as_deref(), Some("invalid"));
    }

    #[test]
    fn a_non_google_row_or_a_missing_file_is_refused_before_any_browser_opens() {
        let row = CredentialRow {
            capability: "x".into(),
            id: "x".into(),
            kind: "api-key".into(),
            label: "X".into(),
            scopes: vec![],
            steps: vec![],
            env_fallback: None,
            in_keychain: false,
            env_file: false,
            error: None,
        };
        assert!(connect_google(&row, std::path::Path::new("/nope")).is_err());
        let google = CredentialRow {
            kind: "google-oauth".into(),
            ..row
        };
        assert!(connect_google(&google, std::path::Path::new("/nope/client.json")).is_err());
        assert!(
            move_to_keychain(&google).is_err(),
            "no env_fallback, nothing to move"
        );
    }
}
