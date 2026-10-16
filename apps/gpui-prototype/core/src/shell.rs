//! Client for the Sjel shell (`capabilities/sjel-status`), authenticated with the inbound token.
//!
//! The same contracts and token as the menu-bar app (`apps/mac/Sources/SjelRelay`): the token
//! comes from the login Keychain item that `tools/setup-inbound-auth.sh` writes, through
//! `/usr/bin/security`, which that script puts on the item's access list.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::process::Command;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use reqwest::{Method, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::Deserialize;

/// The default shell endpoint, from `capabilities/sjel-status/service.toml`.
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:8082";
const KEYCHAIN_SERVICE: &str = "sjel-inbound-token";
const TICKET_PATH: &str = "/api/sjel-status/session/ticket";
const OPEN_PATH: &str = "/session/open?";

/// `GET /api/sjel-status/health` (`AxonStatusHealth` in
/// `capabilities/sjel-status/src/status/registry.rs`).
#[derive(Clone, Debug, Deserialize)]
pub struct NodeHealth {
    pub version: String,
    pub uptime_seconds: u64,
    pub capabilities: BTreeMap<String, CapabilityHealth>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct CapabilityHealth {
    pub up: bool,
    /// Kept running by the machine. A shell from before the field kept every capability running.
    #[serde(default = "kept_running")]
    pub autostart: bool,
}

fn kept_running() -> bool {
    true
}

/// What a capability's state means to the reader. On-demand capabilities that are stopped are
/// normal, not down (`NodeHealth.down` in `apps/mac/Sources/SjelRelay/NodeBridge.swift`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Liveness {
    Up,
    Idle,
    Down,
}

impl CapabilityHealth {
    pub fn liveness(self) -> Liveness {
        match (self.up, self.autostart) {
            (true, _) => Liveness::Up,
            (false, false) => Liveness::Idle,
            (false, true) => Liveness::Down,
        }
    }
}

impl NodeHealth {
    /// Kept-running capabilities that did not answer, by name.
    pub fn down(&self) -> Vec<String> {
        self.capabilities
            .iter()
            .filter(|(_, health)| health.liveness() == Liveness::Down)
            .map(|(name, _)| name.clone())
            .collect()
    }
}

/// `GET /api/sjel-status/agent`.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct AgentView {
    #[serde(default)]
    pub pending: Vec<Approval>,
    /// Newest first, at most 50.
    #[serde(default)]
    pub calls: Vec<AgentCall>,
}

/// One agent write that waits for the owner.
#[derive(Clone, Debug, Deserialize)]
pub struct Approval {
    pub id: String,
    pub capability: String,
    pub method: String,
    pub path: String,
    pub preview: String,
}

/// One call an agent made through the gate (`AgentCall` in `libs/sjel-server/src/agent_policy.rs`).
#[derive(Clone, Debug, Deserialize)]
pub struct AgentCall {
    /// Unix seconds.
    pub at: i64,
    pub capability: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    /// `read` for a plain read; anything else is a write or a refusal.
    pub decision: String,
}

/// `GET /calendar/api/entries` (`CalendarEntry` in `dashboard/src/lib/api.ts`).
#[derive(Clone, Debug, Deserialize)]
pub struct CalendarEntry {
    pub title: String,
    /// `YYYY-MM-DD` when `all_day`, else local `YYYY-MM-DDTHH:MM:SS`.
    pub starts_at: String,
    /// Exclusive for an all-day entry.
    pub ends_at: String,
    pub all_day: bool,
    /// `possible`, `planned` or `committed`.
    pub commitment: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellError {
    /// The endpoint is not on loopback.
    NotLoopback,
    /// No token in the Keychain.
    NotSetUp,
    /// Nothing answered.
    Unreachable,
    /// The shell answered with an error status, and its `error` text when it sent one.
    Refused(u16, Option<String>),
    /// The shell answered 200 with a body this client cannot read.
    Unreadable,
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLoopback => {
                f.write_str("SJEL_STATUS_URL must point to 127.0.0.1 or localhost.")
            }
            Self::NotSetUp => {
                f.write_str("No Sjel token on this Mac. Run tools/setup-inbound-auth.sh.")
            }
            Self::Unreachable => f.write_str("Sjel is not running on this Mac."),
            Self::Refused(_, Some(error)) => f.write_str(error),
            Self::Refused(status, None) => write!(f, "Sjel refused the request ({status})."),
            Self::Unreadable => f.write_str("Sjel answered with something this app cannot read."),
        }
    }
}

