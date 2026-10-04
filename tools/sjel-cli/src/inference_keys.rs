//! Bitwarden-backed inference-key gateway and local keychain migration.
//!
//! Secrets are only written to stdout for the API-key subcommand. Child diagnostics stay in
//! zeroizing buffers and are matched, never printed -- with one exception: a failed unlock
//! surfaces a single bounded line, because `bw` reports an unreachable server and a rejected
//! master password identically otherwise, and those two need different answers.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal as _, Read as _, Write as _};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

const EXIT_USAGE: u8 = 2;
const EXIT_LOCKED: u8 = 3;
const EXIT_MISSING: u8 = 4;
const EXIT_UNAVAILABLE: u8 = 6;
const DEFAULT_KEY_TIMEOUT_MS: u64 = 8_000;
const DEFAULT_ADMIN_TIMEOUT_MS: u64 = 30_000;
// Keep 15 seconds of headroom under the extension's 60-second manifest deadline.
const DEFAULT_MANIFEST_BUDGET_MS: u64 = 45_000;
const DEFAULT_KEYCHAIN_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_FOLDER: &str = "Axon";

#[derive(Debug)]
struct Failure {
    message: String,
    code: u8,
}

impl Failure {
    fn new(message: impl Into<String>, code: u8) -> Self {
        Self {
            message: message.into(),
            code,
        }
    }
}

#[derive(Debug)]
struct ChildOutput {
    success: bool,
    stdout: Zeroizing<Vec<u8>>,
    stderr: Zeroizing<Vec<u8>>,
}

fn timeout_ms(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

fn cache_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("PI_BW_CACHE_DIR") {
        return PathBuf::from(path);
    }
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".cache"))
        .join("pi-inference-keys")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn session_file() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".cache"))
        .join("axon/bw-session")
}

fn session_candidates() -> Vec<(String, Zeroizing<String>)> {
    let mut out: Vec<(String, Zeroizing<String>)> = Vec::new();
    if let Ok(raw) = fs::read_to_string(session_file()) {
        let raw = Zeroizing::new(raw);
        let value = Zeroizing::new(raw.trim().to_owned());
        if !value.is_empty() {
            out.push((session_file().display().to_string(), value));
        }
    }
    if let Ok(raw) = std::env::var("BW_SESSION") {
        let raw = Zeroizing::new(raw);
        let value = Zeroizing::new(raw.trim().to_owned());
        if !value.is_empty()
            && !out
                .iter()
                .any(|(_, previous)| previous.as_str() == value.as_str())
        {
            out.push(("BW_SESSION".to_string(), value));
        }
    }
    out
}

/// Spawn one child with bounded time. Both output pipes are drained concurrently to prevent
/// deadlock; stderr stays private and is inspected only for a locked-vault diagnostic.
fn run_child(
    program: &str,
    args: &[String],
    input: Option<&[u8]>,
    timeout: Duration,
) -> Result<ChildOutput, Failure> {
    run_child_with_env(program, args, input, timeout, None)
}

fn run_child_with_env(
    program: &str,
    args: &[String],
    input: Option<&[u8]>,
    timeout: Duration,
    env: Option<(&str, &str)>,
) -> Result<ChildOutput, Failure> {
    let mut command = Command::new(program);
    if let Some((key, value)) = env {
        command.env(key, value);
    }
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = command
        .spawn()
        .map_err(|_| Failure::new(format!("could not start {program}"), EXIT_UNAVAILABLE))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| Failure::new("could not read child output", EXIT_UNAVAILABLE))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| Failure::new("could not read child error output", EXIT_UNAVAILABLE))?;
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Zeroizing::new(Vec::new());
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Zeroizing::new(Vec::new());
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    if let Some(input) = input {
        if let Some(mut stdin) = child.stdin.take() {
            let result = stdin.write_all(input);
            drop(stdin);
            if result.is_err() {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(Failure::new(
                    format!("could not write to {program}"),
                    EXIT_UNAVAILABLE,
                ));
            }
        }
    }
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Failure::new(
                    format!("{program} timed out after {}ms", timeout.as_millis()),
                    EXIT_UNAVAILABLE,
                ));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Failure::new(
                    format!("could not wait for {program}"),
                    EXIT_UNAVAILABLE,
                ));
            }
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| Failure::new("child output reader failed", EXIT_UNAVAILABLE))?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| Failure::new("child error reader failed", EXIT_UNAVAILABLE))?;
    Ok(ChildOutput {
        success: status.success(),
        stdout,
        stderr,
    })
}

