use super::*;

/// Resolve the active deployment overlay that owns backup receipts.
pub(crate) fn overlay_root() -> Result<PathBuf, String> {
    sjel_config::env_var("SJEL_OVERLAY_ROOT")
        .map(PathBuf::from)
        .map_err(|_| {
            "SJEL_OVERLAY_ROOT is not set — start this through tools/service-runner.sh, or export it"
                .to_string()
        })
}

/// What `backup.sh` writes after the remote byte count matched. Only the fields this
/// surface projects are named: `target`, `tarball` and `sha256` are deliberately absent
/// from the struct, so there is no path by which a destination reaches a response.
#[derive(Deserialize)]
pub(crate) struct Receipt {
    pub(crate) completed_at: String,
    #[serde(default)]
    pub(crate) bytes: u64,
    #[serde(default)]
    pub(crate) contents: String,
}

/// `20260805T220018Z` — fixed-width UTC, written by `date -u +%Y%m%dT%H%M%SZ` in
/// `backup.sh`, parsed here by hand.
///
/// Hand-rolled rather than adding a date crate: the format is ours and fixed, and the
/// alternative costs a dependency in Cargo.lock for one `strptime`. `None` on
/// anything that does not match, which reads downstream as "no
/// usable receipt" — the same answer as a missing file, and the right one, because a
/// receipt this process cannot date cannot be used to claim a backup is fresh.
pub(crate) fn parse_receipt_ts(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() != 16 || b[8] != b'T' || b[15] != b'Z' {
        return None;
    }
    let num = |from: usize, to: usize| s.get(from..to)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(4, 6)?, num(6, 8)?);
    let (h, mi, sec) = (num(9, 11)?, num(11, 13)?, num(13, 15)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }

    // days_from_civil (Howard Hinnant's civil-calendar algorithm): era arithmetic, so no
    // leap-year special-casing and no table.
    let y = y - i64::from(mo <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    u64::try_from(days * 86_400 + h * 3_600 + mi * 60 + sec).ok()
}

pub(crate) fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One in-flight or finished run, kept in this process.
///
/// Process memory rather than a file on purpose: a run's progress is a fact about THIS
/// process, and the durable record of a backup is the receipt `backup.sh` writes. That
/// split is what lets a slow run survive a page refresh — the page is not holding the
/// state — while a restart of sjel-status correctly forgets a run it can no longer
/// observe, instead of leaving a "running" marker on disk that nothing will ever clear.
#[derive(Clone, Serialize)]
pub(crate) struct BackupRun {
    pub(crate) state: &'static str,
    pub(crate) started_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) finished_at: Option<u64>,
    /// `backup.sh`'s own stderr on failure. It reports progress and errors in terms of
    /// capability names and staging steps; the one place it names a destination is the
    /// final success line, which is not on this path.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub(crate) detail: String,
}

pub(crate) static BACKUP_RUNS: OnceLock<Mutex<HashMap<String, BackupRun>>> = OnceLock::new();

/// The run ledger, locked, with a poisoned lock recovered rather than propagated.
///
/// It returns the guard rather than the `Mutex` on purpose: three call sites each wrote
/// `.lock().unwrap()`, and a decision about poisoning that has to be repeated at every call
/// site is a decision the fourth call site will get wrong. This is the only place that locks
/// it, so it is the only place that has to be right.
///
/// **Recover, not fatal**, and the reason is what this map is. It holds one `BackupRun` per
/// capability — a state word, two timestamps and `backup.sh`'s stderr. Nothing on the other
/// side of the lock is a durable invariant: the map is a report about runs, and the runs
/// themselves are subprocesses that outlive any panic here. A `HashMap` cannot be left half
/// updated in safe Rust either, so the value a poisoned lock guards is the value that was
/// there before or after one `insert`, and both are readable.
///
/// The cost of the alternative is the argument. `unwrap()` on a poisoned lock is a second
/// panic, so one panic anywhere under this guard would make `GET /api/sjel-status/backups`
/// answer 500 for the life of the process, and every later `POST .../backup` with it — a
/// backup page that reports nothing, forever, because something once panicked while it was
/// being drawn. `capabilities/scouting/src/config.rs`'s `env_lock` made the same call for
/// the same reason: refusing "would turn one failure into every later [call] failing for a
/// different reason".
///
/// What this deliberately does NOT do is hide the first panic. That one still unwinds, still
/// reaches the runner's captured stderr, and is still the thing to read.
pub(crate) fn backup_runs() -> std::sync::MutexGuard<'static, HashMap<String, BackupRun>> {
    BACKUP_RUNS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Age against the capability's own two thresholds.
