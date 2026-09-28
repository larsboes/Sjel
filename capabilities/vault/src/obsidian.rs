//! The Obsidian Local REST API, which is the only way this capability writes.
//!
//! There is no file-write fallback in this module or anywhere behind it. If
//! Obsidian is not running, [`Obsidian::connect`] refuses with a message that
//! names that, and the run stops having written nothing.
//!
//! ## Why the API rather than the file
//!
//! Obsidian holds the vault open. A file written underneath it races the editor
//! and the metadata cache: a Base reads the cache, and a note whose cache entry
//! is stale shows the old `last_contact` until something touches it again. A
//! write through the plugin goes through Obsidian's own vault adapter, so the
//! cache, the Bases view and the file agree when the call returns.
//!
//! ## Why a whole-file `PUT` and not the frontmatter `PATCH`
//!
//! The plugin can set one frontmatter key with
//! `PATCH /vault/{path}` and `{"targetType":"frontmatter","target":<key>,…}`.
//! That call re-serialises the whole frontmatter block through Obsidian's YAML
//! writer. Measured against a live vault when this writer was first built: it
//! turned `last_contact: "2026-06-17"` into `last_contact: 2026-06-17`, and did
//! the same to `met_at`, a key the call never named. A writer that promises to
//! change one key cannot use it.
//!
//! So a write is: read the note's bytes with `GET`, splice one scalar locally
//! with `markdown_root::set_field` (every other byte kept), and store the result
//! with `PUT /vault/{path}` and `Content-Type: application/octet-stream`. The
//! upstream contract says a body of that type is "stored as raw bytes", so no
//! text or JSON parser sits between this code and the file.
//!
//! ## The contract, read rather than guessed
//!
//! From the plugin's OpenAPI description, `docs/openapi.yaml` in
//! <https://github.com/coddingtonbear/obsidian-local-rest-api> (5.3.1, read
//! 2026-09-29):
//!
//! - `GET /vault/{path}` returns the note's content.
//! - `PUT /vault/{path}` replaces the whole file and answers `204`. A body that
//!   is not text or JSON is stored as raw bytes.
//! - `PUT` has **no precondition**. Only `PATCH` takes an `ifMatch` token, and
//!   `PATCH` is the call that re-serialises. The caller therefore compares the
//!   bytes it reads immediately before the `PUT` with the bytes it planned
//!   from, and reads the note again after the `PUT` to compare it with what it
//!   sent (`fields::apply`). The window that remains is the time between one
//!   `GET` and the next `PUT` on the same note.
//! - Auth is `Authorization: Bearer <key>`. `GET /` answers `200` to any caller
//!   and reports `authenticated`, so the probe reads that field.
//!
//! ## The key
//!
//! Resolved at runtime and never from this repo, in the order below. Nothing in
//! this module logs, formats or returns the value; reqwest carries it in a
//! header and no error path prints headers.
//!
//! 1. `OBSIDIAN_API_KEY` in the environment.
//! 2. `obsidian_api_key_file` in the overlay's `config/knowledge.toml`, read as
//!    a path relative to `config/` or as an absolute or `~`-prefixed path.
//!
//! The base URL comes from `OBSIDIAN_API_URL`, then `obsidian_api_url` in the
//! same file, then `http://127.0.0.1:27123`.

use std::time::Duration;

/// A connected Local REST API, proven to be answering before any write.
pub struct Obsidian {
    base: String,
    key: String,
    http: reqwest::blocking::Client,
}

/// How long to wait on the loopback plugin. Long enough for Obsidian to be
/// busy, short enough that a stopped Obsidian is a refusal and not a hang.
const TIMEOUT: Duration = Duration::from_secs(10);

const DEFAULT_URL: &str = "http://127.0.0.1:27123";

