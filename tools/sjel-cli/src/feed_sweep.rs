//! `tools/feed-sweep` — one pass of the Feed source collector, then exit.
//!
//! Ported from `tools/feed-sweep.ts` on 2026-10-04, so the six-hourly job starts neither an
//! interpreter nor a shell: `capabilities/feed-sweep/service.toml` names the built binary
//! directly, the shape `host-watch` and every Rust capability already use.
//!
//! ## Why the job exists
//!
//! The sweep itself has always existed as comms' `POST /sources/scan`, and until 2026-08-30 the
//! only thing that ever called it was a button in the dashboard — so on a machine where nobody
//! opened the dashboard, nothing was collected and every source's `last_run_at` quietly aged.
//! This is the caller that runs without anyone watching.
//!
//! A job rather than a fifth tokio ticker inside comms-server. The drains beside it are fast,
//! local and stateful; a collector that walks the open web is slow, allowed to fail, and keeps
//! nothing between runs — and giving comms-server a fourth reason to be up is the opposite of
//! what an on-demand capability is for.
//!
//! ## Composition
//!
//! It talks to comms over HTTP and never to comms' database. That is the documented composition
//! edge (CONTRIBUTING.md#schemas-and-dependency-direction): a capability depends on another's
//! contract, never its code. It is also what keeps a second process out of that store —
//! `Store::open` runs the whole migration on every call, and two openers doing that concurrently
//! deadlock on the table locks a no-op `ALTER TABLE` still takes.
//!
//! ## Differences from the TypeScript, all deliberate
//!
//! `-h`/`--help` prints the usage, which the TypeScript did not read at all — every other ported
//! tool answers it, and a scheduled job invoked by hand deserves the same. The two requests keep
//! their `AbortSignal` budgets (300 s and 600 s) through `sjel_http::client`, which is also where
//! the user-agent and the redirect policy come from; the TypeScript used the platform's `fetch`
//! defaults for both. The relevance body is parsed as JSON only after the status check, as before.
//! The comms bearer token is read once instead of once per request — it is one file and the run
//! is minutes long, so a rotation mid-run is not a case worth a second read.

use crate::paths::{manifest_port, Paths};
use sjel_http::Purpose;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

const HELP: &str = "\
tools/feed-sweep — one pass of the Feed source collector, then exit.

  tools/feed-sweep      scan every enabled source, then rank one relevance page
  tools/feed-sweep -h   this help

comms must be answering on its declared port. Schedule: capabilities/feed-sweep/service.toml
";

/// The scan walks the open web and comms paces each adapter; the relevance page re-embeds rows.
/// Both are the budgets the TypeScript gave each request.
const SCAN_TIMEOUT: Duration = Duration::from_secs(300);
const RELEVANCE_TIMEOUT: Duration = Duration::from_secs(600);

fn fail(message: &str) -> ExitCode {
    eprintln!("feed-sweep: {message}");
    ExitCode::from(1)
}

