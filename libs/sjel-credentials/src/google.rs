//! Google OAuth grants: the one sign-in flow and the one token refresh.
//!
//! A grant is stored as one keychain item holding a JSON [`GoogleOAuthCredential`] (client ID,
//! client secret, refresh token, scopes), named in the overlay catalog `credentials.toml` like
//! every other credential. comms, calendar and entities each refreshed tokens from a plaintext
//! overlay env file before 2026-10-23; [`access_token`] reads the keychain first and that env
//! file second, so a grant moves without a cut-over.
//!
//! [`sign_in`] is the loopback flow for a Desktop OAuth client
//! (<https://developers.google.com/identity/protocols/oauth2/native-app>): PKCE, a random
//! `state`, a listener on `127.0.0.1` with a port the OS picks, and `prompt=consent` so Google
//! always returns a refresh token. Nothing here prints a secret.
//!
//! The consent screen must be **In production**. Google expires a Testing app's refresh tokens
//! after 7 days (<https://developers.google.com/identity/protocols/oauth2#expiration>).

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use zeroize::{Zeroize as _, Zeroizing};

use crate::{
    CredentialId, CredentialManager, CredentialProvider, KeychainProvider, ProviderKind,
    SecretValue,
};

/// The `kind` a `[[credentials]]` declaration uses for a grant this module handles.
pub const KIND: &str = "google-oauth";

const AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPE_PREFIX: &str = "https://www.googleapis.com/auth/";
/// How long [`sign_in`] waits for the browser to come back.
const CONSENT_WINDOW: Duration = Duration::from_secs(300);

/// The Desktop OAuth client from the JSON file Google Cloud offers for download.
pub struct GoogleClient {
    pub client_id: String,
    client_secret: Zeroizing<String>,
}