fn bw(
    args: &[String],
    session: Option<&str>,
    timeout: Duration,
) -> Result<Zeroizing<String>, Failure> {
    let mut full = args.to_vec();
    full.push("--nointeraction".to_string());
    let bin = std::env::var("PI_BW_BIN").unwrap_or_else(|_| "bw".to_string());
    let started = Instant::now();
    let output = run_child_with_env(
        &bin,
        &full,
        None,
        timeout,
        session.map(|value| ("BW_SESSION", value)),
    )?;
    let text = String::from_utf8(output.stdout.to_vec())
        .map(Zeroizing::new)
        .map_err(|_| Failure::new("Bitwarden returned invalid text", EXIT_UNAVAILABLE))?;
    let trimmed = Zeroizing::new(text.trim().to_string());
    if !output.success {
        if contains_ascii_case_insensitive(&output.stderr, b"locked")
            || contains_ascii_case_insensitive(trimmed.as_bytes(), b"locked")
        {
            return Err(Failure::new("vault is locked", EXIT_LOCKED));
        }
        return Err(Failure::new("Bitwarden request failed", EXIT_MISSING));
    }
    if trimmed.is_empty() {
        if args.first().is_some_and(|arg| arg == "status") {
            return Err(Failure::new(
                "Bitwarden returned no status",
                EXIT_UNAVAILABLE,
            ));
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        if !remaining.is_zero()
            && read_status(session, remaining)
                .is_ok_and(|status| status.get("status").and_then(Value::as_str) == Some("locked"))
        {
            return Err(Failure::new("vault is locked", EXIT_LOCKED));
        }
        return Err(Failure::new("Bitwarden returned nothing", EXIT_MISSING));
    }
    Ok(trimmed)
}

fn contains_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

fn read_status(session: Option<&str>, timeout: Duration) -> Result<Value, Failure> {
    let raw = bw(&["status".to_string()], session, timeout)?;
    serde_json::from_str(&raw)
        .map_err(|_| Failure::new("Bitwarden returned invalid status", EXIT_UNAVAILABLE))
}

fn usable_session_with_budget(
    timeout: Duration,
    budget: &mut RequestBudget,
) -> Result<Zeroizing<String>, Failure> {
    let candidates = session_candidates();
    if candidates.is_empty() {
        return Err(Failure::new(
            "no Bitwarden session; run `bwu`",
            EXIT_MISSING,
        ));
    }
    let mut last = Failure::new("no usable Bitwarden session", EXIT_MISSING);
    for (_, candidate) in candidates {
        match budget
            .remaining()
            .and_then(|remaining| read_status(Some(&candidate), remaining.min(timeout)))
        {
            Ok(status) if status.get("status").and_then(Value::as_str) == Some("unlocked") => {
                return Ok(candidate)
            }
            Ok(status) if status.get("status").and_then(Value::as_str) == Some("locked") => {
                last = Failure::new("vault is locked; run `bwu`", EXIT_LOCKED);
            }
            Ok(_) => last = Failure::new("Bitwarden session is not usable", EXIT_MISSING),
            Err(error) => last = error,
        }
    }
    Err(last)
}

fn usable_session(timeout: Duration) -> Result<Zeroizing<String>, Failure> {
    let candidates = session_candidates();
    if candidates.is_empty() {
        return Err(Failure::new(
            "no Bitwarden session; run `bwu`",
            EXIT_MISSING,
        ));
    }
    let mut last = Failure::new("no usable Bitwarden session", EXIT_MISSING);
    for (_, candidate) in candidates {
        match read_status(Some(&candidate), timeout) {
            Ok(status) if status.get("status").and_then(Value::as_str) == Some("unlocked") => {
                return Ok(candidate)
            }
            Ok(status) if status.get("status").and_then(Value::as_str) == Some("locked") => {
                last = Failure::new("vault is locked; run `bwu`", EXIT_LOCKED);
            }
            Ok(_) => last = Failure::new("Bitwarden session is not usable", EXIT_MISSING),
            Err(error) => last = error,
        }
    }
    Err(last)
}

fn keychain_service(slug: &str) -> String {
    format!("inference-{slug}-api-key")
}

fn keychain_read(slug: &str, timeout: Duration) -> Option<Zeroizing<Vec<u8>>> {
    if std::env::var("PI_KEYCHAIN").is_ok_and(|v| v == "0") {
        return None;
    }
    let bin = std::env::var("PI_KEYCHAIN_BIN").unwrap_or_else(|_| "/usr/bin/security".to_string());
    let args = vec![
        "find-generic-password".into(),
        "-w".into(),
        "-s".into(),
        keychain_service(slug),
    ];
    let output = run_child(&bin, &args, None, timeout).ok()?;
    if !output.success {
        return None;
    }
    let mut bytes = output.stdout;
    while bytes.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
        bytes.pop();
    }
    if bytes.is_empty() || bytes.iter().any(u8::is_ascii_whitespace) {
        None
    } else {
        Some(bytes)
    }
}

#[derive(Clone, Deserialize)]
struct VaultItem {
    id: String,
    name: String,
    #[serde(rename = "folderId")]
    folder_id: Option<String>,
    #[serde(rename = "revisionDate")]
    revision_date: Option<String>,
}

fn list_items(
    session: &str,
    timeout: Duration,
    search: Option<&str>,
) -> Result<Vec<VaultItem>, Failure> {
    let mut args = vec!["list".into(), "items".into()];
    if let Some(search) = search {
        args.extend(["--search".into(), search.into()]);
    }
    let raw = bw(&args, Some(session), timeout)?;
    serde_json::from_str(&raw)
        .map_err(|_| Failure::new("Bitwarden returned invalid item data", EXIT_UNAVAILABLE))
}

fn folder_id(session: &str, timeout: Duration) -> Option<String> {
    #[derive(Deserialize)]
    struct Folder {
        id: String,
        name: String,
    }
    let raw = bw(&["list".into(), "folders".into()], Some(session), timeout).ok()?;
    let folders: Vec<Folder> = serde_json::from_str(&raw).ok()?;
    let wanted = std::env::var("PI_BW_FOLDER").unwrap_or_else(|_| DEFAULT_FOLDER.into());
    folders.into_iter().find(|f| f.name == wanted).map(|f| f.id)
}

fn choose_item<'a>(items: &'a [VaultItem], folder: Option<&str>) -> Option<&'a VaultItem> {
    let in_folder: Vec<_> = items
        .iter()
        .filter(|item| folder.is_some() && item.folder_id.as_deref() == folder)
        .collect();
    let pool: Vec<_> = if in_folder.is_empty() {
        items.iter().collect()
    } else {
        in_folder
    };
    pool.into_iter()
        .max_by_key(|item| item.revision_date.as_deref().unwrap_or(""))
}

fn parse_spec(spec: &str) -> (&str, &str) {
    match spec.split_once('=') {
        Some((slug, item)) => (slug, item),
        None => (spec, ""),
    }
}

fn validate_spec(spec: &str) -> Result<(), Failure> {
    let (slug, _) = parse_spec(spec);
    if slug.is_empty()
        || slug.len() > 64
        || !slug
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(Failure::new("invalid provider slug", EXIT_USAGE));
    }
    Ok(())
}