pub type ShellResult<T> = Result<T, ShellError>;

pub struct Shell {
    base: Url,
    client: reqwest::Client,
    token: Mutex<Option<String>>,
}

impl Shell {
    /// # Errors
    /// `NotLoopback` when `endpoint` is not an `http` URL on 127.0.0.1 or localhost.
    pub fn new(endpoint: &str) -> ShellResult<Self> {
        let base = Url::parse(endpoint).map_err(|_| ShellError::NotLoopback)?;
        let loopback =
            base.scheme() == "http" && matches!(base.host_str(), Some("127.0.0.1" | "localhost"));
        if !loopback {
            return Err(ShellError::NotLoopback);
        }
        Ok(Self {
            base,
            client: reqwest::Client::new(),
            token: Mutex::new(None),
        })
    }

    pub fn endpoint(&self) -> &str {
        self.base.as_str().trim_end_matches('/')
    }

    pub async fn health(&self) -> ShellResult<NodeHealth> {
        self.json(Method::GET, "/api/sjel-status/health", None, 10)
            .await
    }

    pub async fn agent(&self) -> ShellResult<AgentView> {
        self.json(Method::GET, "/api/sjel-status/agent", None, 5)
            .await
    }

    /// Entries from `from` to `to`, both `YYYY-MM-DD`, both inclusive.
    pub async fn calendar(&self, from: &str, to: &str) -> ShellResult<Vec<CalendarEntry>> {
        let path = format!("/calendar/api/entries?from={from}&to={to}");
        self.json(Method::GET, &path, None, 5).await
    }

    /// Allows or denies one waiting agent write.
    pub async fn decide(&self, id: &str, allow: bool) -> ShellResult<()> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ShellError::Refused(400, Some("Not an approval id.".into())));
        }
        let path = format!("/api/sjel-status/agent/approvals/{id}");
        let body = serde_json::json!({ "allow": allow });
        self.send(Method::POST, &path, Some(body), 5)
            .await
            .map(drop)
    }

    /// Starts or stops one capability. A start waits for the service to come up.
    pub async fn set_running(&self, name: &str, running: bool) -> ShellResult<()> {
        if !is_registry_name(name) {
            return Err(ShellError::Refused(
                400,
                Some("Not a capability name.".into()),
            ));
        }
        let action = if running { "start" } else { "stop" };
        let path = format!("/api/sjel-status/capabilities/{name}/{action}");
        self.send(Method::POST, &path, None, 60).await.map(drop)
    }

    /// A single-use URL that logs the browser in and lands on `next`
    /// (`landing` in `capabilities/sjel-status/src/session.rs`).
    pub async fn dashboard_url(&self, next: &str) -> ShellResult<Url> {
        #[derive(Deserialize)]
        struct Ticket {
            open: String,
        }
        let ticket: Ticket = self.json(Method::POST, TICKET_PATH, None, 5).await?;
        landing_url(&self.base, &ticket.open, next).ok_or(ShellError::Unreadable)
    }

    async fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
        timeout_secs: u64,
    ) -> ShellResult<T> {
        let response = self.send(method, path, body, timeout_secs).await?;
        response.json().await.map_err(|_| ShellError::Unreadable)
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
        timeout_secs: u64,
    ) -> ShellResult<reqwest::Response> {
        let token = self.token()?;
        let url = self.base.join(path).map_err(|_| ShellError::Unreadable)?;
        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(token)
            .timeout(Duration::from_secs(timeout_secs));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|_| ShellError::Unreachable)?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED {
            // The token was rotated: read the Keychain again on the next request.
            *self.token.lock().expect("token lock") = None;
        }
        if status.is_success() {
            return Ok(response);
        }
        let error = response
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|body| body.get("error")?.as_str().map(str::to_owned));
        Err(ShellError::Refused(status.as_u16(), error))
    }

    fn token(&self) -> ShellResult<String> {
        let mut cached = self.token.lock().expect("token lock");
        if let Some(token) = cached.as_ref() {
            return Ok(token.clone());
        }
        let user = std::env::var("USER").map_err(|_| ShellError::NotSetUp)?;
        let output = Command::new("/usr/bin/security")
            .args([
                "find-generic-password",
                "-a",
                &user,
                "-s",
                KEYCHAIN_SERVICE,
                "-w",
            ])
            .output()
            .map_err(|_| ShellError::NotSetUp)?;
        let token = String::from_utf8(output.stdout).map_err(|_| ShellError::NotSetUp)?;
        let token = token.trim();
        if !output.status.success() || token.is_empty() {
            return Err(ShellError::NotSetUp);
        }
        *cached = Some(token.to_owned());
        Ok(token.to_owned())
    }
}