/// Reads a client JSON: the downloaded `{"installed": {…}}`, a `{"web": {…}}`, or the inner
/// object alone. Refuses anything without both an ID and a secret.
pub fn parse_client_json(text: &str) -> Result<GoogleClient, String> {
    let value: serde_json::Value = serde_json::from_str(text.trim())
        .map_err(|_| "this is not a Google OAuth client JSON file".to_owned())?;
    let inner = value
        .get("installed")
        .or_else(|| value.get("web"))
        .unwrap_or(&value);
    let field = |key: &str| {
        inner
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    match (field("client_id"), field("client_secret")) {
        (Some(id), Some(secret)) => Ok(GoogleClient {
            client_id: id.to_owned(),
            client_secret: Zeroizing::new(secret.to_owned()),
        }),
        _ => Err("the client JSON has no client_id and client_secret".into()),
    }
}

/// One grant, as the keychain item holds it.
#[derive(Serialize, Deserialize)]
pub struct GoogleOAuthCredential {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
    pub scopes: Vec<String>,
}

impl Drop for GoogleOAuthCredential {
    fn drop(&mut self) {
        self.client_secret.zeroize();
        self.refresh_token.zeroize();
    }
}

/// `gmail.modify` → `https://www.googleapis.com/auth/gmail.modify`; a full URL stays as it is.
pub fn scope_url(scope: &str) -> String {
    if scope.starts_with("https://") {
        scope.to_owned()
    } else {
        format!("{SCOPE_PREFIX}{scope}")
    }
}

fn random_url_value() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "could not create sign-in state".to_owned())?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Runs the consent flow and returns the grant. `open_url` shows the consent page: the CLI
/// and the desktop app both pass `open`. Blocks until the browser returns or
/// [`CONSENT_WINDOW`] passes; call it off the UI thread.
pub fn sign_in(
    client: &GoogleClient,
    scopes: &[String],
    open_url: impl FnOnce(&str),
) -> Result<GoogleOAuthCredential, String> {
    if scopes.is_empty() {
        return Err("the grant declares no scopes".into());
    }
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|_| "could not open a local port for the Google callback".to_owned())?;
    let port = listener
        .local_addr()
        .map_err(|_| "could not read the local callback port".to_owned())?
        .port();
    let redirect = format!("http://127.0.0.1:{port}");
    let state = random_url_value()?;
    let verifier = Zeroizing::new(random_url_value()?);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let scope = scopes
        .iter()
        .map(|s| scope_url(s))
        .collect::<Vec<_>>()
        .join(" ");

    let mut url = reqwest::Url::parse(AUTHORIZE_URL).map_err(|_| "invalid Google endpoint")?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", client.client_id.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("response_type", "code"),
        ("scope", scope.as_str()),
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ]);
    open_url(url.as_str());

    let code = wait_for_code(&listener, &state)?;
    let http = sjel_http::client(
        sjel_http::Purpose::new("google-oauth-sign-in"),
        Duration::from_secs(30),
    )
    .map_err(|_| "could not prepare the Google token exchange".to_owned())?;
    let response = http
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("client_id", client.client_id.as_str()),
            ("client_secret", client.client_secret.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .map_err(|_| "the Google token exchange failed; try again".to_owned())?;
    if !response.status().is_success() {
        // The body is not echoed: some error shapes carry token material.
        return Err(format!(
            "Google refused the token exchange (HTTP {})",
            response.status()
        ));
    }
    #[derive(Deserialize)]
    struct Exchange {
        refresh_token: Option<String>,
        scope: Option<String>,
    }
    let exchange: Exchange = response
        .json()
        .map_err(|_| "Google answered the token exchange without JSON".to_owned())?;
    let refresh_token = exchange
        .refresh_token
        .filter(|t| !t.is_empty())
        .ok_or("Google returned no refresh token")?;
    Ok(GoogleOAuthCredential {
        client_id: client.client_id.clone(),
        client_secret: client.client_secret.to_string(),
        refresh_token,
        scopes: exchange
            .scope
            .map(|s| s.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_else(|| scopes.iter().map(|s| scope_url(s)).collect()),
    })
}

/// Accepts connections until one carries this flow's `state`, then answers the browser.
fn wait_for_code(listener: &TcpListener, state: &str) -> Result<Zeroizing<String>, String> {
    listener
        .set_nonblocking(true)
        .map_err(|_| "could not prepare the Google callback".to_owned())?;
    let deadline = Instant::now() + CONSENT_WINDOW;
    loop {
        if Instant::now() >= deadline {
            return Err("Google sign-in timed out after 5 minutes".into());
        }
        let mut stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(_) => return Err("the Google callback failed".into()),
        };
        // An accepted socket inherits O_NONBLOCK on macOS (tools/sjel-cli/src/provider_login.rs).
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
        let Some(target) = line.split_whitespace().nth(1) else {
            continue;
        };
        let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else {
            continue;
        };
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        // A favicon request or a stray visit carries no state: ignore it and keep waiting.
        if params.get("state").map(String::as_str) != Some(state) {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
            continue;
        }
        let result = match (params.get("code"), params.get("error")) {
            (Some(code), _) if !code.is_empty() => Ok(Zeroizing::new(code.clone())),
            (_, Some(error)) => Err(format!("Google sign-in was not completed ({error})")),
            _ => Err("Google returned no authorization code".to_owned()),
        };
        let message = if result.is_ok() {
            "Sjel is connected to Google. You can close this tab."
        } else {
            "Google sign-in was not completed. Return to Sjel."
        };
        let body = format!(
            "<!doctype html><meta charset=utf-8><title>Sjel</title><p>{message}</p>"
        );
        let _ = stream.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        );
        return result;
    }
}

fn manager(catalog: &Path) -> Result<CredentialManager, String> {
    let providers: Vec<Arc<dyn CredentialProvider>> = vec![Arc::new(KeychainProvider::system())];
    CredentialManager::new(catalog, providers)
        .map_err(|_| "cannot open the private credential catalog".to_owned())
}

