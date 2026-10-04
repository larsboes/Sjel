//! The pure half of `tools/host-watch`: what a process list, a storage verdict and a host-net
//! verdict mean, and what the store should do about them.
//!
//! No I/O, no clock and no policy file — `mod.rs` owns all three. The cases `tools/host-watch.test.ts`
//! held live here as Rust unit tests, and every fixture row below is a real measurement from
//! the 2026-08-15 incident (Axon#177) rather than an invented number, because the whole
//! question this tool answers is "which of these two processes is the runaway" and the honest
//! answer is only interesting when both rows look alarming.
//!
//! The one that earns its keep is [`classify_processes`]: WindowServer had MORE cumulative CPU
//! than the stuck extension (168m vs 148m) and was fine. A tool that ranks by CPU time flags
//! the compositor every day and is muted within a week.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// One row of `ps -Aceo pid,time,etime,comm`.
#[derive(Debug, Clone, PartialEq)]
pub struct Proc {
    pub pid: i64,
    /// Cumulative CPU time consumed, in seconds.
    pub cpu_seconds: f64,
    /// Wall-clock time the process has existed, in seconds.
    pub elapsed_seconds: f64,
    pub comm: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProcessBudget {
    pub min_cpu_seconds: Option<f64>,
    pub min_cpu_ratio: Option<f64>,
}

/// One `[[allow_process]]` entry. Only `comm` is read: the schema requires a `reason` beside it,
/// and that is for the human deciding whether the name still belongs there. It is not a field
/// here because nothing read it in the TypeScript either, and serde ignores it in the file.
#[derive(Debug, Clone, Deserialize)]
pub struct AllowedProcess {
    pub comm: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WatchPolicy {
    pub process: Option<ProcessBudget>,
    #[serde(default)]
    pub allow_process: Vec<AllowedProcess>,
}

/// A condition worth a row, which is also exactly what the JSON verbs print.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    pub key: String,
    pub title: String,
    pub note: String,
}

/// A runaway, plus the numbers the note was built from.
///
/// Separate from [`Finding`] rather than flattened into it because the extra fields are what
/// `--json` reports and the tests assert on; the store only ever sees `finding`.
///
/// The declaration order IS the wire order, and `rename_all` keeps the one camelCase field the
/// TypeScript printed: this payload is `--json`'s whole surface, so it stays byte-identical to
/// its predecessor rather than being tidied into house style.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcFinding {
    pub key: String,
    pub pid: i64,
    pub comm: String,
    pub ratio: f64,
    pub cpu_seconds: f64,
    pub title: String,
    pub note: String,
}

impl ProcFinding {
    pub fn finding(&self) -> Finding {
        Finding {
            key: self.key.clone(),
            title: self.title.clone(),
            note: self.note.clone(),
        }
    }
}

/// One row of `host_watch_findings`, as the decision below needs to see it.
#[derive(Debug, Clone, PartialEq)]
pub struct FindingRow {
    pub id: String,
    pub key: String,
    pub generation: i64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Emission {
    /// The condition is already open: update it in place.
    Refresh { id: String },
    /// The condition is new, or it cleared and came back.
    Create { id: String, generation: i64 },
}

/// `ps` prints three shapes and only three: `DD-HH:MM:SS`, `HH:MM:SS`, and `MM:SS[.ff]`,
/// where MM is NOT bounded at 60 (a process with 168 minutes of CPU prints `168:50.56`).
/// Anything else is a row we do not understand, and the honest value for that is 0 —
/// NaN would propagate into a ratio and a garbage row would become a finding.
fn hms_to_seconds(raw: &str) -> f64 {
    let t = raw.trim();
    // The day form is the only one with a `-` in it, which is what tells it apart.
    if let Some((days, clock)) = t.split_once('-') {
        return match (number(days), seconds_from_clock(clock, 3)) {
            (Some(days), Some(secs)) => days * 86400.0 + secs,
            _ => 0.0,
        };
    }
    // `HH:MM:SS(.ff)` first, then `MM:SS(.ff)`: the leading field is hours in the three-field
    // form and minutes in the two-field one.
    seconds_from_clock(t, 3)
        .or_else(|| seconds_from_clock(t, 2))
        .unwrap_or(0.0)
}

/// `<h>:<m>:<s[.ff]>` in the three-field form, `<m>:<s[.ff]>` in the two-field one. Neither
/// leading field is bounded.
fn seconds_from_clock(t: &str, fields: usize) -> Option<f64> {
    let parts: Vec<&str> = t.split(':').collect();
    if parts.len() != fields {
        return None;
    }
    let first = number(parts[0])?;
    let minutes = number(parts[1])?;
    Some(match fields {
        3 => first * 3600.0 + minutes * 60.0 + number(parts[2])?,
        _ => first * 60.0 + minutes,
    })
}

/// ASCII digits and at most one dot, so `1e5`, `inf`, `+1` and `" 1"` are all rejected — the
/// same set the TypeScript's `\d+(\.\d+)?` accepts, which is the property that keeps a
/// nonsense row out of the ratio arithmetic.
fn number(s: &str) -> Option<f64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    s.parse().ok()
}