pub fn run(argv: &[String]) -> ExitCode {
    if argv.iter().any(|a| a == "-h" || a == "--help") {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }

    let paths = match Paths::from_env() {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    let Some(overlay) = paths.overlay_root.clone() else {
        return fail("no 'overlay' in axon.local.toml or axon.toml — run tools/install.sh");
    };
    let port = match manifest_port(&paths, "comms") {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    let token = match comms_token(&overlay, &paths.root) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };

    let scan_url = format!("http://127.0.0.1:{port}/sources/scan");
    let client = match sjel_http::client(Purpose::new("feed-sweep"), SCAN_TIMEOUT) {
        Ok(c) => c,
        Err(e) => return fail(&format!("client build: {e}")),
    };

    let response = match client
        .post(&scan_url)
        // Header, not query string (OPERATIONAL_RULES).
        .bearer_auth(&token)
        .header("content-type", "application/json")
        // An empty body scans every ENABLED source. Which sources are enabled is comms'
        // configuration to state, so this job never names one.
        .body("{}")
        .send()
    {
        Ok(r) => r,
        // comms being down is the expected failure: it is an on-demand capability and this job
        // runs on a timer that knows nothing about that. Reported and non-zero, never swallowed:
        // a schedule that silently does nothing is indistinguishable from one that is working.
        Err(e) => {
            return fail(&format!(
                "comms is not answering on 127.0.0.1:{port} — {e}"
            ))
        }
    };

    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        // comms' own error string, which is about a feed source and never about a credential.
        return fail(&format!(
            "/sources/scan answered {}: {}",
            status.as_u16(),
            truncate(&body, 400)
        ));
    }

    let parsed: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            return fail(&format!(
                "/sources/scan answered 200 with a body that is not JSON: {}",
                truncate(&body, 200)
            ))
        }
    };

    // `sources`, matching the key comms' handler actually emits — read off the live response, not
    // guessed from the handler's local variable name, which is `results`.
    let results = parsed
        .get("sources")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    // One line per source and a total, so a run that found nothing reads differently from a run
    // that never happened — which is the whole reason this job exists.
    let mut broken = 0usize;
    for row in results {
        let source_id = row
            .get("source_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("?");
        // A source that could not be reached at all reports its reason and nothing else. Without
        // this it renders as `discovered=0 fetched=0`, which is exactly what a source with
        // nothing new looks like — and the difference between "quiet" and "broken" is the only
        // thing a log nobody reads until something is wrong actually needs to say.
        if let Some(error) = row.get("error").and_then(serde_json::Value::as_str) {
            broken += 1;
            eprintln!("feed-sweep: {source_id} FAILED — {error}");
            continue;
        }
        let count = |key: &str| row.get(key).and_then(serde_json::Value::as_i64).unwrap_or(0);
        let failed = row
            .get("failed")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len);
        println!(
            "feed-sweep: {source_id} discovered={} fetched={} new={} failed={failed}",
            count("discovered"),
            count("fetched"),
            count("new_count"),
        );
    }
    let new_count = parsed
        .get("new_count")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    println!(
        "feed-sweep: {} source(s), {new_count} new item(s){}",
        results.len(),
        if broken > 0 {
            format!(", {broken} source(s) unreachable")
        } else {
            String::new()
        }
    );

    relevance_page(&port, &token);
    ExitCode::SUCCESS
}

/// One bounded relevance page after the scan, so newly collected items are ranked by the time
/// anyone opens the Inbox — and so a lexical row written while the embedding role was down gets a
/// chance to become semantic on an ordinary schedule rather than only on a manual backfill.
///
/// Bounded on purpose: one page of 100, not the whole corpus. This job's contract is the scan;
/// re-scoring everything belongs to `comms relevance backfill`, which pages explicitly.
///
/// No `offset`, deliberately: the route reads the last page's cursor and hands back the NEXT
/// hundred rows, so the nightly schedule walks the corpus a page at a time and wraps at the end.
/// Sending `offset: 0` would pin the sweep to the newest hundred rows, which is what it did until
/// 2026-09-08 — see `page_offset` in capabilities/comms/src/server/feed.rs.
///
/// A failure here is reported and does NOT fail the run. The scan already succeeded and its result
/// is what the schedule exists to produce; refusing to record that because a ranking pass fell
/// over would be the tail wagging the dog.
fn relevance_page(port: &str, token: &str) {
    let skipped = |reason: String| eprintln!("feed-sweep: relevance page skipped — {reason}");
    let Ok(client) = sjel_http::client(Purpose::new("feed-sweep"), RELEVANCE_TIMEOUT) else {
        skipped("client build failed".to_owned());
        return;
    };
    let url = format!("http://127.0.0.1:{port}/feed/relevance/refresh");
    let response = match client
        .post(&url)
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body(r#"{"days":3650,"limit":100}"#)
        .send()
    {
        Ok(r) => r,
        Err(e) => return skipped(e.to_string()),
    };
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        eprintln!(
            "feed-sweep: relevance page answered {}: {}",
            status.as_u16(),
            truncate(&body, 400)
        );
        return;
    }
    let page: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return skipped(e.to_string()),
    };

    let number = |key: &str| page.get(key).and_then(serde_json::Value::as_i64).unwrap_or(0);
    let embedding = page.get("embedding");
    let mode = embedding
        .and_then(|e| e.get("mode"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let fallback = embedding
        .and_then(|e| e.get("error_class"))
        .and_then(serde_json::Value::as_str)
        .map(|class| format!(" fallback={class}"))
        .unwrap_or_default();
    // `offset` is in the line because it is now the route's answer rather than this job's
    // question, and it is the one number that says whether the sweep is moving. A log that read
    // the same every night is what let it stand still at 0 unnoticed.
    let more = if page
        .get("has_more")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        " (more pages remain — tomorrow's page continues from here, or run `comms relevance backfill` now)"
    } else {
        ""
    };
    println!(
        "feed-sweep: relevance offset={} considered={} re-scored={} re-evaluated={} mode={mode}{fallback}{more}",
        number("offset"),
        number("considered"),
        number("rescored"),
        number("reused_relevance"),
    );
}