fn item_name(spec: &str) -> (String, String) {
    let (slug, item) = parse_spec(spec);
    (
        slug.to_string(),
        if item.is_empty() {
            format!("inference-{slug}-api-key")
        } else {
            item.to_string()
        },
    )
}

fn resolve_item(
    name: &str,
    session: &str,
    budget: &mut RequestBudget,
) -> Result<(VaultItem, usize), Failure> {
    let items = list_items(session, budget.remaining()?, Some(name))?;
    let exact: Vec<_> = items.into_iter().filter(|item| item.name == name).collect();
    if exact.is_empty() {
        return Err(Failure::new(
            format!("no vault item named {name:?}"),
            EXIT_MISSING,
        ));
    }
    let folder = budget
        .remaining()
        .ok()
        .and_then(|remaining| folder_id(session, remaining));
    let chosen = choose_item(&exact, folder.as_deref())
        .ok_or_else(|| Failure::new("no matching vault item", EXIT_MISSING))?;
    Ok((
        VaultItem {
            id: chosen.id.clone(),
            name: chosen.name.clone(),
            folder_id: chosen.folder_id.clone(),
            revision_date: chosen.revision_date.clone(),
        },
        exact.len(),
    ))
}

struct RequestBudget {
    deadline: Instant,
}

impl RequestBudget {
    fn new(limit: Duration) -> Self {
        Self {
            deadline: Instant::now() + limit,
        }
    }

    fn remaining(&self) -> Result<Duration, Failure> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            Err(Failure::new(
                "Bitwarden operation timed out",
                EXIT_UNAVAILABLE,
            ))
        } else {
            Ok(remaining)
        }
    }
}

fn fetch_key(name: &str, session: &str, timeout: Duration) -> Result<Zeroizing<Vec<u8>>, Failure> {
    let mut budget = RequestBudget::new(timeout);
    fetch_key_with_budget(name, session, &mut budget)
}

fn fetch_key_with_budget(
    name: &str,
    session: &str,
    budget: &mut RequestBudget,
) -> Result<Zeroizing<Vec<u8>>, Failure> {
    let first = bw(
        &["get".into(), "notes".into(), name.into()],
        Some(session),
        budget.remaining()?,
    );
    let text = match first {
        Ok(value) => value,
        Err(error) if error.code == EXIT_LOCKED || error.code == EXIT_UNAVAILABLE => {
            return Err(error)
        }
        Err(_) => {
            let (item, _) = resolve_item(name, session, budget)?;
            bw(
                &["get".into(), "notes".into(), item.id],
                Some(session),
                budget.remaining()?,
            )?
        }
    };
    validate_key(name, text)
}

fn validate_key(name: &str, text: Zeroizing<String>) -> Result<Zeroizing<Vec<u8>>, Failure> {
    if text.is_empty() {
        return Err(Failure::new(
            format!("vault item {name:?} is empty"),
            EXIT_MISSING,
        ));
    }
    if text.chars().any(char::is_whitespace) {
        return Err(Failure::new(
            format!("vault item {name:?} contains whitespace; expected one API key"),
            EXIT_MISSING,
        ));
    }
    Ok(Zeroizing::new(text.as_bytes().to_vec()))
}

fn ttl() -> Duration {
    Duration::from_secs(
        std::env::var("PI_BW_TTL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60),
    )
}

fn cache_path(slug: &str) -> PathBuf {
    cache_dir().join(format!("{slug}.json"))
}

fn cached_key(slug: &str) -> Option<Zeroizing<Vec<u8>>> {
    if ttl().is_zero() {
        return None;
    }
    #[derive(Deserialize)]
    struct Entry {
        key: String,
        fetched_at: u64,
    }
    let body = Zeroizing::new(fs::read_to_string(cache_path(slug)).ok()?);
    let entry: Entry = serde_json::from_str(&body).ok()?;
    let key = Zeroizing::new(entry.key);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if now.saturating_sub(entry.fetched_at) >= ttl().as_secs() {
        return None;
    }
    Some(Zeroizing::new(key.as_bytes().to_vec()))
}

fn write_cache(slug: &str, key: &[u8]) {
    if ttl().is_zero() {
        return;
    }
    let Ok(key) = std::str::from_utf8(key) else {
        return;
    };
    if fs::create_dir_all(cache_dir()).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::set_permissions(cache_dir(), fs::Permissions::from_mode(0o700)).is_err() {
            return;
        }
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    #[derive(Serialize)]
    struct CacheEntry<'a> {
        key: &'a str,
        fetched_at: u64,
    }
    let mut body = Zeroizing::new(Vec::new());
    if serde_json::to_writer(
        &mut *body,
        &CacheEntry {
            key,
            fetched_at: now,
        },
    )
    .is_err()
    {
        return;
    }
    let path = cache_path(slug);
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let Ok(mut file) = options.open(&temp) {
        if file.write_all(&body).is_ok() && file.sync_all().is_ok() {
            let _ = fs::rename(temp, path);
        } else {
            let _ = fs::remove_file(temp);
        }
    }
}

fn forget(slugs: &[String]) {
    if let Ok(entries) = fs::read_dir(cache_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".json") {
                continue;
            }
            if slugs.is_empty() || slugs.iter().any(|slug| name == format!("{slug}.json")) {
                let _ = fs::remove_file(path);
            }
        }
    }
}

fn print_json(value: &Value) {
    println!("{value}");
}

fn note_error(slug: &str, message: &str) {
    if slug.is_empty() || fs::create_dir_all(cache_dir()).is_err() {
        return;
    }
    let path = cache_dir().join("last-error.json");
    let body = json!({"slug":slug,"message":message,"at":timestamp()}).to_string();
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let Ok(mut file) = options.open(path) {
        let _ = file.write_all(body.as_bytes());
    }
}