impl Obsidian {
    /// Resolve the URL and the key from the environment and the overlay, then
    /// prove the plugin is answering and accepts the key.
    pub fn connect() -> Result<Self, String> {
        let settings = settings();
        let base = std::env::var("OBSIDIAN_API_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| {
                settings
                    .as_ref()
                    .and_then(|s| lookup(s, "obsidian_api_url"))
            })
            .unwrap_or_else(|| DEFAULT_URL.to_string());
        let key = resolve_key(settings.as_deref())?;
        Self::connect_to(&base, key)
    }

    /// Prove that the plugin at `base` answers and accepts `key`.
    ///
    /// The probe is not optional. Without it the first refusal an operator sees
    /// is a connection error in the middle of a run that has already written
    /// some notes and not others.
    pub fn connect_to(base: &str, key: String) -> Result<Self, String> {
        let base = base.trim().trim_end_matches('/').to_string();
        let http = sjel_http::client(sjel_http::Purpose::new("vault-obsidian"), TIMEOUT)
            .map_err(|e| format!("building the HTTP client: {e}"))?;

        let probe = http
            .get(format!("{base}/"))
            .bearer_auth(&key)
            .send()
            .map_err(|e| not_running(&base, &e))?;
        if !probe.status().is_success() {
            return Err(format!(
                "the Local REST API at {base} answered {} to a probe of `/`. \
                 Refusing before any write; there is no file-write fallback.",
                probe.status()
            ));
        }

        // `GET /` answers 200 to an unauthenticated caller and reports whether
        // the credential it was handed was accepted. Reading the status alone
        // would let a wrong key through the probe and turn every note into its
        // own 401 halfway down the run.
        let body: serde_json::Value = probe
            .text()
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .ok_or_else(|| format!("the Local REST API at {base} answered `/` unreadably"))?;
        if body.get("authenticated").and_then(|v| v.as_bool()) != Some(true) {
            return Err(format!(
                "the Local REST API at {base} did not accept the key. It is not the one \
                 Obsidian holds under Settings > Local REST API. Refusing before any write."
            ));
        }

        Ok(Obsidian { base, key, http })
    }

    /// The base URL, for a report that has to say where the writes went.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// A note's bytes exactly as the vault holds them.
    pub fn read(&self, path: &str) -> Result<String, String> {
        let url = format!("{}/vault/{}", self.base, encode_path(path));
        let response = self
            .http
            .get(&url)
            .bearer_auth(&self.key)
            .header("Accept", "text/markdown")
            .send()
            .map_err(|e| format!("reading {path}: {e}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("reading {path}: the API answered {status}"));
        }
        response
            .text()
            .map_err(|e| format!("reading {path}: decoding the body: {e}"))
    }

    /// Replace a note's whole content with exactly `text`.
    ///
    /// `application/octet-stream` so the plugin stores the bytes as sent and
    /// runs no text parser over them (see the module docs). The caller is
    /// responsible for `text` being the note it read with one scalar changed.
    pub fn write(&self, path: &str, text: &str) -> Result<(), String> {
        let url = format!("{}/vault/{}", self.base, encode_path(path));
        let response = self
            .http
            .put(&url)
            .bearer_auth(&self.key)
            .header("Content-Type", "application/octet-stream")
            .body(text.to_string())
            .send()
            .map_err(|e| format!("writing {path}: {e}"))?;
        let status = response.status();
        if !status.is_success() {
            // The body carries the plugin's own `errorCode`/`message`. It
            // describes the request, not the credential.
            let detail = response.text().unwrap_or_default();
            let detail = detail.trim();
            return Err(format!(
                "writing {path}: the API answered {status}{}",
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            ));
        }
        Ok(())
    }
}

/// The refusal for an Obsidian that is not listening.
fn not_running(base: &str, error: &reqwest::Error) -> String {
    if error.is_connect() {
        return format!(
            "Obsidian is not running, or its Local REST API plugin is not listening on {base}. \
             Nothing was written, and there is no file-write fallback. Start Obsidian and \
             run this again."
        );
    }
    format!("reaching the Local REST API at {base}: {error}")
}

/// The overlay's `config/knowledge.toml`, or `None` when there is no overlay.
fn settings() -> Option<String> {
    let path = sjel_config::overlay_config("knowledge.toml")?;
    std::fs::read_to_string(path).ok()
}

/// One key out of a flat TOML file, by line scan.
///
/// The same move `note::resolve_root` makes on the same file and for the same
/// reason: this needs two keys out of it, and a TOML dependency to read two
/// keys is a dependency to audit.
fn lookup(text: &str, key: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().trim_matches('"').to_string())
        .filter(|v| !v.is_empty())
}

/// The API key, from the environment or the overlay.
fn resolve_key(settings: Option<&str>) -> Result<String, String> {
    resolve_key_from(settings, std::env::var("OBSIDIAN_API_KEY").ok())
}