/// Letters, digits and dashes: nothing a path can be built from.
fn is_registry_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Accepts only `/session/open?…` on the shell's own origin, so a wrong answer cannot send the
/// browser anywhere else.
fn landing_url(base: &Url, open: &str, next: &str) -> Option<Url> {
    if !open.starts_with(OPEN_PATH) {
        return None;
    }
    let mut url = base.join(open).ok()?;
    if url.origin() != base.origin() {
        return None;
    }
    url.query_pairs_mut().append_pair("next", next);
    Some(url)
}

/// One Tokio runtime on its own thread for every request. GPUI's executor does not provide
/// Tokio's reactor, which reqwest needs.
fn runtime() -> &'static tokio::runtime::Handle {
    static RUNTIME: LazyLock<tokio::runtime::Handle> = LazyLock::new(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build the HTTP runtime");
        let handle = runtime.handle().clone();
        std::thread::Builder::new()
            .name("sjel-http".into())
            .spawn(move || runtime.block_on(std::future::pending::<()>()))
            .expect("start the HTTP thread");
        handle
    });
    &RUNTIME
}

/// Runs `work` on the HTTP runtime; the returned future can be awaited from GPUI.
pub fn run<T: Send + 'static>(
    work: impl Future<Output = T> + Send + 'static,
) -> impl Future<Output = Option<T>> {
    let (tx, rx) = futures::channel::oneshot::channel();
    runtime().spawn(async move {
        let _ = tx.send(work.await);
    });
    async move { rx.await.ok() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_must_be_loopback_http() {
        assert!(Shell::new("http://127.0.0.1:8082").is_ok());
        assert!(Shell::new("http://localhost:9000/").is_ok());
        assert_eq!(
            Shell::new("https://127.0.0.1:8082").err(),
            Some(ShellError::NotLoopback)
        );
        assert_eq!(
            Shell::new("http://example.com:8082").err(),
            Some(ShellError::NotLoopback)
        );
        assert_eq!(
            Shell::new("http://127.0.0.1.evil:8082").err(),
            Some(ShellError::NotLoopback)
        );
    }

    #[test]
    fn landing_stays_on_the_shell() {
        let base = Url::parse(DEFAULT_ENDPOINT).unwrap();
        let url = landing_url(&base, "/session/open?ticket=ab", "/calendar").unwrap();
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:8082/session/open?ticket=ab&next=%2Fcalendar"
        );
        assert!(landing_url(&base, "//evil.example/session/open?x", "/").is_none());
        assert!(landing_url(&base, "/elsewhere?ticket=ab", "/").is_none());
    }

    #[test]
    fn stopped_on_demand_capability_is_idle_not_down() {
        let health: NodeHealth = serde_json::from_str(
            r#"{"ok":true,"version":"v1","uptime_seconds":5,"capabilities":{
                "a":{"up":true,"autostart":true},
                "b":{"up":false,"autostart":false},
                "c":{"up":false}
            }}"#,
        )
        .unwrap();
        assert_eq!(health.capabilities["b"].liveness(), Liveness::Idle);
        assert_eq!(health.down(), vec!["c".to_owned()]);
    }

    #[test]
    fn registry_names_reject_paths() {
        assert!(is_registry_name("sjel-status"));
        assert!(!is_registry_name("../x"));
        assert!(!is_registry_name(""));
    }
}