fn manifest(specs: &[String]) -> Result<(), Failure> {
    for spec in specs {
        validate_spec(spec)?;
    }
    let timeout = Duration::from_millis(timeout_ms(
        "PI_BW_ADMIN_TIMEOUT_MS",
        DEFAULT_ADMIN_TIMEOUT_MS,
    ));
    let mut budget = RequestBudget::new(Duration::from_millis(DEFAULT_MANIFEST_BUDGET_MS));
    let mut result = json!({"checkedAt": timestamp(), "vault":"unavailable", "folder":std::env::var("PI_BW_FOLDER").unwrap_or_else(|_| DEFAULT_FOLDER.into()), "synced":false, "items":{}});
    let session = match usable_session_with_budget(timeout, &mut budget) {
        Ok(value) => value,
        Err(error) => {
            result["vault"] = json!(if error.code == EXIT_LOCKED {
                "locked"
            } else {
                "unavailable"
            });
            result["error"] = json!(error.message);
            print_json(&result);
            return Err(error);
        }
    };
    if budget
        .remaining()
        .and_then(|remaining| bw(&["sync".into()], Some(&session), remaining.min(timeout)))
        .is_ok()
    {
        result["synced"] = json!(true);
    }
    let status = match budget
        .remaining()
        .and_then(|remaining| read_status(Some(&session), remaining.min(timeout)))
    {
        Ok(status) => status,
        Err(error) => {
            result["vault"] = json!(if error.code == EXIT_LOCKED {
                "locked"
            } else {
                "unavailable"
            });
            result["error"] = json!(error.message);
            print_json(&result);
            return Err(error);
        }
    };
    result["vault"] = status.get("status").cloned().unwrap_or(json!("unknown"));
    let items = match budget
        .remaining()
        .and_then(|remaining| list_items(&session, remaining.min(timeout), None))
    {
        Ok(items) => items,
        Err(error) => {
            result["vault"] = json!("unavailable");
            result["error"] = json!(error.message);
            print_json(&result);
            return Err(error);
        }
    };
    let folder = budget
        .remaining()
        .ok()
        .and_then(|remaining| folder_id(&session, remaining.min(timeout)));
    for spec in specs {
        let (slug, name) = item_name(spec);
        let matches: Vec<_> = items
            .iter()
            .filter(|item| item.name == name)
            .cloned()
            .collect();
        let chosen = choose_item(&matches, folder.as_deref());
        let mut row =
            json!({"item":name, "exists":!matches.is_empty(), "ambiguous":matches.len() > 1});
        if let Some(item) = chosen {
            row["id"] = json!(item.id);
        }
        result["items"][slug.as_str()] = row;
    }
    print_json(&result);
    Ok(())
}

fn timestamp() -> String {
    crate::time::now_iso()
}

fn status() {
    let timeout = Duration::from_millis(timeout_ms(
        "PI_BW_ADMIN_TIMEOUT_MS",
        DEFAULT_ADMIN_TIMEOUT_MS,
    ));
    let mut budget = RequestBudget::new(Duration::from_secs(8));
    let candidates = session_candidates();
    let session = usable_session_with_budget(timeout, &mut budget).ok();
    let vault = session
        .as_deref()
        .and_then(|session| {
            budget
                .remaining()
                .ok()
                .and_then(|remaining| read_status(Some(session), remaining.min(timeout)).ok())
        })
        .or_else(|| {
            budget
                .remaining()
                .ok()
                .and_then(|remaining| read_status(None, remaining.min(timeout)).ok())
        })
        .unwrap_or_else(|| json!({"status":"unavailable","locked":false,"unauthenticated":false}));
    let names: Vec<String> = candidates.into_iter().map(|(name, _)| name).collect();
    print_json(
        &json!({"vault":vault,"sessionCandidates":names,"sessionUsable":session.is_some(),"ttlSeconds":ttl().as_secs(),"cacheDir":cache_dir()}),
    );
}

fn current_account() -> Result<String, Failure> {
    let status = read_status(
        None,
        Duration::from_millis(timeout_ms(
            "PI_BW_ADMIN_TIMEOUT_MS",
            DEFAULT_ADMIN_TIMEOUT_MS,
        )),
    )?;
    if status.get("status").and_then(Value::as_str) == Some("unauthenticated") {
        return Err(Failure::new(
            "Bitwarden is not logged in; run `bw login` first",
            EXIT_MISSING,
        ));
    }
    status
        .get("userEmail")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            Failure::new(
                "Bitwarden status did not identify an account",
                EXIT_UNAVAILABLE,
            )
        })
}

fn secret_backend() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if command_exists("secret-tool") {
        "linux"
    } else {
        "none"
    }
}

fn command_exists(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

fn secret_store_read(account: &str) -> Option<Zeroizing<Vec<u8>>> {
    let (program, args) = match secret_backend() {
        "macos" => (
            "security",
            vec![
                "find-generic-password".to_string(),
                "-w".into(),
                "-s".into(),
                "axon-bw-master".into(),
                "-a".into(),
                account.into(),
            ],
        ),
        "linux" => (
            "secret-tool",
            vec![
                "lookup".into(),
                "service".into(),
                "axon-bw-master".into(),
                "account".into(),
                account.into(),
            ],
        ),
        _ => return None,
    };
    let output = run_child(program, &args, None, Duration::from_secs(30)).ok()?;
    if !output.success {
        return None;
    }
    let mut secret = output.stdout;
    while secret
        .last()
        .is_some_and(|byte| *byte == b'\n' || *byte == b'\r')
    {
        secret.pop();
    }
    if secret.is_empty() {
        None
    } else {
        Some(secret)
    }
}

fn secret_store_has(account: &str) -> bool {
    match secret_backend() {
        "macos" => run_child(
            "security",
            &[
                "find-generic-password".into(),
                "-s".into(),
                "axon-bw-master".into(),
                "-a".into(),
                account.into(),
            ],
            None,
            Duration::from_secs(30),
        )
        .is_ok_and(|out| out.success),
        "linux" => secret_store_read(account).is_some(),
        _ => false,
    }
}

fn encode_hex(bytes: &[u8]) -> Zeroizing<String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = Zeroizing::new(String::with_capacity(bytes.len() * 2));
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 15) as usize] as char);
    }
    output
}