/// Writes the grant to the keychain under `id`, creating the catalog entry the first time.
pub fn store(
    catalog: &Path,
    id: &CredentialId,
    label: &str,
    consumers: Vec<String>,
    credential: &GoogleOAuthCredential,
) -> Result<(), String> {
    let encoded = Zeroizing::new(
        serde_json::to_string(credential).map_err(|_| "could not encode the grant".to_owned())?,
    );
    let secret = SecretValue::new(encoded.as_bytes().to_vec())
        .map_err(|_| "the grant is empty".to_owned())?;
    let manager = manager(catalog)?;
    let exists = manager
        .list()
        .map_err(|_| "cannot read the private credential catalog".to_owned())?
        .iter()
        .any(|entry| &entry.id == id);
    if exists {
        manager.update(id, &secret)
    } else {
        manager
            .create(
                id.clone(),
                label.to_owned(),
                ProviderKind::Keychain,
                consumers,
                &secret,
            )
            .map(|_| ())
    }
    .map_err(|_| "could not write the grant to the OS keychain".to_owned())
}

/// The grant under `id`, or `None` when the catalog does not name it.
fn from_keychain(catalog: &Path, id: &CredentialId) -> Result<Option<GoogleOAuthCredential>, String> {
    let manager = manager(catalog)?;
    let named = manager
        .list()
        .map_err(|_| "cannot read the private credential catalog".to_owned())?
        .iter()
        .any(|entry| &entry.id == id);
    if !named {
        return Ok(None);
    }
    let secret = manager
        .get(id)
        .map_err(|_| format!("cannot read the {id} grant from the OS keychain"))?;
    serde_json::from_slice(secret.expose_bytes())
        .map(Some)
        .map_err(|_| format!("the {id} keychain item is not a Google grant"))
}

/// The grant from a plaintext overlay env file, the form before 2026-10-23.
fn from_env(path: &Path) -> Result<GoogleOAuthCredential, String> {
    let body = Zeroizing::new(
        std::fs::read_to_string(path)
            .map_err(|_| format!("no Google grant in the keychain and none in {path:?}"))?,
    );
    let get = |key: &str| {
        body.lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("{key} is missing or empty in {path:?}"))
    };
    Ok(GoogleOAuthCredential {
        client_id: get("GOOGLE_CLIENT_ID")?,
        client_secret: get("GOOGLE_CLIENT_SECRET")?,
        refresh_token: get("GOOGLE_REFRESH_TOKEN")?,
        scopes: Vec::new(),
    })
}

/// The largest client JSON accepted. Google's is about 400 bytes.
const MAX_CLIENT_JSON: u64 = 64 * 1024;

/// Connects one grant from a downloaded client JSON file: read it into a zeroizing buffer, run
/// [`sign_in`], [`store`] the grant. The one path the desktop app (Settings › Accounts) and
/// `sjel credentials connect` both take. Blocks until the browser returns.
pub fn connect_from_file(
    catalog: &Path,
    id: &CredentialId,
    label: &str,
    capability: &str,
    scopes: &[String],
    client_file: &Path,
    open_url: impl FnOnce(&str),
) -> Result<(), String> {
    let size = std::fs::metadata(client_file)
        .map_err(|_| "cannot read the client JSON file".to_owned())?
        .len();
    if size > MAX_CLIENT_JSON {
        return Err("the file is too large to be a client JSON".into());
    }
    let text = Zeroizing::new(
        std::fs::read_to_string(client_file)
            .map_err(|_| "cannot read the client JSON file".to_owned())?,
    );
    let client = parse_client_json(&text)?;
    let grant = sign_in(&client, scopes, open_url)?;
    store(catalog, id, label, vec![capability.to_owned()], &grant)
}

/// The three keys a pre-keychain grant kept in an overlay env file.
const ENV_KEYS: [&str; 3] = [
    "GOOGLE_CLIENT_ID",
    "GOOGLE_CLIENT_SECRET",
    "GOOGLE_REFRESH_TOKEN",
];

