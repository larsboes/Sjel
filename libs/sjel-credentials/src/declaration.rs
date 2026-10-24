//! `[[credentials]]` in a capability's `service.toml`: what a capability needs, never a value.
//!
//! The overlay catalog (`credentials.toml`) says what this machine has; a declaration says what
//! a capability asks for, and how a person obtains it. The Settings › Accounts page in the
//! desktop app lists declarations and the catalog side by side
//! (schemas/service.toml.example, "Credentials").

use serde::{Deserialize, Serialize};

use crate::CredentialId;

/// One credential a capability asks for.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    /// The catalog ID the grant is stored under, for example `google-comms`.
    pub id: String,
    /// How the value is obtained. Only [`crate::google::KIND`] exists today. Spelled `type` in
    /// the manifest, because the shell's flat reader (`tools/lib/toml.sh`) would read a `kind`
    /// here as the manifest's own `kind`.
    #[serde(rename = "type")]
    pub kind: String,
    pub label: String,
    /// For `google-oauth`: short scope names (`gmail.modify`) or full scope URLs.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// What a person does to obtain the value, one step per entry, shown in order.
    #[serde(default)]
    pub steps: Vec<String>,
    /// The overlay env file the capability read before this grant moved to the keychain.
    #[serde(default)]
    pub env_fallback: Option<String>,
}

/// The declarations in one `service.toml` body. A manifest without `[[credentials]]` has none.
pub fn parse(manifest: &str) -> Result<Vec<Declaration>, String> {
    #[derive(Deserialize)]
    struct Manifest {
        #[serde(default)]
        credentials: Vec<Declaration>,
    }
    let manifest: Manifest =
        toml::from_str(manifest).map_err(|e| format!("invalid [[credentials]]: {e}"))?;
    for d in &manifest.credentials {
        CredentialId::parse(d.id.as_str()).map_err(|e| format!("credential {}: {e}", d.id))?;
        if d.kind != crate::google::KIND {
            return Err(format!("credential {}: unknown kind {}", d.id, d.kind));
        }
        if d.scopes.is_empty() {
            return Err(format!(
                "credential {}: a google-oauth grant needs scopes",
                d.id
            ));
        }
    }
    Ok(manifest.credentials)
}

/// Every capability under `root/capabilities` with the declarations its `service.toml` makes,
/// in name order. A manifest whose `[[credentials]]` does not parse answers its error. The one
/// scan sjel-status (the Accounts list) and host-watch (the hourly check) both read.
pub fn declared_in(root: &std::path::Path) -> Vec<(String, Result<Vec<Declaration>, String>)> {
    let mut dirs: Vec<_> = std::fs::read_dir(root.join("capabilities"))
        .map(|it| it.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    dirs.sort();
    dirs.into_iter()
        .filter_map(|dir| {
            let body = std::fs::read_to_string(dir.join("service.toml")).ok()?;
            let name = dir.file_name()?.to_string_lossy().into_owned();
            Some((name, parse(&body)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_declares_a_grant_and_other_fields_are_ignored() {
        let found = parse(
            r#"
name = "comms"
port = 8083
[[credentials]]
id = "google-comms"
type = "google-oauth"
label = "Google: Gmail and Calendar"
scopes = ["gmail.modify", "calendar.events"]
steps = ["Download the client JSON."]
env_fallback = "comms.env"
"#,
        )
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].scopes, ["gmail.modify", "calendar.events"]);
        assert!(parse("name = \"x\"").unwrap().is_empty());
    }

    #[test]
    fn a_bad_declaration_is_refused() {
        for bad in [
            "[[credentials]]\nid = \"../x\"\ntype = \"google-oauth\"\nlabel = \"x\"\nscopes = [\"a\"]",
            "[[credentials]]\nid = \"x\"\ntype = \"api-key\"\nlabel = \"x\"\nscopes = [\"a\"]",
            "[[credentials]]\nid = \"x\"\ntype = \"google-oauth\"\nlabel = \"x\"",
            "[[credentials]]\nid = \"x\"\ntype = \"google-oauth\"\nlabel = \"x\"\nscopes = [\"a\"]\nsecret = \"v\"",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