fn security_command(
    service: &str,
    account: &str,
    secret: &[u8],
    label: Option<&str>,
    protect: bool,
) -> Zeroizing<String> {
    let mut command = Zeroizing::new(format!(
        "add-generic-password -a {account} -s {service} -U "
    ));
    if protect {
        command.push_str("-T \"\" ");
    }
    if let Some(label) = label {
        command.push_str(&format!("-l \"{label}\" "));
    }
    command.push_str("-X ");
    command.push_str(encode_hex(secret).as_str());
    command.push('\n');
    command
}

fn safe_command_atom(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'@' | b'+')
        })
}

/// Feed the secret as hex through `security -i`'s stdin, never through process arguments.
fn security_write(
    service: &str,
    account: &str,
    secret: &[u8],
    label: Option<&str>,
    protect: bool,
) -> Result<(), Failure> {
    if !safe_command_atom(service) || !safe_command_atom(account) {
        return Err(Failure::new(
            "invalid Keychain service or account",
            EXIT_USAGE,
        ));
    }
    let command = Zeroizing::new(security_command(service, account, secret, label, protect));
    let bin = std::env::var("PI_KEYCHAIN_BIN").unwrap_or_else(|_| "security".into());
    let output = run_child(
        &bin,
        &["-i".into()],
        Some(command.as_bytes()),
        Duration::from_secs(30),
    )?;
    if output.success {
        Ok(())
    } else {
        Err(Failure::new(
            "macOS Keychain write failed",
            EXIT_UNAVAILABLE,
        ))
    }
}

fn secret_store_delete(account: &str) -> bool {
    let (program, args) = match secret_backend() {
        "macos" => (
            "security",
            vec![
                "delete-generic-password".into(),
                "-s".into(),
                "axon-bw-master".into(),
                "-a".into(),
                account.into(),
            ],
        ),
        "linux" => (
            "secret-tool",
            vec![
                "clear".into(),
                "service".into(),
                "axon-bw-master".into(),
                "account".into(),
                account.into(),
            ],
        ),
        _ => return false,
    };
    run_child(program, &args, None, Duration::from_secs(30)).is_ok_and(|out| out.success)
}

fn store_session(session: &[u8]) -> Result<(), Failure> {
    let path = session_file();
    let parent = path
        .parent()
        .ok_or_else(|| Failure::new("invalid session-cache path", EXIT_UNAVAILABLE))?;
    fs::create_dir_all(parent)
        .map_err(|_| Failure::new("could not create session-cache directory", EXIT_UNAVAILABLE))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(|_| {
            Failure::new("could not secure session-cache directory", EXIT_UNAVAILABLE)
        })?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|_| Failure::new("could not write session cache", EXIT_UNAVAILABLE))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|_| Failure::new("could not secure session cache", EXIT_UNAVAILABLE))?;
    }
    file.write_all(session)
        .and_then(|_| file.write_all(b"\n"))
        .map_err(|_| Failure::new("could not write session cache", EXIT_UNAVAILABLE))
}

/// Why an unlock failed, in `bw`'s own words.
///
/// A self-hosted Bitwarden needs the server before it can accept the master password, so a
/// disconnected VPN and a typo look the same from here. The transport failure is checked first
/// and named with the URL `bw` printed; anything else falls back to the last non-empty stderr
/// line, which is where a rejected password puts its message.
fn unlock_diagnostic(stderr: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(stderr);
    if text.contains("FetchError") || text.contains("Unable to fetch ServerConfig") {
        let url = text
            .split_whitespace()
            .find(|word| word.starts_with("http://") || word.starts_with("https://"))
            .map(|url| {
                url.trim_end_matches(['.', ',', ':', '"', '\'', ')'])
                    .to_string()
            });
        return Some(match url {
            Some(url) => format!(
                "cannot reach the Bitwarden server at {url} -- check the network (Tailscale), then run `bwu`"
            ),
            None => "cannot reach the Bitwarden server -- check the network (Tailscale), then run `bwu`"
                .to_string(),
        });
    }
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| format!("Bitwarden unlock failed: {line}"))
}

fn unlock_failure(stderr: &[u8]) -> Failure {
    match unlock_diagnostic(stderr) {
        Some(message) => Failure::new(message, EXIT_UNAVAILABLE),
        None => Failure::new("Bitwarden unlock failed", EXIT_LOCKED),
    }
}

fn bw_unlock(input: Option<&[u8]>, interactive: bool) -> Result<Zeroizing<Vec<u8>>, Failure> {
    let bin = std::env::var("PI_BW_BIN").unwrap_or_else(|_| "bw".into());
    let mut command = Command::new(bin);
    command.args(if interactive {
        vec!["unlock", "--raw"]
    } else {
        vec!["unlock", "--passwordfile", "/dev/stdin", "--raw"]
    });
    if interactive {
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let output = command
            .output()
            .map_err(|_| Failure::new("could not start Bitwarden unlock", EXIT_UNAVAILABLE))?;
        if !output.status.success() {
            return Err(unlock_failure(&output.stderr));
        }
        let mut session = Zeroizing::new(output.stdout);
        while session.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
            session.pop();
        }
        if session.is_empty() {
            return Err(Failure::new(
                "Bitwarden unlock returned no session",
                EXIT_LOCKED,
            ));
        }
        return Ok(session);
    }
    run_child(
        &command.get_program().to_string_lossy(),
        &[
            "unlock".into(),
            "--passwordfile".into(),
            "/dev/stdin".into(),
            "--raw".into(),
        ],
        input,
        Duration::from_secs(30),
    )
    .and_then(|out| {
        if !out.success {
            return Err(unlock_failure(&out.stderr));
        }
        let mut session = out.stdout;
        while session.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
            session.pop();
        }
        if session.is_empty() {
            Err(Failure::new(
                "Bitwarden unlock returned no session",
                EXIT_LOCKED,
            ))
        } else {
            Ok(session)
        }
    })
}