/// Moves a grant from its old overlay env file into the keychain, with no new consent.
///
/// Order matters, because the env file is the only copy until the end: store in the keychain,
/// prove the keychain copy works with a real token refresh, and only then remove the three
/// `GOOGLE_*` lines from the env file. Other lines stay; a file left empty is deleted.
pub fn move_env_grant(
    catalog: &Path,
    id: &CredentialId,
    label: &str,
    consumers: Vec<String>,
    scopes: &[String],
    env_path: &Path,
) -> Result<(), String> {
    let mut grant = from_env(env_path)?;
    grant.scopes = scopes.iter().map(|s| scope_url(s)).collect();
    store(catalog, id, label, consumers, &grant)?;
    // A fallback path that cannot exist, so only the keychain item can answer.
    access_token(id.as_str(), Path::new("/nonexistent/sjel-keychain-check.env"))
        .map_err(|e| format!("the keychain copy does not work yet, so the env file stays: {e}"))?;
    strip_env_keys(env_path)
}

/// Removes the old env-file copy of a grant the keychain already holds, after a real refresh
/// proves the keychain grant works. For a grant connected fresh, whose env copy is older:
/// [`move_env_grant`] would put that older token back over it.
pub fn prune_env_grant(id: &CredentialId, env_path: &Path) -> Result<(), String> {
    access_token(id.as_str(), Path::new("/nonexistent/sjel-keychain-check.env"))
        .map_err(|e| format!("the keychain grant does not work, so the env file stays: {e}"))?;
    strip_env_keys(env_path)
}

/// Removes [`ENV_KEYS`] from `path`, keeping every other line, with an atomic owner-only
/// replace. Deletes the file when nothing else is left in it.
fn strip_env_keys(path: &Path) -> Result<(), String> {
    let body = Zeroizing::new(
        std::fs::read_to_string(path).map_err(|_| format!("cannot read {path:?}"))?,
    );
    let kept: Vec<&str> = body
        .lines()
        .filter(|l| !ENV_KEYS.iter().any(|k| l.starts_with(&format!("{k}="))))
        .collect();
    if kept.iter().all(|l| l.trim().is_empty() || l.trim_start().starts_with('#')) {
        return std::fs::remove_file(path).map_err(|_| format!("cannot remove {path:?}"));
    }
    let temp = path.with_extension("env.tmp");
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp)
            .map_err(|_| format!("cannot write {temp:?}"))?;
        file.write_all((kept.join("\n") + "\n").as_bytes())
            .map_err(|_| format!("cannot write {temp:?}"))?;
    }
    std::fs::rename(&temp, path).map_err(|_| format!("cannot replace {path:?}"))
}

struct Cached {
    token: Zeroizing<String>,
    expires_at: u64,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// An access token for the grant `id`, refreshed when the cached one is within 60 s of expiry.
///
/// The keychain item named in the overlay catalog wins; `env_fallback` is read only when the
/// catalog does not name `id`. The token is cached in this process and never persisted. A
/// failed refresh drops the cache entry, so a grant replaced in the keychain is picked up on the
/// next call without a restart.
pub fn access_token(id: &str, env_fallback: &Path) -> Result<String, String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Cached>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = cache
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(id)
        .filter(|c| c.expires_at > now_secs() + 60)
    {
        return Ok(hit.token.to_string());
    }

    let credential_id = CredentialId::parse(id).map_err(|_| format!("invalid grant id {id}"))?;
    let keychain = match sjel_config::overlay_config("credentials.toml") {
        Some(catalog) => from_keychain(&catalog, &credential_id)?,
        None => None,
    };
    let grant = match keychain {
        Some(grant) => grant,
        None => from_env(env_fallback)?,
    };

