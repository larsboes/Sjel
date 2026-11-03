//! Interactive provider sign-in lifecycle. OAuth tokens and hidden API keys go directly to the
//! OS keychain; the overlay catalog stores identifiers and labels only.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sjel_credentials::{
    CredentialId, CredentialManager, CredentialProvider, KeychainProvider, Presence, ProviderKind,
    SecretValue,
};
use std::io::{self, BufRead as _, IsTerminal as _, Write as _};
use std::net::TcpListener;
use std::process::{Command, ExitCode, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use zeroize::Zeroize as _;
use zeroize::Zeroizing;

const PROVIDERS: &[(&str, &str)] = &[
    ("openai", "OpenAI (Sign in with ChatGPT)"),
    ("openai-api", "OpenAI API key"),
    ("deepseek", "DeepSeek API"),
    ("opencode-go", "OpenCode Go API"),
    ("nvidia-nim", "NVIDIA NIM API"),
    ("gemini", "Google Gemini API (AI Studio)"),
];

pub fn run(args: &[String]) -> ExitCode {
    let result = run_inner(args);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("sjel provider: {message}");
            ExitCode::from(1)
        }
    }
}

fn run_inner(args: &[String]) -> Result<(), String> {
    let Some((verb, rest)) = args.split_first() else {
        return Err(usage());
    };
    match verb.as_str() {
        "list" | "status" if rest.is_empty() => list(),
        "login" if rest.is_empty() => {
            let selected = choose_provider()?;
            login(&selected)
        }
        "login" if rest.len() == 1 => login(&rest[0]),
        "logout" if rest.len() == 1 => logout(&rest[0]),
        "help" | "-h" | "--help" if rest.is_empty() => {
            println!("{}", usage());
            Ok(())
        }
        _ => Err(usage()),
    }
}

fn choose_provider() -> Result<String, String> {
    if !io::stdin().is_terminal() {
        return Err("provider selection requires an interactive terminal".into());
    }
    for (index, (slug, label)) in PROVIDERS.iter().enumerate() {
        println!("{}. {label} ({slug})", index + 1);
    }
    eprint!("Provider number: ");
    io::stderr()
        .flush()
        .map_err(|_| "could not display prompt".to_owned())?;
    let mut choice = String::new();
    io::stdin()
        .read_line(&mut choice)
        .map_err(|_| "could not read provider selection".to_owned())?;
    let index = choice
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=PROVIDERS.len()).contains(n))
        .ok_or_else(|| "invalid provider selection".to_owned())?;
    Ok(PROVIDERS[index - 1].0.to_owned())
}

fn usage() -> String {
    "usage: sjel provider {list|login [openai|openai-api|deepseek|opencode-go|nvidia-nim|gemini]|logout <provider>}".into()
}

fn manager() -> Result<CredentialManager, String> {
    let root = sjel_config::overlay_root().ok_or_else(|| {
        "private overlay is not configured (SJEL_PERSONAL_ROOT is unset)".to_owned()
    })?;
    let providers: Vec<Arc<dyn CredentialProvider>> = vec![Arc::new(KeychainProvider::system())];
    CredentialManager::new(root.join("config/credentials.toml"), providers)
        .map_err(|_| "cannot open the private credential catalog".into())
}

fn provider(value: &str) -> Result<(&'static str, &'static str, CredentialId), String> {
    let (slug, label) = PROVIDERS
        .iter()
        .copied()
        .find(|(slug, _)| *slug == value)
        .ok_or_else(|| "unknown provider; run `sjel provider list`".to_owned())?;
    let id = credential_id(slug)?;
    Ok((slug, label, id))
}

fn credential_id(slug: &str) -> Result<CredentialId, String> {
    let id = match slug {
        "openai" => "inference-openai-chatgpt",
        "openai-api" => "inference-openai",
        "deepseek" | "opencode-go" | "nvidia-nim" | "gemini" => {
            return CredentialId::parse(format!("inference-{slug}"))
                .map_err(|_| "invalid provider credential identifier".to_owned());
        }
        _ => return Err("unknown provider; run `sjel provider list`".to_owned()),
    };
    CredentialId::parse(id.to_owned())
        .map_err(|_| "invalid provider credential identifier".to_owned())
}