fn unlock_session() -> Result<Zeroizing<Vec<u8>>, Failure> {
    let account = current_account()?;
    let session = if secret_backend() == "none" {
        if !io::stdin().is_terminal() {
            return Err(Failure::new(
                "no OS secret store and no terminal; run `bw unlock` manually",
                EXIT_UNAVAILABLE,
            ));
        }
        bw_unlock(None, true)?
    } else {
        let password = secret_store_read(&account).ok_or_else(|| {
            Failure::new(
                format!("nothing enrolled for {account}; run `tools/bw-unlock --enroll` first"),
                EXIT_MISSING,
            )
        })?;
        bw_unlock(Some(&password), false)?
    };
    store_session(&session)?;
    Ok(session)
}

fn unlock() -> Result<(), Failure> {
    let session = unlock_session()?;
    io::stdout()
        .write_all(&session)
        .and_then(|_| io::stdout().write_all(b"\n"))
        .map_err(|_| Failure::new("could not write session to stdout", EXIT_UNAVAILABLE))
}

fn unlock_status() -> Result<(), Failure> {
    let session = unlock_session()?;
    let session = std::str::from_utf8(&session)
        .map_err(|_| Failure::new("invalid Bitwarden session", EXIT_UNAVAILABLE))?;
    let status = read_status(Some(session), Duration::from_secs(30))?;
    print_json(
        &json!({"status": status.get("status").and_then(Value::as_str).unwrap_or("unknown")}),
    );
    Ok(())
}

fn enrollment(account: &str) -> Result<(), Failure> {
    if secret_backend() == "none" {
        return Err(Failure::new(
            "no OS secret store is available; use `bwu` to unlock interactively",
            EXIT_UNAVAILABLE,
        ));
    }
    if !io::stdin().is_terminal() {
        return Err(Failure::new("--enroll needs a terminal", EXIT_UNAVAILABLE));
    }
    if secret_backend() == "macos" {
        let status = Command::new("security")
            .args([
                "add-generic-password",
                "-a",
                account,
                "-s",
                "axon-bw-master",
                "-T",
                "",
                "-U",
                "-l",
                "Axon - Bitwarden master password",
                "-w",
            ])
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|_| Failure::new("could not start Keychain enrollment", EXIT_UNAVAILABLE))?;
        if !status.success() {
            return Err(Failure::new("Keychain enrollment failed", EXIT_UNAVAILABLE));
        }
    } else {
        let status = Command::new("secret-tool")
            .args([
                "store",
                "--label=Axon - Bitwarden master password",
                "service",
                "axon-bw-master",
                "account",
                account,
            ])
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|_| {
                Failure::new(
                    "could not start Secret Service enrollment",
                    EXIT_UNAVAILABLE,
                )
            })?;
        if !status.success() {
            return Err(Failure::new(
                "Secret Service enrollment failed",
                EXIT_UNAVAILABLE,
            ));
        }
    }
    match unlock_session() {
        Ok(_session) => {
            eprintln!("enrolled and verified; new shells reuse the cached session");
            Ok(())
        }
        Err(error) => {
            let _ = secret_store_delete(account);
            Err(Failure::new(
                format!("stored password was rejected ({})", error.message),
                error.code,
            ))
        }
    }
}

fn unlock_command(args: &[String]) -> Result<(), Failure> {
    match args.first().map(String::as_str) {
        None => unlock(),
        Some("--status") => {
            let account = current_account()?;
            let timeout = Duration::from_secs(30);
            let session = usable_session(timeout).ok();
            let vault = session
                .as_deref()
                .and_then(|session| read_status(Some(session), timeout).ok())
                .or_else(|| read_status(None, timeout).ok())
                .unwrap_or_else(|| json!({"status":"unavailable"}));
            println!("backend:  {}", secret_backend());
            println!("account:  {account}");
            println!(
                "enrolled: {}",
                if secret_backend() == "none" {
                    "n/a (no secret store)"
                } else if secret_store_has(&account) {
                    "yes"
                } else {
                    "no"
                }
            );
            println!(
                "vault:    {}",
                vault
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unavailable")
            );
            println!(
                "session:  {}",
                if session_file().exists() {
                    format!("cached ({})", session_file().display())
                } else {
                    "none".into()
                }
            );
            Ok(())
        }
        Some("--enroll") => enrollment(&current_account()?),
        Some("--help" | "-h") => {
            eprint!("tools/bw-unlock — unlock the Bitwarden CLI without retyping the master password.\n\n  tools/bw-unlock            unlock (the OS asks to authorize), print the session key\n  tools/bw-unlock --enroll   store the master password in the OS secret store (asks once)\n  tools/bw-unlock --status   backend, what is enrolled, is the vault locked, session cached?\n  tools/bw-unlock --forget   remove the stored password, clear the cache, lock the vault\n\nSecret store per OS: macOS login keychain, Linux Secret Service (secret-tool). Where none\nexists (e.g. WSL) unlock still works — it prompts for the master password and caches the\nsession so new shells reuse it; only --enroll is unavailable.\n\nIn a shell:  export BW_SESSION=\"$(tools/bw-unlock)\"   — or just `bwu`, from\ncapabilities/shell/init.{{zsh,bash}}, which also picks up a cached session in every new shell.\n");
            Ok(())
        }
        Some("--forget") => {
            let account = current_account()?;
            let _ = secret_store_delete(&account);
            let _ = fs::remove_file(session_file());
            let bin = std::env::var("PI_BW_BIN").unwrap_or_else(|_| "bw".into());
            let _ = run_child(
                &bin,
                &["lock".into(), "--nointeraction".into()],
                None,
                Duration::from_secs(30),
            );
            eprintln!("session cache cleared and vault locked");
            Ok(())
        }
        Some(_) => Err(Failure::new(
            "usage: tools/bw-unlock [--status|--enroll|--forget]",
            EXIT_USAGE,
        )),
    }
}

