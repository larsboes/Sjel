use super::*;

/// `GET /api/sjel-status/credentials` — every `[[credentials]]` a capability manifest declares
/// (schemas/service.toml.example, "Credentials"), with where this machine holds it.
///
/// Presence only: whether the keychain holds the grant and whether the old overlay env file
/// still exists. No value is read. The desktop app connects a grant in-process
/// (`libs/sjel-credentials/src/google.rs`), so nothing secret ever crosses this route.
pub(crate) async fn credentials_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let root = axon_root().map_err(bad_gateway)?;
    let rows = tokio::task::spawn_blocking(move || declared(&root))
        .await
        .map_err(|_| bad_gateway("the credentials scan stopped"))?;
    Ok(Json(json!(rows)))
}

fn declared(root: &std::path::Path) -> Vec<Value> {
    use sjel_credentials::{CredentialId, CredentialManager, KeychainProvider, Presence};
    use std::sync::Arc;

    let catalog = sjel_config::overlay_config("credentials.toml");
    let manager = catalog.as_ref().and_then(|path| {
        let providers: Vec<Arc<dyn sjel_credentials::CredentialProvider>> =
            vec![Arc::new(KeychainProvider::system())];
        CredentialManager::new(path, providers).ok()
    });
    let mut rows = Vec::new();
    for (capability, declarations) in sjel_credentials::declaration::declared_in(root) {
        let declarations = match declarations {
            Ok(d) => d,
            Err(error) => {
                rows.push(json!({ "capability": capability, "error": error }));
                continue;
            }
        };
        for d in declarations {
            let in_keychain = manager.as_ref().is_some_and(|m| {
                CredentialId::parse(d.id.as_str())
                    .ok()
                    .and_then(|id| m.status(&id).ok())
                    == Some(Presence::Present)
            });
            let env_file = d
                .env_fallback
                .as_deref()
                .and_then(sjel_config::overlay_config)
                .is_some_and(|p| p.is_file());
            rows.push(json!({
                "capability": capability,
                "id": d.id,
                "type": d.kind,
                "label": d.label,
                "scopes": d.scopes,
                "steps": d.steps,
                "env_fallback": d.env_fallback,
                "in_keychain": in_keychain,
                "env_file": env_file,
            }));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_checkout_declares_both_google_grants_without_a_parse_error() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let rows = super::declared(&root);
        assert!(rows.iter().all(|r| r.get("error").is_none()), "{rows:?}");
        let ids: Vec<&str> = rows.iter().filter_map(|r| r["id"].as_str()).collect();
        assert!(ids.contains(&"google-comms") && ids.contains(&"google-entities"), "{ids:?}");
        assert!(rows.iter().all(|r| r.get("value").is_none() && r.get("refresh_token").is_none()));
    }
}
