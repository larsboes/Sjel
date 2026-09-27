use super::*;

pub(super) const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub(super) const CALENDAR_API: &str = "https://www.googleapis.com/calendar/v3/calendars";

/// The OAuth scopes this capability needs. Import alone is satisfied by the
/// readonly scope; `calendar.events` covers reading *and* writing events and
/// is therefore the one grant that serves both runs.
///
/// `capabilities/comms/auth/get-refresh-token.ts` already requests
/// `calendar.events`, so an operator who has run that bootstrap can point
/// `google.env_path` at `comms.env` instead of minting a second grant.
pub const SCOPE_READONLY: &str = "https://www.googleapis.com/auth/calendar.readonly";
pub const SCOPE_EVENTS: &str = "https://www.googleapis.com/auth/calendar.events";

pub(super) const REQUIRED_KEYS: [&str; 3] = [
    "GOOGLE_CLIENT_ID",
    "GOOGLE_CLIENT_SECRET",
    "GOOGLE_REFRESH_TOKEN",
];

pub(super) type SyncResult<T> = Result<T, String>;

/// A deliberately bounded review window.  Google can hold a lifetime of
/// history and recurring instances; a review surface that quietly turns into
/// a bulk import is neither useful nor safe.
pub const MAX_REVIEW_DAYS: i64 = 90;

// ---- resolved, checked settings -------------------------------------------

/// The two values Phase E refuses to guess, resolved once so both runs fail
/// the same way and fail before any network call.
#[derive(Debug, Clone)]
pub struct Settings {
    pub tz: HomeTimezone,
    pub calendar_id: String,
    pub google: GoogleConfig,
}

impl Settings {
    pub fn resolve(config: &Config) -> SyncResult<Self> {
        let path = crate::config::config_path();
        let name = config.home_timezone.as_deref().ok_or_else(|| {
            format!(
                "home_timezone is not configured. Google events carry a real UTC offset and this \
                 capability stores naive local wall time, so a sync cannot convert between them \
                 without knowing the operator's zone. Declare SJEL_HOME_TIMEZONE in the overlay's \
                 config/deployment.env — scouting resolves the same value — or set \
                 \"home_timezone\" in {path:?} for this capability alone (README § Time model)."
            )
        })?;
        let tz = HomeTimezone::parse(name)?;
        let calendar_id = config
            .google
            .calendar_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                format!(
                    "google.calendar_id is not configured. Set \"google\": {{\"calendar_id\": \
                     \"primary\"}} in {path:?}, or the \
                     …@group.calendar.google.com id of a secondary calendar. There is no default: \
                     guessing would sync the wrong calendar."
                )
            })?
            .to_string();
        Ok(Self {
            tz,
            calendar_id,
            google: config.google.clone(),
        })
    }
}

// ---- credentials ----------------------------------------------------------

pub(super) struct CachedToken {
    /// The credential file this token came from. Keyed rather than global: a
    /// cache shared across two credential files would hand one calendar's
    /// token to the other's account, and the failure would look like a
    /// permissions problem rather than a caching one.
    source: std::path::PathBuf,
    token: String,
    expires_at: u64,
}

/// In-process only, never persisted. Refreshed within 60s of expiry.
pub(super) static TOKEN_CACHE: OnceLock<Mutex<Option<CachedToken>>> = OnceLock::new();

pub(super) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Reads one `KEY=value` from a plain env file. The value is returned but
/// never logged, and the error deliberately names only the key and the path.
pub(super) fn read_env_key(env_path: &Path, key: &str) -> SyncResult<String> {
    let body = std::fs::read_to_string(env_path).map_err(|error| {
        format!(
            "cannot read the Google credential file {env_path:?}: {error}. Phase E expects \
             {} in that file. It lives in the private overlay and is never part of this repo; \
             see README § Phases > E for the steps that produce it.",
            REQUIRED_KEYS.join(", ")
        )
    })?;
    body.lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix(&format!("{key}="))
                .map(|value| value.trim().to_string())
        })
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!(
                "{key} is missing or empty in {env_path:?}. Phase E needs all of {}.",
                REQUIRED_KEYS.join(", ")
            )
        })
}

pub(super) fn http_client() -> SyncResult<reqwest::blocking::Client> {
    // 20s, matching calendar's other outbound calls. It carried no timeout at
    // all until sjel_http made one an argument, so a hung Google token endpoint
    // hung the sync thread with it.
    sjel_http::builder(
        sjel_http::Purpose::new("calendar-google-auth"),
        std::time::Duration::from_secs(20),
    )
    .gzip(true)
    .build()
    .map_err(|error| format!("could not build an HTTP client: {error}"))
}

#[derive(Deserialize)]
pub(super) struct TokenResponse {
    access_token: String,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
}

pub(super) fn default_expires_in() -> u64 {
    3600
}

/// A valid access token, refreshed through the OAuth refresh grant when the
/// cache is cold or near expiry.
///
/// The failed-refresh path prints the status and nothing else: Google's error
/// bodies can echo token material.
pub fn access_token(env_path: &Path) -> SyncResult<String> {
    let cache = TOKEN_CACHE.get_or_init(|| Mutex::new(None));
    if let Some(cached) = cache.lock().unwrap().as_ref() {
        if cached.source == env_path && cached.expires_at > now_secs() + 60 {
            return Ok(cached.token.clone());
        }
    }

    let client_id = read_env_key(env_path, "GOOGLE_CLIENT_ID")?;
    let client_secret = read_env_key(env_path, "GOOGLE_CLIENT_SECRET")?;
    let refresh_token = read_env_key(env_path, "GOOGLE_REFRESH_TOKEN")?;

    let response = http_client()?
        .post(TOKEN_URL)
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .map_err(|error| format!("token refresh could not reach Google: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "token refresh failed with HTTP {} — the refresh token in {env_path:?} is probably \
             revoked or was issued for different scopes (import needs {SCOPE_READONLY}, export \
             needs {SCOPE_EVENTS}). Re-run the consent step in README § Phases > E.",
            response.status()
        ));
    }

    let token: TokenResponse = response
        .json()
        .map_err(|error| format!("token refresh returned an unreadable body: {error}"))?;
    *cache.lock().unwrap() = Some(CachedToken {
        source: env_path.to_path_buf(),
        token: token.access_token.clone(),
        expires_at: now_secs() + token.expires_in,
    });
    Ok(token.access_token)
}