fn migrate(specs: &[String]) -> Result<(), Failure> {
    if matches!(specs.first().map(String::as_str), Some("-h" | "--help")) {
        println!("usage: keychain-migrate.sh [slug[=Vault Item Name] ...]");
        return Ok(());
    }
    if !cfg!(target_os = "macos") {
        return Err(Failure::new(
            "keychain migration is supported only on macOS",
            EXIT_UNAVAILABLE,
        ));
    }
    let timeout = Duration::from_millis(timeout_ms(
        "PI_BW_ADMIN_TIMEOUT_MS",
        DEFAULT_ADMIN_TIMEOUT_MS,
    ));
    let session = usable_session(timeout)
        .map_err(|_| Failure::new("no usable vault session; run `bwu` first", EXIT_LOCKED))?;
    let specs = if specs.is_empty() {
        vec![
            "groq".into(),
            "nvidia-nim".into(),
            "gemini".into(),
            "cohere".into(),
            "ollama-cloud".into(),
            "deepseek=Deepseek API Key".into(),
            "openrouter=Openrouter API Key".into(),
            "kimi=Kimi K2 API Token".into(),
        ]
    } else {
        specs.to_vec()
    };
    for spec in &specs {
        validate_spec(spec)?;
    }
    let account = std::env::var("PI_KEYCHAIN_ACCOUNT")
        .unwrap_or_else(|_| std::env::var("USER").unwrap_or_default());
    let mut moved = 0;
    let mut skipped = 0;
    let mut failed = 0;
    for spec in specs {
        let (slug, name) = item_name(&spec);
        let service = keychain_service(&slug);
        if run_child(
            "security",
            &[
                "find-generic-password".into(),
                "-s".into(),
                service.clone(),
                "-w".into(),
            ],
            None,
            Duration::from_secs(10),
        )
        .is_ok_and(|out| out.success)
        {
            println!("  = {slug} (already in the keychain)");
            skipped += 1;
            continue;
        }
        let key = match fetch_key(&name, &session, timeout) {
            Ok(key) => key,
            Err(_) => {
                println!("  ! {slug} FAILED — still served from the vault, nothing changed");
                failed += 1;
                continue;
            }
        };
        if security_write(&service, &account, &key, None, false).is_err() {
            println!("  ! {slug} FAILED — still served from the vault, nothing changed");
            failed += 1;
            continue;
        }
        let verified = run_child(
            "security",
            &[
                "find-generic-password".into(),
                "-s".into(),
                service.clone(),
                "-w".into(),
            ],
            None,
            Duration::from_secs(10),
        )
        .ok()
        .filter(|out| out.success)
        .map(|out| {
            let mut bytes = out.stdout;
            while bytes.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
                bytes.pop();
            }
            bytes
        });
        if verified
            .as_ref()
            .is_some_and(|value| value.as_slice() == key.as_slice())
        {
            println!("  + {slug} -> {service} (verified against the vault)");
            moved += 1;
            forget(std::slice::from_ref(&slug));
        } else {
            println!("  ! {slug} MISMATCH between keychain and vault — left in place, investigate");
            failed += 1;
        }
    }
    println!("\n  moved:   {moved}\n  skipped: {skipped} (already present)\n  failed:  {failed}");
    Ok(())
}