/// Cumulative CPU time a process has consumed, in seconds.
pub fn parse_cpu_time(raw: &str) -> f64 {
    hms_to_seconds(raw)
}

/// Wall-clock time a process has existed, in seconds.
pub fn parse_elapsed(raw: &str) -> f64 {
    hms_to_seconds(raw)
}

/// `ps -Aceo pid,time,etime,comm`. The command is LAST because it is the only field that
/// contains spaces ("Spotify Helper (Renderer)"), so it takes the rest of the line; the header
/// has no leading digits and falls out on its own.
pub fn parse_ps_output(text: &str) -> Vec<Proc> {
    text.lines().filter_map(parse_ps_line).collect()
}

fn parse_ps_line(line: &str) -> Option<Proc> {
    let mut rest = line;
    let pid = take_field(&mut rest)?;
    if pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let cpu = take_field(&mut rest)?;
    let elapsed = take_field(&mut rest)?;
    // The remainder, not a fourth split: this is the field allowed to contain spaces, and
    // re-joining a split would rewrite any run of two of them.
    let comm = rest.trim();
    if comm.is_empty() {
        return None;
    }
    Some(Proc {
        pid: pid.parse().ok()?,
        cpu_seconds: parse_cpu_time(cpu),
        elapsed_seconds: parse_elapsed(elapsed),
        comm: comm.to_owned(),
    })
}

/// The next whitespace-delimited field, leaving `rest` past it.
fn take_field<'a>(rest: &mut &'a str) -> Option<&'a str> {
    let trimmed = rest.trim_start();
    let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let (field, tail) = trimmed.split_at(end);
    if field.is_empty() {
        return None;
    }
    *rest = tail;
    Some(field)
}