fn list() -> Result<(), String> {
    let manager = manager()?;
    for (slug, label) in PROVIDERS {
        let id = credential_id(slug)?;
        let status = manager
            .status(&id)
            .map(|presence| match presence {
                Presence::Present => "connected",
                Presence::Missing => "not connected",
            })
            .unwrap_or("not connected");
        println!("{slug:12} {status:14} {label}");
    }
    Ok(())
}

fn login(value: &str) -> Result<(), String> {
    let (_, label, id) = provider(value)?;
    if value == "openai" {
        return openai_oauth_login(id, label);
    }
    let key = read_hidden_key()?;
    let manager = manager()?;
    let entry = manager
        .list()
        .map_err(|_| "cannot read the private credential catalog".to_owned())?
        .into_iter()
        .find(|entry| entry.id == id);
    let secret = SecretValue::new(key.as_bytes().to_vec())
        .map_err(|_| "the API key was empty".to_owned())?;
    if entry.is_some() {
        manager
            .update(&id, &secret)
            .map_err(|_| "could not update the API key in the OS keychain".to_owned())?;
    } else {
        manager
            .create(
                id,
                label.to_owned(),
                ProviderKind::Keychain,
                vec!["sjel-inference".to_owned()],
                &secret,
            )
            .map_err(|_| "could not store the API key in the OS keychain".to_owned())?;
    }
    println!("{value} API key stored in the OS keychain.");
    Ok(())
}

fn logout(value: &str) -> Result<(), String> {
    let (_, _, id) = provider(value)?;
    manager()?
        .delete(&id)
        .map_err(|_| "could not remove this provider key from the OS keychain".to_owned())?;
    println!("{value} credentials removed from the OS keychain.");
    Ok(())
}

fn read_hidden_key() -> Result<Zeroizing<String>, String> {
    if !io::stdin().is_terminal() {
        return Err("login requires an interactive terminal; API keys are never accepted as arguments or piped input".into());
    }
    eprint!("API key (input hidden): ");
    io::stderr()
        .flush()
        .map_err(|_| "could not display prompt".to_owned())?;
    let interrupted = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&interrupted))
        .map_err(|_| "could not prepare safe terminal interruption".to_owned())?;
    let hidden = CommandEcho::disable()?;
    if interrupted.load(Ordering::Relaxed) {
        drop(hidden);
        return Err("login interrupted; terminal input was restored".into());
    }
    let mut value = Zeroizing::new(String::new());
    let read = io::stdin().read_line(&mut value);
    drop(hidden);
    eprintln!();
    if interrupted.load(Ordering::Relaxed) {
        return Err("login interrupted; terminal input was restored".into());
    }
    read.map_err(|_| "could not read the API key".to_owned())?;
    while value.ends_with(['\n', '\r']) {
        value.pop();
    }
    if value.trim().is_empty() {
        return Err("the API key was empty".into());
    }
    Ok(value)
}

const OPENAI_AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
const OPENAI_RESOURCE: &str = "https://api.openai.com/v1";
const OPENAI_REDIRECT_URI: &str = "http://127.0.0.1:1455/auth/callback";
const OPENAI_CALLBACK_PATH: &str = "/auth/callback";
const OPENAI_SCOPE: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";

#[derive(Deserialize, Serialize)]
pub(crate) struct OpenAiOAuthCredential {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: u64,
    pub client_id: String,
    pub scopes: Vec<String>,
}

impl Drop for OpenAiOAuthCredential {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
    }
}

#[derive(Deserialize)]
struct OpenAiOAuthTokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    scope: Option<String>,
}

struct OpenAiAuthorizationCallback {
    code: Zeroizing<String>,
    client_id: String,
}