/// The token for comms' mutating routes, resolved the way the Rust server and the dashboard's
/// Vite proxy resolve it: `comms.json` names a file, the file holds the secret. It is read into
/// memory and sent as a header — never as an argument, where `ps` would show it, and never in the
/// URL, which leaks to logs, history and referrers.
fn comms_token(overlay: &Path, root: &Path) -> Result<String, String> {
    let config_path = overlay.join("config").join("comms.json");
    if !config_path.is_file() {
        return Err(format!("no {}", config_path.display()));
    }
    let text = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("could not read {}: {e}", config_path.display()))?;
    let parsed: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("could not read {}: {e}", config_path.display()))?;
    let reference = parsed
        .get("api_secret_file")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(
            "comms.json has no api_secret_file — /sources/scan rejects every request without one",
        )?;
    let path = if let Some(rest) = reference.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest)
    } else if Path::new(reference).is_absolute() {
        PathBuf::from(reference)
    } else {
        root.join(reference)
    };
    // The path is named, the contents never are. A message that quoted the file would be one
    // `cat` away from a message that quoted the secret.
    let token = std::fs::read_to_string(&path)
        .map_err(|_| "cannot read the comms API secret file named by comms.json".to_owned())?
        .trim()
        .to_owned();
    if token.is_empty() {
        return Err("the comms API secret file is empty".to_owned());
    }
    Ok(token)
}

/// `body.slice(0, n)`, counting characters rather than bytes so a multi-byte body cannot split.
fn truncate(body: &str, n: usize) -> String {
    body.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_is_truncated_without_splitting_a_character() {
        assert_eq!(truncate("abcdef", 3), "abc");
        assert_eq!(truncate("abc", 9), "abc");
        assert_eq!(truncate("äöü", 2), "äö");
    }

    #[test]
    fn a_comms_json_with_no_secret_reference_is_refused_by_name() {
        let dir = std::env::temp_dir().join(format!("feed-sweep-test-{}", std::process::id()));
        let config = dir.join("config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("comms.json"), r#"{"port": 8099}"#).unwrap();
        let err = comms_token(&dir, &dir).unwrap_err();
        assert!(err.contains("no api_secret_file"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The file is named, its contents are not: a message that quoted it would be one `cat` away
    /// from a message that quoted the secret.
    #[test]
    fn an_unreadable_secret_file_never_names_its_contents() {
        let dir = std::env::temp_dir().join(format!("feed-sweep-test-missing-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("config")).unwrap();
        std::fs::write(
            dir.join("config").join("comms.json"),
            r#"{"api_secret_file": "nowhere/secret.txt"}"#,
        )
        .unwrap();
        let err = comms_token(&dir, &dir).unwrap_err();
        assert_eq!(
            err,
            "cannot read the comms API secret file named by comms.json"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