/// [`resolve_key`] with the environment value passed in rather than read.
///
/// A parameter so a test can pin the overlay path without depending on the
/// shell it runs in: `OBSIDIAN_API_KEY` is one of the two documented ways to
/// supply the key, and a test that read it would fail on the machine of every
/// operator who uses it. Every error names a *location*, never a value.
fn resolve_key_from(settings: Option<&str>, from_env: Option<String>) -> Result<String, String> {
    if let Some(value) = from_env {
        let value = value.trim().to_string();
        if !value.is_empty() {
            return Ok(value);
        }
    }

    let Some(settings) = settings else {
        return Err(
            "no overlay: set SJEL_PERSONAL_ROOT, or put the key in OBSIDIAN_API_KEY".to_string(),
        );
    };
    let Some(declared) = lookup(settings, "obsidian_api_key_file") else {
        return Err(
            "the overlay's config/knowledge.toml declares no `obsidian_api_key_file`, and \
             OBSIDIAN_API_KEY is unset. Point it at a runtime-secrets file holding the \
             Local REST API key, the way config/inference.json points at its own."
                .to_string(),
        );
    };

    // Relative paths resolve under `config/`, which is where `runtime-secrets/`
    // lives and the shape `inference.json` already declares.
    let path = if declared.starts_with('/') || declared.starts_with('~') {
        sjel_config::expand_tilde(&declared)
    } else {
        match sjel_config::overlay_config(&declared) {
            Some(p) => p,
            None => return Err("no overlay: set SJEL_PERSONAL_ROOT".to_string()),
        }
    };

    let value = std::fs::read_to_string(&path)
        .map_err(|e| format!("reading the key file at {}: {e}", path.display()))?;
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(format!("the key file at {} is empty", path.display()));
    }
    Ok(value)
}

