//! `sjel credentials`: the declared credentials and their keychain presence, and the two ways a
//! Google grant gets into the keychain. The same calls the desktop app's Settings › Accounts
//! page makes (`libs/sjel-credentials/src/google.rs`), for a Mac without the app. It replaces
//! `capabilities/comms/auth/get-refresh-token.ts`, which wrote plaintext overlay env files.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use sjel_credentials::declaration::{declared_in, Declaration};
use sjel_credentials::{
    google, CredentialId, CredentialManager, CredentialProvider, KeychainProvider, Presence,
};

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let result = match args.first().map(String::as_str) {
        None | Some("list") => list(root),
        Some("connect") => match (args.get(1), args.get(2)) {
            (Some(id), Some(file)) => connect(root, id, Path::new(file)),
            _ => Err("usage: sjel credentials connect <id> <client.json>".into()),
        },
        Some("prune") => match args.get(1) {
            Some(id) => prune(root, id),
            None => Err("usage: sjel credentials prune <id>".into()),
        },
        Some("move") => match args.get(1) {
            Some(id) => move_grant(root, id),
            None => Err("usage: sjel credentials move <id>".into()),
        },
        Some(other) => Err(format!(
            "unknown command {other}; run `sjel help credentials`"
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sjel credentials: {error}");
            ExitCode::FAILURE
        }
    }
}

fn catalog() -> Result<std::path::PathBuf, String> {
    sjel_config::overlay_config("credentials.toml")
        .ok_or_else(|| "no private overlay is configured".to_owned())
}

/// The declaration for `id`, with the capability that declares it.
fn find(root: &Path, id: &str) -> Result<(String, Declaration), String> {
    declared_in(root)
        .into_iter()
        .filter_map(|(capability, d)| Some((capability, d.ok()?)))
        .flat_map(|(capability, ds)| ds.into_iter().map(move |d| (capability.clone(), d)))
        .find(|(_, d)| d.id == id)
        .ok_or_else(|| format!("no capability declares {id}; run `sjel credentials list`"))
}

fn list(root: &Path) -> Result<(), String> {
    let manager = catalog().ok().and_then(|path| {
        let providers: Vec<Arc<dyn CredentialProvider>> =
            vec![Arc::new(KeychainProvider::system())];
        CredentialManager::new(path, providers).ok()
    });
    for (capability, declared) in declared_in(root) {
        match declared {
            Err(error) => println!("{capability:<12} invalid [[credentials]]: {error}"),
            Ok(ds) => {
                for d in ds {
                    let present = manager.as_ref().is_some_and(|m| {
                        CredentialId::parse(d.id.as_str())
                            .ok()
                            .and_then(|id| m.status(&id).ok())
                            == Some(Presence::Present)
                    });
                    let state = if present { "keychain" } else { "not connected" };
                    println!("{capability:<12} {:<18} {state:<14} {}", d.id, d.label);
                }
            }
        }
    }
    Ok(())
}

fn connect(root: &Path, id: &str, file: &Path) -> Result<(), String> {
    let (capability, d) = find(root, id)?;
    if d.kind != google::KIND {
        return Err(format!("{id} is not a Google grant"));
    }
    let credential = CredentialId::parse(id).map_err(|e| e.to_string())?;
    google::connect_from_file(
        &catalog()?,
        &credential,
        &d.label,
        &capability,
        &d.scopes,
        file,
        |url| {
            eprintln!("Opening Google consent in your browser…");
            if std::process::Command::new("open").arg(url).spawn().is_err() {
                eprintln!("Open this URL in your browser:\n{url}");
            }
        },
    )?;
    println!("{} is connected. The grant is in the OS keychain.", d.label);
    Ok(())
}

fn prune(root: &Path, id: &str) -> Result<(), String> {
    let (_, d) = find(root, id)?;
    let env = d
        .env_fallback
        .as_deref()
        .and_then(sjel_config::overlay_config)
        .filter(|p| p.is_file())
        .ok_or_else(|| format!("{id} has no old env file"))?;
    let credential = CredentialId::parse(id).map_err(|e| e.to_string())?;
    google::prune_env_grant(&credential, &env)?;
    println!("{}: the keychain grant works; its old env-file copy is removed.", d.label);
    Ok(())
}

fn move_grant(root: &Path, id: &str) -> Result<(), String> {
    let (capability, d) = find(root, id)?;
    let env = d
        .env_fallback
        .as_deref()
        .and_then(sjel_config::overlay_config)
        .filter(|p| p.is_file())
        .ok_or_else(|| format!("{id} has no old env file to move"))?;
    let credential = CredentialId::parse(id).map_err(|e| e.to_string())?;
    google::move_env_grant(
        &catalog()?,
        &credential,
        &d.label,
        vec![capability],
        &d.scopes,
        &env,
    )?;
    println!("{} moved to the OS keychain; the env file no longer holds it.", d.label);
    Ok(())
}