    let http = sjel_http::client(
        sjel_http::Purpose::new("google-oauth-refresh"),
        Duration::from_secs(30),
    )
    .map_err(|_| "could not prepare the Google token refresh".to_owned())?;
    // Repeated on a transport failure: a refresh mints a token and withdraws none, and a dark
    // wake once cut the first request off (capabilities/entities, measured 2026-10-18).
    let response = sjel_http::send(
        || {
            http.post(TOKEN_URL).form(&[
                ("client_id", grant.client_id.as_str()),
                ("client_secret", grant.client_secret.as_str()),
                ("refresh_token", grant.refresh_token.as_str()),
                ("grant_type", "refresh_token"),
            ])
        },
        sjel_http::Idempotent::Yes,
    )
    .map_err(|e| format!("the Google token refresh failed: {e}"))?;
    if !response.status().is_success() {
        cache.lock().unwrap_or_else(|p| p.into_inner()).remove(id);
        return Err(format!(
            "token refresh failed with HTTP {}",
            response.status()
        ));
    }
    #[derive(Deserialize)]
    struct Refreshed {
        access_token: String,
        #[serde(default = "an_hour")]
        expires_in: u64,
    }
    fn an_hour() -> u64 {
        3600
    }
    let refreshed: Refreshed = response
        .json()
        .map_err(|_| "Google answered the refresh without JSON".to_owned())?;
    cache.lock().unwrap_or_else(|p| p.into_inner()).insert(
        id.to_owned(),
        Cached {
            token: Zeroizing::new(refreshed.access_token.clone()),
            expires_at: now_secs() + refreshed.expires_in,
        },
    );
    Ok(refreshed.access_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_downloaded_client_json_parses_in_all_three_shapes() {
        for text in [
            r#"{"installed":{"client_id":"a.apps.googleusercontent.com","client_secret":"s"}}"#,
            r#"{"web":{"client_id":"a","client_secret":"s"}}"#,
            r#"{"client_id":"a","client_secret":"s"}"#,
        ] {
            let client = parse_client_json(text).unwrap_or_else(|e| panic!("{e}: {text}"));
            assert!(client.client_id.starts_with('a'));
        }
        assert!(parse_client_json(r#"{"installed":{"client_id":"a"}}"#).is_err());
        assert!(parse_client_json("~/Developer/Sjel main ❯").is_err());
    }

    #[test]
    fn short_scopes_expand_and_full_ones_stay() {
        assert_eq!(
            scope_url("gmail.modify"),
            "https://www.googleapis.com/auth/gmail.modify"
        );
        assert_eq!(scope_url("https://x/y"), "https://x/y");
    }

    #[test]
    fn the_env_fallback_reads_the_three_keys() {
        let dir = std::env::temp_dir().join(format!("sjel-google-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.env");
        std::fs::write(
            &path,
            "OTHER=1\nGOOGLE_CLIENT_ID=id\nGOOGLE_CLIENT_SECRET=s\nGOOGLE_REFRESH_TOKEN=r\n",
        )
        .unwrap();
        let grant = from_env(&path).unwrap();
        assert_eq!(
            (grant.client_id.as_str(), grant.refresh_token.as_str()),
            ("id", "r")
        );
        std::fs::write(&path, "GOOGLE_CLIENT_ID=id\n").unwrap();
        assert!(from_env(&path).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stripping_keeps_other_lines_and_deletes_a_file_left_empty() {
        let dir = std::env::temp_dir().join(format!("sjel-google-strip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mixed = dir.join("comms.env");
        std::fs::write(
            &mixed,
            "OTHER=1\nGOOGLE_CLIENT_ID=id\nGOOGLE_CLIENT_SECRET=s\nGOOGLE_REFRESH_TOKEN=r\n",
        )
        .unwrap();
        strip_env_keys(&mixed).unwrap();
        assert_eq!(std::fs::read_to_string(&mixed).unwrap(), "OTHER=1\n");
        let only = dir.join("entities.env");
        std::fs::write(&only, "# minted\nGOOGLE_CLIENT_ID=id\nGOOGLE_REFRESH_TOKEN=r\n").unwrap();
        strip_env_keys(&only).unwrap();
        assert!(!only.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