/// Percent-encode a vault path for a URL, keeping `/` as the separator.
///
/// Hand-rolled rather than taken as a dependency: the rule is one line and the
/// input is a vault-relative note id. People notes are named after people, so
/// spaces and non-ASCII letters are the normal case, not the edge.
pub(crate) fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 8);
    for byte in path.as_bytes() {
        match byte {
            b'/' | b'-' | b'.' | b'_' | b'~' => out.push(*byte as char),
            b if b.is_ascii_alphanumeric() => out.push(*b as char),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A stand-in for the Local REST API, for tests only.
///
/// A loopback listener on an ephemeral port that answers the three calls this
/// module makes (`GET /`, `GET /vault/…`, `PUT /vault/…`) from an in-memory map,
/// and records every request line. No test in this crate reaches a real
/// Obsidian.
#[cfg(test)]
pub(crate) mod mock {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    pub const KEY: &str = "test-key-not-a-secret";

    #[derive(Default)]
    struct State {
        /// Note bytes, keyed by the encoded vault path.
        notes: HashMap<String, Vec<u8>>,
        /// `METHOD /path content-type` per request, in order.
        log: Vec<String>,
        /// Simulate a serialiser that drops double quotes on every stored
        /// note, to prove the after-read catches it.
        strip_quotes_on_put: bool,
    }

    pub struct Server {
        pub base: String,
        state: Arc<Mutex<State>>,
    }

    impl Server {
        pub fn start() -> Server {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let base = format!("http://{}", listener.local_addr().expect("addr"));
            let state = Arc::new(Mutex::new(State::default()));
            let shared = Arc::clone(&state);
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    handle(stream, &shared);
                }
            });
            Server { base, state }
        }

        pub fn put_note(&self, path: &str, text: &str) {
            self.state
                .lock()
                .unwrap()
                .notes
                .insert(super::encode_path(path), text.as_bytes().to_vec());
        }

        pub fn note(&self, path: &str) -> Option<String> {
            self.state
                .lock()
                .unwrap()
                .notes
                .get(&super::encode_path(path))
                .map(|b| String::from_utf8(b.clone()).expect("utf-8"))
        }

        pub fn log(&self) -> Vec<String> {
            self.state.lock().unwrap().log.clone()
        }

        pub fn strip_quotes_on_put(&self) {
            self.state.lock().unwrap().strip_quotes_on_put = true;
        }
    }

    fn handle(mut stream: TcpStream, state: &Arc<Mutex<State>>) {
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            return;
        }
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("").to_string();

        let mut length = 0usize;
        let mut auth = String::new();
        let mut content_type = String::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty() {
                break;
            }
            let (name, value) = line.split_once(':').unwrap_or((&line, ""));
            let value = value.trim().to_string();
            match name.to_ascii_lowercase().as_str() {
                "content-length" => length = value.parse().unwrap_or(0),
                "authorization" => auth = value,
                "content-type" => content_type = value,
                _ => {}
            }
        }
        let mut body = vec![0u8; length];
        let _ = reader.read_exact(&mut body);

        let authorised = auth == format!("Bearer {KEY}");
        let mut state = state.lock().unwrap();
        state
            .log
            .push(format!("{method} {path} {content_type}").trim().to_string());

        let (status, reply): (&str, Vec<u8>) = match (method.as_str(), path.as_str()) {
            ("GET", "/") => (
                "200 OK",
                format!("{{\"status\":\"OK\",\"authenticated\":{authorised}}}").into_bytes(),
            ),
            _ if !authorised => ("401 Unauthorized", b"{\"errorCode\":40101}".to_vec()),
            ("GET", p) if p.starts_with("/vault/") => {
                match state.notes.get(&p["/vault/".len()..]) {
                    Some(bytes) => ("200 OK", bytes.clone()),
                    None => ("404 Not Found", b"{\"errorCode\":40400}".to_vec()),
                }
            }
            ("PUT", p) if p.starts_with("/vault/") => {
                let stored = if state.strip_quotes_on_put {
                    body.iter().copied().filter(|b| *b != b'"').collect()
                } else {
                    body
                };
                state.notes.insert(p["/vault/".len()..].to_string(), stored);
                ("204 No Content", Vec::new())
            }
            _ => ("405 Method Not Allowed", Vec::new()),
        };
        drop(state);

        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            reply.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&reply);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_with_a_space_and_an_umlaut_is_encoded_per_byte() {
        assert_eq!(
            encode_path("Atlas/People/Ida Müßig.md"),
            "Atlas/People/Ida%20M%C3%BC%C3%9Fig.md"
        );
        assert_eq!(
            encode_path("Atlas/People/Ida Muster.md"),
            "Atlas/People/Ida%20Muster.md"
        );
    }

    #[test]
    fn the_separator_survives_and_nothing_else_does() {
        assert_eq!(encode_path("a/b?c#d"), "a/b%3Fc%23d");
    }

    #[test]
    fn a_setting_is_read_past_comments_and_quotes() {
        let text = "# a comment\n# obsidian_api_url = \"http://decoy\"\nobsidian_api_url = \"http://127.0.0.1:27123\"\nvault_root = \"~/x\"\n";
        assert_eq!(
            lookup(text, "obsidian_api_url").as_deref(),
            Some("http://127.0.0.1:27123")
        );
        assert_eq!(lookup(text, "obsidian_api_key_file"), None);
    }

    /// Hermetic: the environment value is a parameter, so an operator who
    /// exports `OBSIDIAN_API_KEY` in their shell does not turn this green test
    /// into a pass for the wrong reason.
    #[test]
    fn a_missing_declaration_names_the_setting_rather_than_guessing_a_path() {
        let err = resolve_key_from(Some("vault_root = \"~/x\"\n"), None).expect_err("must refuse");
        assert!(err.contains("obsidian_api_key_file"), "{err}");
        // The refusal names where to put the key and never a value.
        assert!(err.contains("runtime-secrets"), "{err}");
    }

    #[test]
    fn the_environment_wins_and_a_blank_one_falls_through() {
        assert_eq!(
            resolve_key_from(None, Some("  from-env \n".into())).as_deref(),
            Ok("from-env")
        );
        let err = resolve_key_from(None, Some("   ".into())).expect_err("blank is unset");
        assert!(err.contains("SJEL_PERSONAL_ROOT"), "{err}");
    }

    #[test]
    fn a_key_the_plugin_does_not_accept_is_refused_at_connect() {
        let server = mock::Server::start();
        let err = Obsidian::connect_to(&server.base, "wrong".into())
            .err()
            .expect("must refuse");
        assert!(err.contains("did not accept the key"), "{err}");
        assert!(
            !err.contains("wrong"),
            "the error must not echo the key: {err}"
        );
    }

    #[test]
    fn a_plugin_that_is_not_listening_is_named() {
        // Bind and drop, so the port is known to be closed.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .expect("port")
            .port();
        let err = Obsidian::connect_to(&format!("http://127.0.0.1:{port}"), mock::KEY.into())
            .err()
            .expect("must refuse");
        assert!(err.contains("Obsidian is not running"), "{err}");
    }

    /// The write is a raw-bytes `PUT`, never the frontmatter `PATCH`, and what
    /// is stored is exactly what was sent.
    #[test]
    fn a_write_is_a_raw_put_and_round_trips_byte_for_byte() {
        let server = mock::Server::start();
        let api = Obsidian::connect_to(&server.base, mock::KEY.into()).expect("connect");
        let text = "---\nname: 'Ida'\n# kept\n---\n\nbody \"quoted\"\r\n";
        api.write("Atlas/People/Ida Muster.md", text)
            .expect("write");
        assert_eq!(api.read("Atlas/People/Ida Muster.md").as_deref(), Ok(text));
        let log = server.log();
        assert!(
            log.contains(
                &"PUT /vault/Atlas/People/Ida%20Muster.md application/octet-stream".to_string()
            ),
            "{log:?}"
        );
        assert!(!log.iter().any(|l| l.starts_with("PATCH")), "{log:?}");
    }
}