fn openai_oauth_login(id: CredentialId, label: &str) -> Result<(), String> {
    if !io::stdin().is_terminal() {
        return Err("ChatGPT sign-in requires an interactive terminal".into());
    }
    let state = random_url_value()?;
    let nonce = random_url_value()?;
    let verifier = Zeroizing::new(random_url_value()?);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let host_id = openai_agent_host_id()?;

    let listener = TcpListener::bind("127.0.0.1:1455")
        .map_err(|_| "OpenAI sign-in needs localhost port 1455; close any other pending ChatGPT login and retry".to_owned())?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "could not prepare the local OpenAI sign-in callback".to_owned())?;
    let interrupted = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&interrupted))
        .map_err(|_| "could not prepare safe login interruption".to_owned())?;
    let callback_state = state.clone();
    let callback_interrupted = Arc::clone(&interrupted);
    let callback = std::thread::spawn(move || {
        wait_for_openai_callback(listener, &callback_state, &callback_interrupted)
    });

    let mut authorization = reqwest::Url::parse(OPENAI_AUTHORIZE_URL)
        .map_err(|_| "invalid OpenAI authorization endpoint".to_owned())?;
    authorization.query_pairs_mut().extend_pairs([
        ("client_id", "dynamic_agent_client"),
        ("agent_name_hint", "Sjel"),
        ("ext_agent_host_id", host_id.as_str()),
        ("response_type", "code"),
        ("redirect_uri", OPENAI_REDIRECT_URI),
        ("resource", OPENAI_RESOURCE),
        ("scope", OPENAI_SCOPE),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("nonce", nonce.as_str()),
    ]);
    let authorization_url = authorization.to_string();
    eprintln!("Opening ChatGPT sign-in in your browser…");
    if Command::new("open")
        .arg(&authorization_url)
        .spawn()
        .is_err()
    {
        eprintln!("Open this sign-in URL in your browser:\n{authorization_url}");
    }

    let result = callback
        .join()
        .map_err(|_| "OpenAI sign-in callback stopped unexpectedly".to_owned())??;
    if interrupted.load(Ordering::Relaxed) {
        return Err("ChatGPT sign-in was cancelled".into());
    }
    let credential =
        exchange_openai_authorization_code(&result.code, &verifier, &result.client_id)?;
    store_oauth_credential(id, label, &credential)?;
    println!("OpenAI ChatGPT sign-in stored in the OS keychain.");
    Ok(())
}

fn random_url_value() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|_| "could not create secure OpenAI sign-in state".to_owned())?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn openai_agent_host_id() -> Result<String, String> {
    let root = sjel_config::overlay_root().ok_or_else(|| {
        "private overlay is not configured (SJEL_PERSONAL_ROOT is unset)".to_owned()
    })?;
    let path = root.join("config/provider-device-id");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let existing = existing.trim();
        if valid_uuid(existing) {
            return Ok(format!("urn:uuid:{}", existing.to_ascii_lowercase()));
        }
        return Err("the saved provider device identifier is invalid".into());
    }

    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| "could not create provider device identifier".to_owned())?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let uuid = format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    );
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => {
            use std::io::Write as _;
            file.write_all(uuid.as_bytes())
                .map_err(|_| "could not save provider device identifier".to_owned())?;
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let existing = std::fs::read_to_string(&path)
                .map_err(|_| "could not read provider device identifier".to_owned())?;
            let existing = existing.trim();
            if valid_uuid(existing) {
                return Ok(format!("urn:uuid:{}", existing.to_ascii_lowercase()));
            }
            return Err("the saved provider device identifier is invalid".into());
        }
        Err(_) => return Err("could not save provider device identifier".into()),
    }
    Ok(format!("urn:uuid:{uuid}"))
}

fn valid_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23].into_iter().all(|i| bytes[i] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(i, byte)| [8, 13, 18, 23].contains(&i) || byte.is_ascii_hexdigit())
}