pub fn run(args: &[String]) -> ExitCode {
    let (mode, rest) = match args.split_first() {
        Some((mode, rest)) => (mode.as_str(), rest),
        None => {
            eprintln!("inference-keys: expected a command");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let result = match mode {
        "-h" | "--help" | "help" => {
            println!("usage: tools/inference-keys {{key <spec>|check <spec>|manifest <spec...>|status|forget [slug...]|unlock [--status|--enroll|--forget]|migrate [spec...]}}");
            Ok(())
        }
        "manifest" => manifest(rest),
        "unlock" => unlock_command(rest),
        "unlock-status" => unlock_status(),
        "migrate" => migrate(rest),
        "status" => { status(); Ok(()) },
        "forget" => { forget(rest); Ok(()) },
        "check" => check(rest.first().map(String::as_str).unwrap_or("")),
        "key" => key(rest.first().map(String::as_str).unwrap_or("")),
        _ => Err(Failure::new("usage: inference-keys {key <slug[=Item]>|check <slug>|manifest <spec...>|status|forget [slug...]}", EXIT_USAGE)),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if matches!(mode, "key" | "check") {
                if let Some(spec) = rest.first() {
                    note_error(spec.split('=').next().unwrap_or(""), &error.message);
                }
            }
            eprintln!("inference-keys: {}", error.message);
            ExitCode::from(error.code)
        }
    }
}

fn key(spec: &str) -> Result<(), Failure> {
    validate_spec(spec)?;
    let (slug, name) = item_name(spec);
    let timeout = Duration::from_millis(timeout_ms("PI_BW_TIMEOUT_MS", DEFAULT_KEY_TIMEOUT_MS))
        .min(Duration::from_secs(8));
    let mut budget = RequestBudget::new(timeout);
    let keychain_timeout = Duration::from_millis(timeout_ms(
        "PI_KEYCHAIN_TIMEOUT_MS",
        DEFAULT_KEYCHAIN_TIMEOUT_MS,
    ))
    .min(budget.remaining()?);
    if let Some(key) = keychain_read(&slug, keychain_timeout).or_else(|| cached_key(&slug)) {
        io::stdout()
            .write_all(&key)
            .map_err(|_| Failure::new("could not write API key", EXIT_UNAVAILABLE))?;
        println!();
        return Ok(());
    }
    let candidates = session_candidates();
    if candidates.is_empty() {
        return Err(Failure::new(
            "no Bitwarden session; run `bwu`",
            EXIT_MISSING,
        ));
    }
    let mut failure = Failure::new("no usable Bitwarden session", EXIT_MISSING);
    for (_, session) in candidates {
        match fetch_key_with_budget(&name, &session, &mut budget) {
            Ok(key) => {
                write_cache(&slug, &key);
                io::stdout()
                    .write_all(&key)
                    .map_err(|_| Failure::new("could not write API key", EXIT_UNAVAILABLE))?;
                println!();
                return Ok(());
            }
            Err(error) if matches!(error.code, EXIT_LOCKED | EXIT_UNAVAILABLE) => failure = error,
            Err(error) => return Err(error),
        }
    }
    Err(failure)
}

fn check(spec: &str) -> Result<(), Failure> {
    validate_spec(spec)?;
    let (slug, name) = item_name(spec);
    if keychain_read(
        &slug,
        Duration::from_millis(timeout_ms(
            "PI_KEYCHAIN_TIMEOUT_MS",
            DEFAULT_KEYCHAIN_TIMEOUT_MS,
        )),
    )
    .is_some()
    {
        print_json(&json!({"slug":slug,"source":"keychain","service":keychain_service(&slug)}));
        return Ok(());
    }
    let session = usable_session(Duration::from_millis(timeout_ms(
        "PI_BW_TIMEOUT_MS",
        DEFAULT_KEY_TIMEOUT_MS,
    )))?;
    let _ = fetch_key(
        &name,
        &session,
        Duration::from_millis(timeout_ms("PI_BW_TIMEOUT_MS", DEFAULT_KEY_TIMEOUT_MS)),
    )?;
    print_json(&json!({"slug":slug,"source":"vault","item":name}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_convention_and_legacy_names() {
        assert_eq!(
            item_name("groq"),
            ("groq".into(), "inference-groq-api-key".into())
        );
        assert_eq!(
            item_name("deepseek=Deepseek API Key"),
            ("deepseek".into(), "Deepseek API Key".into())
        );
    }

    #[test]
    fn provider_slug_rejects_cache_path_traversal() {
        assert!(validate_spec("openrouter=Openrouter API Key").is_ok());
        assert!(validate_spec("../outside").is_err());
        assert!(validate_spec("nested/provider").is_err());
        assert!(validate_spec("").is_err());
    }

    #[test]
    fn item_selection_prefers_folder_then_latest_revision() {
        let items = vec![
            VaultItem {
                id: "old".into(),
                name: "key".into(),
                folder_id: Some("other".into()),
                revision_date: Some("2026-01".into()),
            },
            VaultItem {
                id: "new".into(),
                name: "key".into(),
                folder_id: Some("wanted".into()),
                revision_date: Some("2025-01".into()),
            },
            VaultItem {
                id: "latest".into(),
                name: "key".into(),
                folder_id: Some("other".into()),
                revision_date: Some("2026-09".into()),
            },
        ];
        assert_eq!(choose_item(&items, Some("wanted")).unwrap().id, "new");
        assert_eq!(choose_item(&items, None).unwrap().id, "latest");
    }

    #[test]
    fn key_validation_rejects_empty_and_whitespace() {
        assert!(validate_key("x", Zeroizing::new(String::new())).is_err());
        assert!(validate_key("x", Zeroizing::new("two words".to_string())).is_err());
        assert!(validate_key("x", Zeroizing::new("one-token".to_string())).is_ok());
    }

    #[test]
    fn keychain_write_keeps_secret_out_of_argv_and_command_plaintext() {
        let command = security_command("service", "account", b"dummy-token", None, false);
        assert!(command.contains("-X 64756d6d792d746f6b656e"));
        assert!(!command.contains("dummy-token"));
    }

    #[test]
    fn manifest_timestamp_is_iso8601() {
        assert_eq!(timestamp().len(), 24);
        assert!(timestamp().ends_with('Z'));
    }

    #[test]
    fn keychain_command_atoms_reject_parser_metacharacters() {
        assert!(safe_command_atom("inference-groq-api-key"));
        assert!(!safe_command_atom("service; delete-keychain"));
        assert!(!safe_command_atom("account\\n-argument"));
    }

    #[test]
    fn locked_diagnostic_match_is_case_insensitive() {
        assert!(contains_ascii_case_insensitive(
            b"Vault is LOCKED",
            b"locked"
        ));
        assert!(!contains_ascii_case_insensitive(
            b"network unavailable",
            b"locked"
        ));
    }

    #[test]
    fn unlock_diagnostic_names_an_unreachable_server() {
        let stderr = b"Unable to fetch ServerConfig from https://homepi.example.ts.net/api FetchError: request to https://homepi.example.ts.net/api/config failed, reason: getaddrinfo ENOTFOUND homepi.example.ts.net";
        let message = unlock_diagnostic(stderr).expect("a transport failure is named");
        assert!(message.contains("https://homepi.example.ts.net/api"));
        assert!(message.contains("Tailscale"));
        assert!(!message.contains("homepi.example.ts.net/api FetchError"));
    }

    #[test]
    fn unlock_diagnostic_reports_a_rejected_password_verbatim() {
        assert_eq!(
            unlock_diagnostic(b"\nInvalid master password.\n").as_deref(),
            Some("Bitwarden unlock failed: Invalid master password.")
        );
        assert_eq!(unlock_diagnostic(b""), None);
    }

    #[test]
    fn request_budget_refuses_expired_fallbacks() {
        assert!(RequestBudget::new(Duration::ZERO).remaining().is_err());
        assert!(!RequestBudget::new(Duration::from_secs(1))
            .remaining()
            .unwrap()
            .is_zero());
    }
}