///
/// `unknown` when the manifest declares no `backup_stale_days`: this surface will not
/// invent a cadence for data whose owner did not state one, because a red badge derived
/// from a number Axon made up is worse than no badge. `never` outranks everything —
/// a capability with a backup contract and no receipt has the problem, whatever its
/// thresholds say.
///
/// What the two words mean, because the thresholds are a manifest's to choose and this
/// is the one place that reads them:
///
/// - **due** — the data is older than the owner said it should be. A reminder. It is
///   the *expected* state for any manual contract between two runs.
/// - **overdue** — the schedule that should have refreshed it did not. A fault.
///
/// So `stale_days` is not "a bit more than advise_days"; it is the age that can only be
/// reached by runs that are failing. A daily contract reaches it in two days. A manual
/// contract — vaultwarden, which needs an unlocked vault and a held-down container —
/// legitimately keeps a week, because nothing there is scheduled to fail.
///
/// Getting that wrong is silent by construction, and it was: the store's daily backup
/// failed on every run from 2026-08-24, and this function answered "due" for three days
/// because that contract had inherited `stale_days = 7` (PRD Q47, measured 2026-08-26).
/// Nothing here was broken. The number it was reading was.
pub(crate) fn backup_state(age_secs: Option<u64>, c: &BackupContract) -> &'static str {
    let Some(age) = age_secs else { return "never" };
    let days = age / 86_400;
    match (c.stale_days, c.advise_days) {
        (Some(stale), _) if days >= stale => "overdue",
        (_, Some(advise)) if days >= advise => "due",
        (None, None) => "unknown",
        _ => "ok",
    }
}

/// Every capability that declares a backup contract, with the age of its last one.
pub(crate) async fn backups_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let services = registry().await.map_err(bad_gateway)?;
    let overlay = overlay_root().map_err(bad_gateway)?;
    let now = now_epoch();
    let runs = backup_runs().clone();

    let mut out = Vec::new();
    for service in services {
        let Some(contract) = service.backup_contract() else {
            continue;
        };
        let receipt: Option<Receipt> = std::fs::read_to_string(
            overlay
                .join("backup/receipts")
                .join(format!("{}.json", service.name)),
        )
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok());

        let last = receipt
            .as_ref()
            .and_then(|r| parse_receipt_ts(&r.completed_at));
        // saturating: a receipt dated in the future is a clock problem, not a negative
        // age, and it must not wrap into "overdue by 500 years".
        let age = last.map(|t| now.saturating_sub(t));

        out.push(json!({
            "capability": service.name,
            "state": backup_state(age, &contract),
            "holds_service": contract.holds_service,
            "advise_days": contract.advise_days,
            "stale_days": contract.stale_days,
            "last_success": receipt.as_ref().map(|r| r.completed_at.clone()),
            "age_seconds": age,
            "bytes": receipt.as_ref().map(|r| r.bytes),
            "contents": receipt.as_ref().map(|r| r.contents.clone()),
            "run": runs.get(&service.name),
        }));
    }
    Ok(Json(json!({ "backups": out })))
}

/// Ask for a backup now. Accepts the run and returns; it does not wait for it.
///
/// Asynchronous because a real run tars, hashes, ships over ssh and verifies a remote
/// byte count — minutes, not one HTTP request. The response says the run was accepted;
/// `GET /api/sjel-status/backups` says how it went, and keeps saying so across a page
/// refresh because the state lives here rather than in the page.
pub(crate) async fn backup_handler(
    Path(name): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let services = registry().await.map_err(bad_gateway)?;
    let Some(service) = services.into_iter().find(|s| s.name == name) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(
                json!({ "error": format!("'{name}' is not an enabled capability on this machine") }),
            ),
        ));
    };
    if service.backup_contract().is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("'{name}' declares no backup contract") })),
        ));
    }

    let root = axon_root().map_err(bad_gateway)?;
    let started = now_epoch();
    {
        // Refuse rather than queue. A second concurrent run on one capability is not a
        // wasted fork: for a SQLite contract both runs drive the same maintenance hold,
        // and the first to finish resumes the capability out from under the second,
        // which is how a "coherent cold snapshot" stops being either.
        let mut runs = backup_runs();
        if let Some(existing) = runs.get(&name).filter(|r| r.state == "running") {
            return Err((
                StatusCode::CONFLICT,
                Json(json!({
                    "error": format!("a backup of '{name}' is already running"),
                    "run": existing,
                })),
            ));
        }
        runs.insert(
            name.clone(),
            BackupRun {
                state: "running",
                started_at: started,
                finished_at: None,
                detail: String::new(),
            },
        );
    }

    let task_name = name.clone();
    tokio::spawn(async move {
        let out = tokio::process::Command::new(root.join("tools/backup.sh"))
            .arg(&task_name)
            .output()
            .await;
        let (state, detail) = match out {
            Ok(o) if o.status.success() => ("succeeded", String::new()),
            Ok(o) => (
                "failed",
                String::from_utf8_lossy(&o.stderr).trim().to_string(),
            ),
            Err(e) => ("failed", format!("could not run tools/backup.sh: {e}")),
        };
        let mut runs = backup_runs();
        runs.insert(
            task_name,
            BackupRun {
                state,
                started_at: started,
                finished_at: Some(now_epoch()),
                detail,
            },
        );
    });

    Ok(Json(json!({
        "name": name,
        "accepted": true,
        "holds_service": service.backup_contract().map(|c| c.holds_service),
    })))
}