fn wait_for_openai_callback(
    listener: TcpListener,
    expected_state: &str,
    interrupted: &AtomicBool,
) -> Result<OpenAiAuthorizationCallback, String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(600);
    loop {
        if interrupted.load(Ordering::Relaxed) {
            return Err("ChatGPT sign-in was cancelled".into());
        }
        if std::time::Instant::now() >= deadline {
            return Err(
                "ChatGPT sign-in timed out; run `sjel provider login openai` to retry".into(),
            );
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                // On macOS an accepted socket inherits the listener's O_NONBLOCK, and a read
                // timeout does nothing on a non-blocking socket: a request still in flight fails
                // the read at once with WouldBlock instead of waiting the five seconds.
                stream
                    .set_nonblocking(false)
                    .map_err(|_| "could not read the local OpenAI sign-in callback".to_owned())?;
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .map_err(|_| "could not read the local OpenAI sign-in callback".to_owned())?;
                let mut reader =
                    io::BufReader::new(stream.try_clone().map_err(|_| {
                        "could not read the local OpenAI sign-in callback".to_owned()
                    })?);
                let mut request_line = String::new();
                reader
                    .read_line(&mut request_line)
                    .map_err(|_| "could not read the local OpenAI sign-in callback".to_owned())?;
                let target = request_line
                    .split_whitespace()
                    .nth(1)
                    .ok_or_else(|| "invalid local OpenAI sign-in callback".to_owned())?;
                let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}"))
                    .map_err(|_| "invalid local OpenAI sign-in callback".to_owned())?;
                let mut header = String::new();
                loop {
                    header.clear();
                    if reader
                        .read_line(&mut header)
                        .map_err(|_| "could not read local OpenAI sign-in headers".to_owned())?
                        == 0
                        || header == "\r\n"
                    {
                        break;
                    }
                }
                let result = if url.path() != OPENAI_CALLBACK_PATH
                    || url
                        .query_pairs()
                        .find(|(key, _)| key == "state")
                        .map(|(_, v)| v.to_string())
                        .as_deref()
                        != Some(expected_state)
                {
                    Err("OpenAI sign-in state did not match; refusing the callback".to_owned())
                } else {
                    let params = url
                        .query_pairs()
                        .into_owned()
                        .collect::<std::collections::HashMap<_, _>>();
                    match (params.get("code"), params.get("client_id")) {
                        (Some(code), Some(client_id))
                            if !code.is_empty() && !client_id.is_empty() =>
                        {
                            Ok(OpenAiAuthorizationCallback {
                                code: Zeroizing::new(code.clone()),
                                client_id: client_id.clone(),
                            })
                        }
                        _ => Err("OpenAI sign-in did not return an authorization code".to_owned()),
                    }
                };
                let (status, message) = if result.is_ok() {
                    (
                        "200 OK",
                        "ChatGPT sign-in complete. You can close this tab.",
                    )
                } else {
                    (
                        "400 Bad Request",
                        "ChatGPT sign-in could not be completed. Return to the terminal.",
                    )
                };
                let body = format!(
                    "<!doctype html><meta charset=utf-8><title>Sjel sign-in</title><p>{message}</p>"
                );
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                return result;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return Err("local OpenAI sign-in callback failed".into()),
        }
    }
}

fn exchange_openai_authorization_code(
    code: &str,
    verifier: &str,
    client_id: &str,
) -> Result<OpenAiOAuthCredential, String> {
    let client = sjel_http::client(
        sjel_http::Purpose::new("openai-oauth-login"),
        Duration::from_secs(30),
    )
    .map_err(|_| "could not prepare OpenAI token exchange".to_owned())?;
    let response = client
        .post(OPENAI_TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", OPENAI_REDIRECT_URI),
            ("resource", OPENAI_RESOURCE),
        ])
        .send()
        .map_err(|_| "OpenAI token exchange failed; retry ChatGPT sign-in".to_owned())?;
    if !response.status().is_success() {
        return Err(format!(
            "OpenAI token exchange failed (HTTP {})",
            response.status()
        ));
    }
    let token = response
        .json::<OpenAiOAuthTokenResponse>()
        .map_err(|_| "OpenAI returned an invalid OAuth token response".to_owned())?;
    if token.access_token.trim().is_empty()
        || token.refresh_token.trim().is_empty()
        || client_id.trim().is_empty()
    {
        return Err("OpenAI returned incomplete OAuth credentials".into());
    }
    let scopes = token
        .scope
        .as_deref()
        .unwrap_or(OPENAI_SCOPE)
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if !scopes
        .iter()
        .any(|scope| scope == "chatgpt.tokens.use.direct")
    {
        return Err("ChatGPT sign-in did not grant the OpenAI Responses API scope".into());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before Unix epoch".to_owned())?
        .as_millis();
    let expires_at_ms = u64::try_from(now.saturating_add(u128::from(token.expires_in) * 1000))
        .map_err(|_| "OpenAI token expiry is out of range".to_owned())?
        .saturating_sub(180_000);
    Ok(OpenAiOAuthCredential {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at_ms,
        client_id: client_id.to_owned(),
        scopes,
    })
}