/// The runaway rule, and the reason it is two conditions rather than one.
///
/// Ranking by cumulative CPU is the obvious implementation and it is wrong: on the day this was
/// written WindowServer had MORE CPU time than the stuck extension (168m vs 148m) and was
/// perfectly healthy — it had simply been alive four times longer. The signal is the RATIO, how
/// much of a core a process has held for its whole life.
///
/// The absolute floor is the second condition, and it exists for the opposite error: a compiler
/// at 100% for five minutes has a ratio of 1.0 and is not a runaway, it is a build. Something
/// has to have been wrong for a while before it is worth an interrupt.
///
/// The ratio is deliberately NOT capped at 1. A process pinning four cores for an hour reads as
/// 4.0, which is exactly how alarming it should look.
///
/// ## One finding per command, the worst instance
///
/// The key is the command, so a browser with four helper processes over the line is ONE
/// condition, not four. Collapsing them here rather than downstream was found by the first
/// end-to-end run against the new store, which failed on the unique index: `tasks` had been
/// absorbing the duplicates silently, returning the row it already owned and letting this tool
/// count a second "new task" that was never written. The worst instance wins because it is the
/// one worth looking at, and the note names how many others crossed the line so the count is
/// not lost with them.
pub fn classify_processes(procs: &[Proc], policy: &WatchPolicy) -> Vec<ProcFinding> {
    let floor = policy
        .process
        .as_ref()
        .and_then(|p| p.min_cpu_seconds)
        .unwrap_or(f64::INFINITY);
    let min_ratio = policy
        .process
        .as_ref()
        .and_then(|p| p.min_cpu_ratio)
        .unwrap_or(f64::INFINITY);
    let allowed: HashSet<&str> = policy
        .allow_process
        .iter()
        .map(|a| a.comm.as_str())
        .collect();

    let mut over: Vec<(f64, &Proc)> = procs
        .iter()
        .filter(|p| !allowed.contains(p.comm.as_str()))
        .filter(|p| p.cpu_seconds >= floor)
        .filter_map(|p| {
            let ratio = if p.elapsed_seconds > 0.0 {
                p.cpu_seconds / p.elapsed_seconds
            } else {
                0.0
            };
            (ratio >= min_ratio).then_some((ratio, p))
        })
        .collect();
    // Worst first. `partial_cmp` rather than `total_cmp` so the ordering is the one the sort
    // below always had; neither input can be NaN (see `number`).
    over.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut found: Vec<ProcFinding> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (ratio, p) in &over {
        // Keyed on the command, never the pid: a pid is a different number every boot and
        // would make every restart look like a new problem.
        let key = format!("cpu:{}", p.comm);
        if !seen.insert(key.clone()) {
            continue;
        }
        let others = over.iter().filter(|(_, q)| q.comm == p.comm).count() - 1;
        let others_note = if others > 0 {
            format!(
                "{others} other process(es) named {} are also over the line; this is the worst.\n",
                p.comm
            )
        } else {
            String::new()
        };
        found.push(ProcFinding {
            title: format!(
                "{} has used {} of CPU ({ratio:.2} cores sustained)",
                p.comm,
                hours(p.cpu_seconds),
            ),
            note: format!(
                "pid {pid} · {cpu} CPU over {wall} wall = {ratio:.2} cores held continuously.\n\
                 {others_note}Inspect: ps -p {pid} -o pid,lstart,time,pcpu,command\n\
                 If it is stuck rather than working: kill {pid}",
                pid = p.pid,
                cpu = hours(p.cpu_seconds),
                wall = hours(p.elapsed_seconds),
            ),
            key,
            pid: p.pid,
            comm: p.comm.clone(),
            ratio: *ratio,
            cpu_seconds: p.cpu_seconds,
        });
    }
    found
}

fn hours(s: f64) -> String {
    if s >= 3600.0 {
        format!("{:.1}h", s / 3600.0)
    } else {
        format!("{}m", (s / 60.0).round() as i64)
    }
}

/// `tools/storage report --json`, of which only these two fields matter.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct StorageReport {
    pub disk: Option<StorageDisk>,
    pub state: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StorageDisk {
    pub free: Option<f64>,
    pub target: Option<String>,
}

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

/// One finding title needs a byte count in the same units storage prints. `tools/storage` became
/// a Rust crate on 2026-09-03, so the four lines are here rather than in a shim kept alive to
/// export them — and the unit switch at 1 GB is the part that has to agree with `fmt_bytes` in
/// `tools/storage/src/measure.rs`.
fn fmt_bytes(b: f64) -> String {
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{} MB", (b / (1024.0 * 1024.0)).round() as i64)
    }
}

/// The volume state is the finding; a large class is not.
///
/// Decided in Axon#177 against the tempting alternative. `class_flag_gb` legitimately fires
/// today — the cargo target dir is 28 GB against a 20 GB flag — on a machine with 130 GB free
/// and nothing wrong with it. Alerting on that would mean this watcher's FIRST run produced a
/// task nobody needed, which is precisely how a watcher gets muted. The class breakdown stays
/// what it already was: what `sysmon storage` tells you once you are looking.
pub fn storage_finding(report: &StorageReport) -> Option<Finding> {
    let state = report.state.as_deref().unwrap_or("ok");
    if state == "ok" {
        return None;
    }
    let free = report.disk.as_ref().and_then(|d| d.free).unwrap_or(0.0);
    let target = report
        .disk
        .as_ref()
        .and_then(|d| d.target.as_deref())
        .unwrap_or("the data volume");
    Some(Finding {
        key: "storage:free-below-threshold".to_owned(),
        title: format!("Disk {state}: {} free on {target}", fmt_bytes(free)),
        note: format!(
            "Free space crossed the {state} threshold in the overlay's storage-policy.toml.\n\
             What is filling it, and what is safe to reclaim: tools/sysmon storage\n\
             Reclaim the applicable classes: tools/sysmon storage --apply"
        ),
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetExposure {
    pub process: String,
    pub port: String,
    pub protos: String,
    pub pid: i64,
}

/// `host-net check --json`, of which only the unexpected listeners matter.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NetReport {
    #[serde(default)]
    pub unexpected: Vec<NetExposure>,
    pub policy: Option<String>,
}

/// One finding for the whole condition, never one per port.
///
/// The same rule `cpu:<comm>` follows, for the same reason and one sharper case. The partial
/// unique index allows one open row per key, and a mesh VPN's wildcard ports are assigned per
/// start — `*:41641` today, a different number after the next restart — so a per-port key would
/// mint a fresh generation every hour and the ladder would fill with the same fact.
///
/// host-net owns the scope rule, the policy file and the parsing. This reads its verdict and
/// adds nothing: the note lists what came back, in the order host-net sorted it.
pub fn net_finding(report: Option<&NetReport>) -> Option<Finding> {
    let unexpected = report.map(|r| r.unexpected.as_slice()).unwrap_or(&[]);
    if unexpected.is_empty() {
        return None;
    }
    let mut names: Vec<&str> = Vec::new();
    for exposure in unexpected {
        if !names.contains(&exposure.process.as_str()) {
            names.push(&exposure.process);
        }
    }
    let shown: Vec<&str> = names.iter().copied().take(3).collect();
    let title = format!(
        "{} wildcard listener(s) not in the host-net policy ({}{})",
        unexpected.len(),
        shown.join(", "),
        if names.len() > 3 { ", …" } else { "" }
    );
    let listed: Vec<String> = unexpected
        .iter()
        .map(|e| {
            format!(
                "{} on *:{} ({}) · pid {}",
                e.process, e.port, e.protos, e.pid
            )
        })
        .collect();
    Some(Finding {
        key: "net:unexpected-exposure".to_owned(),
        title,
        note: format!(
            "{}\n\nEvery interface this host has, now or later, reaches these.\n\
             Inspect: host-net listen\n\
             Accept one by adding an [[expect_wildcard]] entry to {}",
            listed.join("\n"),
            report
                .and_then(|r| r.policy.as_deref())
                .unwrap_or("the host-net policy")
        ),
    })
}

/// One row per RUN of a condition, not one per check and not one forever.
///
/// The unique index on an open `key` collapses repeats, which is most of the job. What it cannot
/// express on its own is the case that makes the difference between a watcher that works next
/// year and one that silently stops: the condition clears, and six weeks later it comes back.
/// Re-using the same row would upsert onto the closed one and say nothing, forever. So a row
/// carries a generation, and a new one is minted only once every prior row for this condition is
/// closed.
///
/// The generation used to be packed into the id as `{key}~{n}` because `tasks` gave this watcher
/// one string field to key on, and parsing that string back out is where the two bugs of
/// Axon#177 lived — a `#` separator truncated the PATCH path, and without any separator
/// `cpu:Storage` claimed `cpu:StorageManagementService`'s history. Owning the table makes both
/// unrepresentable: `key` and `generation` are columns, and the comparison below is exact by
/// construction rather than by choosing a lucky character.
pub fn decide_emission(key: &str, existing: &[FindingRow]) -> Emission {
    let mut highest = 0;
    for row in existing.iter().filter(|row| row.key == key) {
        if row.status == "open" {
            return Emission::Refresh { id: row.id.clone() };
        }
        highest = highest.max(row.generation);
    }
    let generation = highest + 1;
    Emission::Create {
        id: format!("{key}~{generation}"),
        generation,
    }
}

/// Which open findings this run did NOT see, and must therefore close.
///
/// The half that could not exist before. Under `tasks` a finding closed only when the operator
/// pressed Done; that button is gone with the capability, so without this a row written once
/// would stay open forever and the ladder would keep ranking a process that exited months ago.
/// A watcher whose findings only accumulate stops meaning anything.
pub fn decide_resolutions(present: &[Finding], existing: &[FindingRow]) -> Vec<String> {
    let seen: HashSet<&str> = present.iter().map(|f| f.key.as_str()).collect();
    existing
        .iter()
        .filter(|row| row.status == "open" && !seen.contains(row.key.as_str()))
        .map(|row| row.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real `ps -Aceo pid,time,etime,comm` block from 2026-08-15 17:52, trimmed to the rows
    /// that matter. ApplicationsStorageExtension is the stuck System Settings Storage pane; it
    /// had run since 08:35 and burned 2h28m of CPU by this sample.
    const PS_FIXTURE: &str = "\
  PID      TIME     ELAPSED COMM
13105 148:50.33    09:17:29 ApplicationsStorageExtension
  402 168:50.56 01-02:00:53 WindowServer
12791  39:47.47    09:17:29 Storage
 1436  40:11.42 01-02:00:41 Spotify Helper (Renderer)
  330   0:00.10 01-02:00:54 smd
";

    fn policy() -> WatchPolicy {
        WatchPolicy {
            process: Some(ProcessBudget {
                min_cpu_seconds: Some(3600.0),
                min_cpu_ratio: Some(0.15),
            }),
            allow_process: vec![AllowedProcess {
                comm: "WindowServer".to_owned(),
            }],
        }
    }

    fn proc_(pid: i64, comm: &str, cpu: f64, elapsed: f64) -> Proc {
        Proc {
            pid,
            cpu_seconds: cpu,
            elapsed_seconds: elapsed,
            comm: comm.to_owned(),
        }
    }

    fn comms(found: &[ProcFinding]) -> Vec<String> {
        found.iter().map(|f| f.comm.clone()).collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.01
    }

    // ── parseCpuTime / parseElapsed ───────────────────────────────────────────

    #[test]
    fn minutes_past_sixty_are_seconds_not_an_hour() {
        assert!(close(parse_cpu_time("148:50.33"), 8930.33));
        assert!(close(parse_cpu_time("168:50.56"), 10130.56));
        assert!(close(parse_cpu_time("0:00.10"), 0.1));
    }

    #[test]
    fn the_day_form_parses_whole_days() {
        assert_eq!(
            parse_cpu_time("02-03:04:05"),
            2.0 * 86400.0 + 3.0 * 3600.0 + 4.0 * 60.0 + 5.0
        );
        assert_eq!(parse_elapsed("01-02:00:53"), 86400.0 + 2.0 * 3600.0 + 53.0);
    }

    #[test]
    fn hours_minutes_and_seconds_all_parse() {
        assert_eq!(parse_elapsed("09:17:29"), 9.0 * 3600.0 + 17.0 * 60.0 + 29.0);
        assert_eq!(parse_elapsed("04:12"), 4.0 * 60.0 + 12.0);
    }

    /// A bad row must not become a finding: NaN would propagate into a ratio.
    #[test]
    fn anything_unparseable_is_zero_never_nan() {
        for bad in ["-", "", "garbage", "1e5", "inf", "+1", "1:2:3:4"] {
            assert_eq!(parse_cpu_time(bad), 0.0, "{bad}");
            assert!(!parse_cpu_time(bad).is_nan(), "{bad}");
            assert_eq!(parse_elapsed(bad), 0.0, "{bad}");
        }
    }

    // ── parsePsOutput ─────────────────────────────────────────────────────────

    #[test]
    fn the_header_falls_out_and_a_command_keeps_its_spaces() {
        let procs = parse_ps_output(PS_FIXTURE);
        assert_eq!(procs.len(), 5, "{procs:?}");
        assert!(procs.iter().any(|p| p.comm == "Spotify Helper (Renderer)"));
        let stuck = procs.iter().find(|p| p.pid == 13105).expect("the row");
        assert!(close(stuck.cpu_seconds, 8930.33));
    }

    // ── classifyProcesses: the runaway rule ───────────────────────────────────

    #[test]
    fn the_stuck_storage_extension_is_flagged() {
        let found = classify_processes(&parse_ps_output(PS_FIXTURE), &policy());
        assert!(comms(&found).contains(&"ApplicationsStorageExtension".to_owned()));
    }

    #[test]
    fn windowserver_is_spared_even_though_it_had_more_cumulative_cpu() {
        let procs = parse_ps_output(PS_FIXTURE);
        let ws = procs.iter().find(|p| p.comm == "WindowServer").expect("ws");
        let stuck = procs
            .iter()
            .find(|p| p.comm == "ApplicationsStorageExtension")
            .expect("stuck");
        // The premise of the test: the naive rule would flag the wrong one.
        assert!(ws.cpu_seconds > stuck.cpu_seconds);
        assert!(!comms(&classify_processes(&procs, &policy())).contains(&"WindowServer".to_owned()));
    }

    #[test]
    fn the_ratio_alone_spares_windowserver_too() {
        let mut no_allow = policy();
        no_allow.allow_process.clear();
        let found = classify_processes(&parse_ps_output(PS_FIXTURE), &no_allow);
        assert!(!comms(&found).contains(&"WindowServer".to_owned()));
    }

    #[test]
    fn a_short_burst_at_full_cpu_is_not_a_runaway() {
        let burst = vec![proc_(1, "cc1plus", 300.0, 305.0)];
        assert!(classify_processes(&burst, &policy()).is_empty());
    }

    #[test]
    fn a_process_alive_as_long_as_it_has_been_busy_cannot_exceed_one_core() {
        let found = classify_processes(&[proc_(1, "runaway", 9000.0, 9000.0)], &policy());
        assert!(found[0].ratio <= 1.0);
    }

    #[test]
    fn zero_elapsed_does_not_divide_by_zero() {
        // A ratio of 0 is below the threshold, so this is not a finding — the point is that it
        // is neither a panic nor a NaN.
        let found = classify_processes(&[proc_(1, "x", 9000.0, 0.0)], &policy());
        assert!(found.is_empty());
    }

    #[test]
    fn the_key_names_the_process_not_the_pid_or_the_clock() {
        let a = classify_processes(&parse_ps_output(PS_FIXTURE), &policy());
        let shifted = parse_ps_output(&PS_FIXTURE.replace("13105", "99999"));
        let b = classify_processes(&shifted, &policy());
        assert_eq!(a[0].key, b[0].key);
    }

    /// Regression, found by the first end-to-end run against host-watch's own table
    /// (2026-08-28): a browser has several helper processes under one command name, so one run
    /// produced several findings on one key and the store's unique index refused the second.
    /// `tasks` had been swallowing that silently and this tool counted a task it never wrote.
    #[test]
    fn several_processes_sharing_a_command_are_one_finding_the_worst_of_them() {
        let mut no_allow = policy();
        no_allow.allow_process.clear();
        let helpers = vec![
            proc_(1, "Google Chrome Helper", 7200.0, 14_400.0),
            proc_(2, "Google Chrome Helper", 7200.0, 7_200.0),
            proc_(3, "Google Chrome Helper", 7200.0, 36_000.0),
        ];
        let found = classify_processes(&helpers, &no_allow);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].pid, 2, "ratio 1.00 against 0.50 and 0.20");
        assert!(found[0]
            .note
            .contains("2 other process(es) named Google Chrome Helper"));
    }

    // ── storageFinding ────────────────────────────────────────────────────────

    fn storage(limit: &str, free: f64) -> StorageReport {
        serde_json::from_str(&format!(
            r#"{{"disk":{{"free":{free},"target":"/System/Volumes/Data"}},"state":"{limit}",
                 "classes":[{{"name":"rust-workspace-target","bytes":30500000000,"flagged":true}}]}}"#
        ))
        .expect("the storage fixture")
    }

    #[test]
    fn a_healthy_volume_is_not_a_finding() {
        assert_eq!(storage_finding(&storage("ok", 140_000_000_000.0)), None);
    }

    #[test]
    fn an_over_flag_class_on_a_healthy_volume_stays_silent() {
        // The fixture carries flagged: true, which is the decision Axon#177 made.
        assert_eq!(storage_finding(&storage("ok", 140_000_000_000.0)), None);
    }

    #[test]
    fn the_volume_state_is_the_finding_and_it_names_the_free_space() {
        let f = storage_finding(&storage("warn", 70_000_000_000.0)).expect("a finding");
        assert_eq!(f.title, "Disk warn: 65.2 GB free on /System/Volumes/Data");
        assert!(f.note.contains("tools/sysmon storage"));
    }

    #[test]
    fn the_storage_key_is_stable_so_a_week_long_breach_is_one_finding() {
        let a = storage_finding(&storage("warn", 70_000_000_000.0)).expect("a");
        let b = storage_finding(&storage("warn", 69_000_000_000.0)).expect("b");
        assert_eq!(a.key, b.key);
    }

    // ── decideEmission ────────────────────────────────────────────────────────

    fn row(id: &str, key: &str, generation: i64, status: &str) -> FindingRow {
        FindingRow {
            id: id.to_owned(),
            key: key.to_owned(),
            generation,
            status: status.to_owned(),
        }
    }

    const KEY: &str = "cpu:ApplicationsStorageExtension";

    #[test]
    fn no_history_creates_generation_one() {
        assert_eq!(
            decide_emission(KEY, &[]),
            Emission::Create {
                id: format!("{KEY}~1"),
                generation: 1
            }
        );
    }

    #[test]
    fn an_open_row_is_refreshed_not_duplicated() {
        let existing = vec![row(&format!("{KEY}~1"), KEY, 1, "open")];
        assert_eq!(
            decide_emission(KEY, &existing),
            Emission::Refresh {
                id: format!("{KEY}~1")
            }
        );
    }

    #[test]
    fn a_condition_that_cleared_and_returned_gets_a_new_generation() {
        let existing = vec![row(&format!("{KEY}~1"), KEY, 1, "resolved")];
        assert_eq!(
            decide_emission(KEY, &existing),
            Emission::Create {
                id: format!("{KEY}~2"),
                generation: 2
            }
        );
    }

    #[test]
    fn generations_count_from_the_highest_seen_not_the_row_count() {
        let existing = vec![row("a", KEY, 1, "resolved"), row("b", KEY, 7, "resolved")];
        assert_eq!(
            decide_emission(KEY, &existing),
            Emission::Create {
                id: format!("{KEY}~8"),
                generation: 8
            }
        );
    }

    #[test]
    fn another_conditions_history_is_not_this_conditions_history() {
        let existing = vec![row("a", "storage:free-below-threshold", 3, "open")];
        assert_eq!(
            decide_emission(KEY, &existing),
            Emission::Create {
                id: format!("{KEY}~1"),
                generation: 1
            }
        );
    }

    /// The Axon#177 bug a packed `{key}~{n}` id made possible: `cpu:Storage` is a prefix of
    /// `cpu:StorageManagementService`, so a startsWith comparison gave one condition the other's
    /// history. `key` is a column now, so this is exact — the test stays because the guarantee
    /// is what matters, not how it is obtained.
    #[test]
    fn a_key_that_prefixes_another_key_is_not_confused_with_it() {
        let existing = vec![row("cpu:Storage~4", "cpu:Storage", 4, "open")];
        assert_eq!(
            decide_emission("cpu:StorageManagementService", &existing),
            Emission::Create {
                id: "cpu:StorageManagementService~1".to_owned(),
                generation: 1
            }
        );
    }

    // ── decideResolutions ─────────────────────────────────────────────────────

    fn finding(key: &str) -> Finding {
        Finding {
            key: key.to_owned(),
            title: key.to_owned(),
            note: String::new(),
        }
    }

    #[test]
    fn a_condition_the_run_did_not_see_is_closed() {
        let existing = vec![
            row("cpu:Foo~1", "cpu:Foo", 1, "open"),
            row("cpu:Bar~1", "cpu:Bar", 1, "open"),
        ];
        assert_eq!(
            decide_resolutions(&[finding("cpu:Foo")], &existing),
            vec!["cpu:Bar~1".to_owned()]
        );
    }

    #[test]
    fn a_healthy_run_closes_everything_that_was_open() {
        let existing = vec![
            row("cpu:Foo~1", "cpu:Foo", 1, "open"),
            row("cpu:Bar~1", "cpu:Bar", 1, "open"),
        ];
        assert_eq!(
            decide_resolutions(&[], &existing),
            vec!["cpu:Foo~1".to_owned(), "cpu:Bar~1".to_owned()]
        );
    }

    #[test]
    fn an_already_resolved_row_is_not_resolved_twice() {
        let existing = vec![row("cpu:Foo~1", "cpu:Foo", 1, "resolved")];
        assert!(decide_resolutions(&[], &existing).is_empty());
    }

    // ── netFinding ────────────────────────────────────────────────────────────

    /// The payload shape `host-net check --json` emits, with the process names replaced: this
    /// file is public and which daemons a given machine runs is an overlay fact.
    fn net(policy: &str, listeners: &[(&str, &str, &str, i64)]) -> NetReport {
        let rows: Vec<String> = listeners
            .iter()
            .map(|(process, port, protos, pid)| {
                format!(
                    r#"{{"process":"{process}","port":"{port}","protos":"{protos}","pid":{pid}}}"#
                )
            })
            .collect();
        serde_json::from_str(&format!(
            r#"{{"listeners":46,"wildcard":24,"policy":"{policy}","unexpected":[{}]}}"#,
            rows.join(",")
        ))
        .expect("the host-net fixture")
    }

    #[test]
    fn a_host_whose_wildcard_listeners_are_all_declared_files_nothing() {
        assert_eq!(net_finding(Some(&net("<overlay>", &[]))), None);
    }

    #[test]
    fn host_net_could_not_run_is_no_finding_and_no_throw() {
        assert_eq!(net_finding(None), None);
    }

    /// Three listeners are one condition. The key carries no port because a mesh VPN's wildcard
    /// ports change on every restart, and a key that moves mints a new generation every hour.
    #[test]
    fn three_unexpected_listeners_are_one_finding_with_a_port_free_key() {
        let three = net_finding(Some(&net(
            "<overlay>/config/host-net-policy.toml",
            &[
                ("example-daemon", "19222", "tcp46", 22458),
                ("example-vpn-extension", "443", "tcp4+tcp6", 11481),
                ("example-vpn-extension", "41641", "udp4+udp6", 11481),
            ],
        )))
        .expect("a finding");
        assert_eq!(three.key, "net:unexpected-exposure");
        assert!(three.note.contains("example-daemon on *:19222"));
        assert!(three.note.contains("*:443"));
        assert!(three.note.contains("*:41641"));
        assert!(three
            .note
            .contains("[[expect_wildcard]] entry to <overlay>/config/host-net-policy.toml"));

        // The same three listeners after a restart, on different ephemeral ports: same key.
        let later = net_finding(Some(&net(
            "<overlay>/config/host-net-policy.toml",
            &[
                ("example-daemon", "19222", "tcp46", 31002),
                ("example-vpn-extension", "443", "tcp4+tcp6", 31111),
                ("example-vpn-extension", "50007", "udp4+udp6", 31111),
            ],
        )))
        .expect("a finding");
        assert_eq!(later.key, three.key);
    }

    #[test]
    fn the_title_names_the_distinct_processes_not_one_line_per_port() {
        let f = net_finding(Some(&net(
            "<overlay>",
            &[
                ("example-vpn-extension", "443", "tcp4", 1),
                ("example-vpn-extension", "41641", "udp4", 1),
            ],
        )))
        .expect("a finding");
        assert_eq!(
            f.title,
            "2 wildcard listener(s) not in the host-net policy (example-vpn-extension)"
        );
    }

    #[test]
    fn more_than_three_processes_are_summarised_rather_than_listed() {
        let f = net_finding(Some(&net(
            "<overlay>",
            &[
                ("a", "1", "tcp4", 1),
                ("b", "2", "tcp4", 1),
                ("c", "3", "tcp4", 1),
                ("d", "4", "tcp4", 1),
            ],
        )))
        .expect("a finding");
        assert!(f.title.ends_with("(a, b, c, …)"), "{}", f.title);
    }
}