fn store_oauth_credential(
    id: CredentialId,
    label: &str,
    credential: &OpenAiOAuthCredential,
) -> Result<(), String> {
    let encoded = Zeroizing::new(
        serde_json::to_string(credential)
            .map_err(|_| "could not encode OpenAI OAuth credentials".to_owned())?,
    );
    let secret = SecretValue::new(encoded.as_bytes().to_vec())
        .map_err(|_| "OpenAI returned empty credentials".to_owned())?;
    let manager = manager()?;
    let exists = manager
        .list()
        .map_err(|_| "cannot read the private credential catalog".to_owned())?
        .into_iter()
        .any(|entry| entry.id == id);
    if exists {
        manager
            .update(&id, &secret)
            .map_err(|_| "could not update OpenAI credentials in the OS keychain".to_owned())?;
    } else {
        manager
            .create(
                id,
                label.to_owned(),
                ProviderKind::Keychain,
                vec!["sjel-inference".to_owned()],
                &secret,
            )
            .map_err(|_| "could not store OpenAI credentials in the OS keychain".to_owned())?;
    }
    Ok(())
}

struct CommandEcho {
    original: String,
}

impl CommandEcho {
    fn disable() -> Result<Self, String> {
        let original = stty_command().arg("-g").output().map_err(|_| {
            "cannot read terminal settings; refusing to read a visible key".to_owned()
        })?;
        if !original.status.success() {
            return Err("cannot read terminal settings; refusing to read a visible key".into());
        }
        let original = String::from_utf8(original.stdout)
            .map_err(|_| {
                "cannot read terminal settings; refusing to read a visible key".to_owned()
            })?
            .trim()
            .to_owned();
        let status = stty_command().arg("-echo").status().map_err(|_| {
            "cannot disable terminal echo; refusing to read a visible key".to_owned()
        })?;
        if status.success() {
            Ok(Self { original })
        } else {
            Err("cannot disable terminal echo; refusing to read a visible key".into())
        }
    }
}

impl Drop for CommandEcho {
    fn drop(&mut self) {
        let _ = stty_command().arg(&self.original).status();
    }
}

fn stty_command() -> Command {
    // `stty` reads terminal attributes from stdin. `Command::output()` otherwise
    // gives the child a closed pipe, which makes terminal detection fail.
    let mut command = Command::new("stty");
    command.stdin(Stdio::inherit());
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_ids_match_inference_profile_references() {
        for (slug, _) in PROVIDERS {
            let (_, _, id) = provider(slug).unwrap();
            let expected = match *slug {
                "openai" => "inference-openai-chatgpt".to_owned(),
                "openai-api" => "inference-openai".to_owned(),
                other => format!("inference-{other}"),
            };
            assert_eq!(id.as_str(), expected);
        }
    }

    #[test]
    fn provider_catalog_separates_chatgpt_login_from_api_key() {
        assert!(provider("openai").unwrap().1.contains("ChatGPT"));
        assert_eq!(provider("openai-api").unwrap().1, "OpenAI API key");
    }

    #[test]
    fn uuid_validation_rejects_non_uuid_identifiers() {
        assert!(valid_uuid("d9428888-122b-4b68-8f4f-3c2d3f4d5e6f"));
        assert!(!valid_uuid("not-a-device-id"));
    }

    #[test]
    fn oauth_callback_checks_state_and_returns_the_code_and_client_id() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stopped = AtomicBool::new(false);
        let result = std::thread::scope(|scope| {
            let callback = scope.spawn(|| wait_for_openai_callback(listener, "state-value", &stopped));
            let mut browser = std::net::TcpStream::connect(address).unwrap();
            browser
                .write_all(b"GET /auth/callback?code=auth-code&client_id=agent-client&state=state-value HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
                .unwrap();
            callback.join().unwrap()
        })
        .unwrap();
        assert_eq!(result.code.as_str(), "auth-code");
        assert_eq!(result.client_id, "agent-client");
    }

    #[test]
    fn oauth_callback_rejects_a_mismatched_state() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stopped = AtomicBool::new(false);
        let result = std::thread::scope(|scope| {
            let callback = scope.spawn(|| wait_for_openai_callback(listener, "expected", &stopped));
            let mut browser = std::net::TcpStream::connect(address).unwrap();
            browser
                .write_all(b"GET /auth/callback?code=auth-code&client_id=agent-client&state=wrong HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
                .unwrap();
            callback.join().unwrap()
        });
        let Err(error) = result else {
            panic!("mismatched OAuth state was accepted");
        };
        assert!(error.contains("state did not match"), "{error}");
    }

    #[test]
    fn invalid_provider_input_is_never_echoed() {
        let error = provider("not-a-provider").unwrap_err();
        assert!(!error.contains("not-a-provider"));
    }
}
