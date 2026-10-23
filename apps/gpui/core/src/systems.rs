//! The Systems page's numbers, and the rules that decide how they read.
//!
//! Every payload here is another owner's JSON, passed through unchanged: macmon's sample,
//! `tools/storage report --json`, `tools/updates --json`, `tools/host-watch`'s findings, and
//! `sjel_runtime::Status`. The reductions below are the native half of
//! `dashboard/src/lib/systems/{storage,updates}.ts` and `dashboard/src/lib/runtime.ts` —
//! same rules, same wording, so a reader comparing the two surfaces sees one answer.
//!
//! Nothing here draws. The page lives in `apps/gpui/src/systems.rs`; this file is pure so every
//! ordering, unit switch and "not checked is not current" rule is testable without a window.

use std::collections::BTreeMap;

use chrono::{DateTime, Local};
use serde::Deserialize;

/// One sample from macmon's `GET /json`.
///
/// Only the fields this page draws. `macmon serve` emits more (`ane_power`, `gpu_usage`,
/// `gpu_ram_power`), and a field added upstream must not break the page, so unknown keys are
/// ignored and every known one defaults.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct MacmonSample {
    #[serde(default)]
    pub all_power: f64,
    #[serde(default)]
    pub cpu_power: f64,
    #[serde(default)]
    pub gpu_power: f64,
    #[serde(default)]
    pub ram_power: f64,
    #[serde(default)]
    pub sys_power: f64,
    /// 0–1, not percent.
    #[serde(default)]
    pub cpu_usage_pct: f64,
    /// `[frequency_mhz, utilisation_0_1]`.
    #[serde(default)]
    pub pcpu_usage: [f64; 2],
    #[serde(default)]
    pub ecpu_usage: [f64; 2],
    #[serde(default)]
    pub memory: MacmonMemory,
    #[serde(default)]
    pub temp: MacmonTemperature,
    /// RFC 3339. macmon stamps the sample with the machine's clock.
    #[serde(default)]
    pub timestamp: String,
}

/// Bytes, despite the name: macmon reports memory in bytes and the dashboard divides by
/// 1024³ to print GB.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct MacmonMemory {
    #[serde(default)]
    pub ram_total: f64,
    #[serde(default)]
    pub ram_usage: f64,
    #[serde(default)]
    pub swap_total: f64,
    #[serde(default)]
    pub swap_usage: f64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct MacmonTemperature {
    #[serde(default)]
    pub cpu_temp_avg: f64,
    #[serde(default)]
    pub gpu_temp_avg: f64,
}

/// How hot a reading is. The thresholds are the dashboard's, and they are about a Mac's own
/// comfort rather than a hardware limit: 60 °C is where a fan becomes audible on Apple Silicon
/// under a sustained load, 80 °C is where a compile is throttling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Temperature {
    Cool,
    Warm,
    Hot,
}

pub fn temperature(celsius: f64) -> Temperature {
    if celsius >= 80.0 {
        Temperature::Hot
    } else if celsius >= 60.0 {
        Temperature::Warm
    } else {
        Temperature::Cool
    }
}

impl MacmonSample {
    /// The share of RAM in use, 0–1, or 0 when mach answered no total.
    pub fn ram_ratio(&self) -> f32 {
        ratio(self.memory.ram_usage, self.memory.ram_total)
    }

    /// The share of swap in use, 0–1. Zero swap configured is zero, not undefined.
    pub fn swap_ratio(&self) -> f32 {
        ratio(self.memory.swap_usage, self.memory.swap_total)
    }

    /// Swap past half is what macOS does when the working set no longer fits; below it, a page
    /// moved to disk is bookkeeping rather than pressure.
    pub fn under_pressure(&self) -> bool {
        self.memory.swap_total > 0.0 && self.memory.swap_usage > self.memory.swap_total * 0.5
    }
}

fn ratio(part: f64, whole: f64) -> f32 {
    if whole <= 0.0 {
        return 0.0;
    }
    (part / whole).clamp(0.0, 1.0) as f32
}

/// Bytes in the unit macmon's fields are in: gigabytes, one decimal, the dashboard's format.
pub fn gigabytes(bytes: f64) -> String {
    format!("{:.1} GB", bytes / 1024.0 / 1024.0 / 1024.0)
}

pub fn watts(value: f64) -> String {
    format!("{value:.2} W")
}

/// A 0–1 fraction as a percentage with one decimal.
pub fn percent(fraction: f64) -> String {
    format!("{:.1}%", fraction * 100.0)
}

/// The sample's clock time in the reader's own zone. An unparseable stamp prints as itself:
/// the stamp is the evidence, and `Invalid Date` would hide it.
pub fn clock_label(stamp: &str) -> String {
    DateTime::parse_from_rfc3339(stamp).map_or_else(
        |_| stamp.to_owned(),
        |at| at.with_timezone(&Local).format("%H:%M:%S").to_string(),
    )
}

// ── storage ──────────────────────────────────────────────────────────────────

/// `GET /api/sjel-status/storage`, which is `tools/storage report --json` verbatim.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct StorageReport {
    #[serde(default)]
    pub disk: Disk,
    /// `ok` · `warn` · `critical`. The volume's state, never a class being large.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub classes: Vec<StorageClass>,
    #[serde(default)]
    pub protected: Vec<StorageProtected>,
    #[serde(default)]
    pub expected_service: Vec<ExpectedService>,
    /// Absent from a report an older build produced.
    #[serde(default)]
    pub named_bytes: Option<u64>,
    #[serde(default)]
    pub unattributed_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct Disk {
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub used: u64,
    #[serde(default)]
    pub free: u64,
}

/// One reclaimable class in the policy.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct StorageClass {
    pub name: String,
    #[serde(default)]
    pub bytes: u64,
    /// Whether the policy lets `apply` reclaim this class.
    #[serde(default)]
    pub applicable: bool,
    /// Over the policy's `class_flag_gb`. Loud on this machine, not necessarily wrong.
    #[serde(default)]
    pub flagged: bool,
    /// False when `du` answered partially or not at all, so `bytes` is a lower bound.
    #[serde(default)]
    pub measured: Option<bool>,
    /// False when nothing on the machine answers to the class's paths.
    #[serde(default)]
    pub present: Option<bool>,
}

/// A path the policy reports and never reclaims, with the reason it is safe to leave.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct StorageProtected {
    pub path: String,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub present: Option<bool>,
}

/// A service the policy declares, and the machine's answer.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ExpectedService {
    pub name: String,
    #[serde(default)]
    pub kind: String,
    /// `running` · `absent` · `unverifiable`.
    #[serde(default)]
    pub presence: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// `ok` · `warn` · `critical`. A fourth value the policy grows reads as `ok`: a build that
/// does not know it must not paint a healthy disk red, and the sizes still say what is true.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Severity {
    #[default]
    Ok,
    Warn,
    Critical,
}

pub fn severity(state: &str) -> Severity {
    match state {
        "critical" => Severity::Critical,
        "warn" => Severity::Warn,
        _ => Severity::Ok,
    }
}

/// Bytes in the units the storage CLI prints: MB below 1 GB, GB above it.
///
/// This is `fmt_bytes` in `tools/storage/src/measure.rs`, and the switch is the same on
/// purpose: a reader comparing a class here against `sjel storage report` in a terminal has to
/// see one number, not two that differ by a rounding rule.
pub fn storage_bytes(bytes: u64) -> String {
    const GB: u64 = 1024 * 1024 * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else {
        format!("{} MB", bytes / (1024 * 1024))
    }
}

/// The report, reduced to what the page renders and in the order it renders it.
#[derive(Clone, Debug, Default)]
pub struct StorageView {
    pub used_pct: f32,
    pub used: String,
    pub total: String,
    pub free: String,
    pub severity: Severity,
    /// The tool's own word, shown verbatim so an unknown state still reads as what it said.
    pub state_label: String,
    /// Measured classes, largest first.
    pub classes: Vec<StorageClass>,
    /// What the class bars are a fraction of: the largest class `du` read completely.
    ///
    /// Not simply `classes[0]`. A class `du` could only read part of reports a lower bound that
    /// can exceed everything real — this machine's `system-state` reads 874 GB that way — and
    /// scaling every other row against it draws them all as one-pixel slivers, which says
    /// "nothing here is big" about the rows that are.
    pub scale_bytes: u64,
    pub reclaimable_label: String,
    /// Every protected path, largest first — kept at 0 bytes too, because the reason is the point.
    pub protected: Vec<StorageProtected>,
    pub protected_absent: usize,
    pub protected_absent_paths: Vec<String>,
    pub protected_unreadable: usize,
    /// Classes the tool could not measure at all: the path is here and `du` returned nothing.
    pub unmeasured: Vec<StorageClass>,
    /// Classes whose paths no longer exist; they carry no size, so they are named instead.
    pub classes_absent: Vec<StorageClass>,
    /// Declared services the machine answers `absent` for. `unverifiable` is not here: that is
    /// the probe refusing to answer, which the tool keeps apart and so does this.
    pub services_absent: Vec<String>,
    /// `(named, unattributed)` as labels, or none for a report from before the fields existed.
    pub reconciliation: Option<(String, String)>,
}

impl StorageReport {
    pub fn view(&self) -> StorageView {
        let sized: Vec<StorageClass> = self
            .classes
            .iter()
            .filter(|class| class.bytes > 0)
            .cloned()
            .collect();
        let mut classes = sized;
        classes.sort_by_key(|class| std::cmp::Reverse(class.bytes));

        // `present == false` is the tool's own answer that the path is gone, and it is checked
        // before `measured`: an absent class measures "completely", there being nothing to fail on.
        let mut classes_absent: Vec<StorageClass> = self
            .classes
            .iter()
            .filter(|class| class.present == Some(false))
            .cloned()
            .collect();
        classes_absent.sort_by(|a, b| a.name.cmp(&b.name));

        let mut unmeasured: Vec<StorageClass> = self
            .classes
            .iter()
            .filter(|class| {
                class.present != Some(false) && class.measured == Some(false) && class.bytes == 0
            })
            .cloned()
            .collect();
        unmeasured.sort_by(|a, b| a.name.cmp(&b.name));

        let mut protected = self.protected.clone();
        protected.sort_by_key(|entry| std::cmp::Reverse(entry.bytes));

        let reclaimable: u64 = classes
            .iter()
            .filter(|class| class.applicable)
            .map(|class| class.bytes)
            .sum();

        let scale_bytes = classes
            .iter()
            .filter(|class| class.measured != Some(false))
            .map(|class| class.bytes)
            .max()
            .or_else(|| classes.first().map(|class| class.bytes))
            .unwrap_or(0)
            .max(1);

        let mut services_absent: Vec<String> = self
            .expected_service
            .iter()
            .filter(|service| service.presence.as_deref() == Some("absent"))
            .map(|service| service.name.clone())
            .collect();
        services_absent.sort();

        StorageView {
            used_pct: if self.disk.total > 0 {
                (self.disk.used as f64 / self.disk.total as f64) as f32
            } else {
                0.0
            },
            used: storage_bytes(self.disk.used),
            total: storage_bytes(self.disk.total),
            free: storage_bytes(self.disk.free),
            severity: severity(&self.state),
            state_label: self.state.clone(),
            classes,
            scale_bytes,
            reclaimable_label: storage_bytes(reclaimable),
            protected_absent: protected
                .iter()
                .filter(|entry| entry.present == Some(false))
                .count(),
            protected_absent_paths: protected
                .iter()
                .filter(|entry| entry.present == Some(false))
                .map(|entry| entry.path.clone())
                .collect(),
            // A 0-byte path that is not known-absent: a live but empty directory, or one the
            // measuring user cannot read. The tool does not distinguish those two either.
            protected_unreadable: protected
                .iter()
                .filter(|entry| entry.bytes == 0 && entry.present != Some(false))
                .count(),
            protected,
            unmeasured,
            classes_absent,
            services_absent,
            reconciliation: match (self.named_bytes, self.unattributed_bytes) {
                (Some(named), Some(unattributed)) => {
                    Some((storage_bytes(named), storage_bytes(unattributed)))
                }
                _ => None,
            },
        }
    }
}

/// `GET /api/sjel-status/storage-history`: the disk's own hourly readings.
///
/// `samples` empty is "no history", not an error — a machine that has not yet completed an hour
/// under sjel-status has none.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct StorageHistory {
    #[serde(default)]
    pub retention_days: u32,
    #[serde(default)]
    pub samples: Vec<StorageSample>,
    #[serde(default)]
    pub absent: Vec<StorageAbsence>,
    #[serde(default)]
    pub classes: Vec<StorageClassSample>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct StorageSample {
    /// The hour this reading belongs to, canonical UTC.
    pub hour: String,
    #[serde(default)]
    pub used: i64,
    #[serde(default)]
    pub unattributed: i64,
}

/// One name the machine no longer answers for, and since when.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct StorageAbsence {
    /// `class` · `protected` · `service`; the three lists a name can come from.
    pub kind: String,
    pub name: String,
    /// The oldest hour of the still-open run, canonical UTC.
    pub since: String,
    /// True when the run reaches the oldest kept sample, so `since` is a floor, not the hour.
    #[serde(default)]
    pub capped: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StorageClassSample {
    pub name: String,
    #[serde(default)]
    pub bytes: i64,
}

/// How many hourly samples a trend reads: one week.
pub const TREND_WINDOW_HOURS: usize = 168;

/// The disk's own history, reduced to a slope and the bar heights that draw it.
#[derive(Clone, Debug, Default)]
pub struct StorageTrend {
    /// True once the window holds two readings, so a line and a slope mean anything.
    pub enough: bool,
    pub delta_label: String,
    pub unattributed_delta_label: String,
    /// How much time the window actually covers, e.g. `7d` or `3h`.
    pub span_label: String,
    /// One bar per sample, 0–1 of the window's own range, oldest first.
    pub bars: Vec<f32>,
}

impl StorageHistory {
    pub fn trend(&self) -> Option<StorageTrend> {
        let window = self.samples.len().min(TREND_WINDOW_HOURS);
        let samples = self
            .samples
            .get(self.samples.len().saturating_sub(window)..)?;
        let first = samples.first()?;
        let last = samples.last()?;
        let enough = samples.len() >= 2;
        let values: Vec<i64> = samples.iter().map(|sample| sample.used).collect();
        Some(StorageTrend {
            enough,
            delta_label: signed_storage_bytes(last.used - first.used),
            unattributed_delta_label: signed_storage_bytes(last.unattributed - first.unattributed),
            // The window's real span, not its sample count: an hour the machine was off leaves
            // a gap, and a count would quietly claim the bar chart covers less time than it does.
            span_label: span_label(hours_between(&first.hour, &last.hour)),
            bars: bars(&values),
        })
    }

    /// Per-class growth over the window, by class name. A report says what fills the disk now;
    /// only the history says whether a class is still filling.
    pub fn class_deltas(&self) -> BTreeMap<String, String> {
        let mut by_name: BTreeMap<String, Vec<i64>> = BTreeMap::new();
        for entry in &self.classes {
            by_name
                .entry(entry.name.clone())
                .or_default()
                .push(entry.bytes);
        }
        by_name
            .into_iter()
            .filter_map(|(name, values)| {
                if values.len() < 2 {
                    return None;
                }
                let window = values.len().min(TREND_WINDOW_HOURS);
                let windowed = &values[values.len() - window..];
                let delta = windowed.last()? - windowed.first()?;
                Some((name, signed_storage_bytes(delta)))
            })
            .collect()
    }
}

/// `+12.3 GB`, `-1.0 GB`, `±0 MB`. ASCII signs, because the label sits beside a byte count in
/// a column and a typographic minus would change the column's width.
pub fn signed_storage_bytes(delta: i64) -> String {
    let size = storage_bytes(delta.unsigned_abs());
    if size == "0 MB" {
        return "±0 MB".to_owned();
    }
    format!("{}{size}", if delta > 0 { "+" } else { "-" })
}

/// Normalizes values to bar heights in 0–1. A flat series draws as a row of half-height bars
/// rather than as nothing, so the chart says "no change" instead of looking empty.
fn bars(values: &[i64]) -> Vec<f32> {
    let min = values.iter().copied().min().unwrap_or(0);
    let max = values.iter().copied().max().unwrap_or(0);
    let range = max - min;
    values
        .iter()
        .map(|value| match range {
            0 => 0.5,
            range => {
                // A quarter to one: the bottom of the range is still visible.
                let share = (*value - min) as f32 / range as f32;
                0.25 + share * 0.75
            }
        })
        .collect()
}

fn hours_between(a: &str, b: &str) -> i64 {
    let parse = |stamp: &str| -> Option<DateTime<Local>> {
        // The canonical stamp is `2026-10-12 21:00:00.000+00:00`; RFC 3339 wants a `T`.
        DateTime::parse_from_rfc3339(&stamp.replacen(' ', "T", 1))
            .ok()
            .map(|at| at.with_timezone(&Local))
    };
    match (parse(a), parse(b)) {
        (Some(from), Some(to)) => (to - from).num_hours().max(0),
        _ => 0,
    }
}

fn span_label(hours: i64) -> String {
    if hours < 24 {
        format!("{hours}h")
    } else {
        format!("{}d", hours / 24)
    }
}

/// `2026-10-12 21:00:00.000+00:00` → `12 Oct`.
///
/// Sliced out of the stamp rather than converted to the reader's zone on purpose: the stamp is
/// canonical UTC, and a local rendering would show a day earlier or later than the hour the
/// sample is keyed on.
pub fn day_label(stamp: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let bytes = stamp.as_bytes();
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return stamp.to_owned();
    }
    let (Ok(month), Ok(day)) = (stamp[5..7].parse::<usize>(), stamp[8..10].parse::<usize>()) else {
        return stamp.to_owned();
    };
    match MONTHS.get(month.wrapping_sub(1)) {
        Some(name) if (1..=31).contains(&day) => format!("{day} {name}"),
        _ => stamp.to_owned(),
    }
}

/// The report is a point and the history is a line: the report says a class's path is not
/// there, and only the history says since when. The join is by name and kind, and a name the
/// history does not answer for prints bare — a guessed date would be worse than none.
pub fn dated_absences(
    names: &[String],
    kind: &str,
    history: Option<&StorageHistory>,
) -> Vec<String> {
    names
        .iter()
        .map(|name| {
            let found = history.and_then(|history| {
                history
                    .absent
                    .iter()
                    .find(|entry| entry.kind == kind && &entry.name == name)
            });
            match found {
                None => name.clone(),
                Some(row) => format!(
                    "{name} (absent since {}{})",
                    day_label(&row.since),
                    if row.capped { " or earlier" } else { "" }
                ),
            }
        })
        .collect()
}

// ── updates ──────────────────────────────────────────────────────────────────

/// `GET /api/sjel-status/updates`, which is `tools/updates --json` verbatim.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct UpdatesReport {
    #[serde(default)]
    pub offline: bool,
    #[serde(rename = "lastApply", default)]
    pub last_apply: Option<ApplyReceipt>,
    #[serde(default)]
    pub surfaces: Vec<UpdateSurface>,
    #[serde(default)]
    pub rows: Vec<UpdateRow>,
}

/// Who owns moving one class of installed software: `scheduled` a capability moves it, `manual`
/// a verb exists and nothing schedules it, `unowned` nothing moves it, `self` the vendor does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Owner {
    Scheduled,
    Manual,
    Unowned,
    SelfManaged,
}

impl Owner {
    pub const ORDER: [Self; 4] = [
        Self::Scheduled,
        Self::Manual,
        Self::Unowned,
        Self::SelfManaged,
    ];

    fn parse(value: &str) -> Self {
        match value {
            "scheduled" => Self::Scheduled,
            "manual" => Self::Manual,
            "self" => Self::SelfManaged,
            _ => Self::Unowned,
        }
    }
}

/// One class as `tools/sjel-cli/src/updates/report.rs`'s `SURFACES` declares it, sent with the
/// report so the page renders the tool's reasoning rather than a second copy of the table.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct UpdateSurface {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    owner: String,
    #[serde(rename = "ownerDetail", default)]
    pub owner_detail: String,
    /// Whether `apply` moves this class. A `self` class is never actionable.
    #[serde(default)]
    pub actionable: bool,
    #[serde(default)]
    pub why: String,
}

impl UpdateSurface {
    pub fn owner(&self) -> Owner {
        Owner::parse(&self.owner)
    }
}

/// `current` · `stale` · `unknown` · `n/a`. A status this build does not know reads as
/// `unknown`, never as `current`: a tool that grew a fifth status must not render as a clean
/// bill of health.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Current,
    Stale,
    Unknown,
    NotApplicable,
}

/// One row: a package, a crate, an integration, or a class's own receipt line.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct UpdateRow {
    pub surface: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    status: String,
    // `owner` and `ownerDetail` are on each row too; the surface above carries them once, and a
    // second copy per row would be a second place for the page to disagree with itself.
    #[serde(default)]
    pub installed: Option<String>,
    #[serde(default)]
    pub latest: Option<String>,
    /// The command that moves it. Present only on rows the tool can act on.
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

impl UpdateRow {
    pub fn status(&self) -> UpdateStatus {
        match self.status.as_str() {
            "current" => UpdateStatus::Current,
            "stale" => UpdateStatus::Stale,
            "n/a" => UpdateStatus::NotApplicable,
            _ => UpdateStatus::Unknown,
        }
    }
}

/// What the last `apply` recorded. The apply route answers `202` before the work starts, so the
/// report's own receipt is the only progress there is.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ApplyReceipt {
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub steps: Option<u64>,
    #[serde(default)]
    pub failed: Option<u64>,
    #[serde(rename = "stillStale", default)]
    pub still_stale: Option<u64>,
    #[serde(default)]
    pub state: Option<String>,
    /// `tools/audit`'s verdict taken after the steps. A string rather than an enum, so a fifth
    /// verdict the tool grows is shown rather than dropped.
    #[serde(default)]
    pub audit: Option<String>,
}

/// One class with its rows, ready to render.
#[derive(Clone, Debug, Default)]
pub struct UpdateGroup {
    pub surface: UpdateSurface,
    pub rows: Vec<UpdateRow>,
    /// Rows the tool can act on now. Zero means no button, whatever the owner says.
    pub actionable: usize,
}

/// The report, reduced to what the page renders.
#[derive(Clone, Debug, Default)]
pub struct UpdatesView {
    pub groups: Vec<UpdateGroup>,
    pub stale: usize,
    /// Rows the tool could not check — `--offline`, or a registry that did not answer. Counted
    /// apart from `stale` on purpose: "not checked" is not "current".
    pub unknown: usize,
    pub last_apply: Option<ApplyReceipt>,
    /// True while an apply runs: the page disables the buttons rather than let a second one race
    /// the first over the same package manager.
    pub busy: bool,
}

impl UpdatesReport {
    pub fn view(&self) -> UpdatesView {
        let mut groups = Vec::new();
        for owner in Owner::ORDER {
            for surface in self
                .surfaces
                .iter()
                .filter(|surface| surface.owner() == owner)
            {
                let rows: Vec<UpdateRow> = self
                    .rows
                    .iter()
                    .filter(|row| row.surface == surface.id)
                    .cloned()
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                // `action` and not just `stale`: a package another global pins is stale and
                // deliberately not actionable, and a button counting it would promise work the
                // tool refuses to do. The count is what the button says, so it has to agree.
                let actionable = if surface.actionable {
                    rows.iter()
                        .filter(|row| row.status() == UpdateStatus::Stale && row.action.is_some())
                        .count()
                } else {
                    0
                };
                groups.push(UpdateGroup {
                    surface: surface.clone(),
                    rows,
                    actionable,
                });
            }
        }

        let stale = self
            .rows
            .iter()
            .filter(|row| row.status() == UpdateStatus::Stale)
            .count();
        let unknown = self
            .rows
            .iter()
            .filter(|row| row.status() == UpdateStatus::Unknown)
            .count();
        let busy = self
            .last_apply
            .as_ref()
            .and_then(|receipt| receipt.state.as_deref())
            == Some("running");
        UpdatesView {
            groups,
            stale,
            unknown,
            last_apply: self.last_apply.clone(),
            busy,
        }
    }
}

/// The version cell: `0.7.0 → 0.8.2` when both are known, one version otherwise.
pub fn version_label(row: &UpdateRow) -> String {
    let Some(installed) = row.installed.as_deref() else {
        return row.latest.clone().unwrap_or_default();
    };
    match row.latest.as_deref() {
        Some(latest) if latest != installed => format!("{installed} → {latest}"),
        _ => installed.to_owned(),
    }
}

/// The line under the buttons: what the last run did, or what it is doing.
pub fn apply_summary(view: &UpdatesView, now: DateTime<Local>) -> String {
    let Some(last) = &view.last_apply else {
        return String::new();
    };
    let class = last.class.clone().unwrap_or_default();
    let when = last
        .at
        .as_deref()
        .map(|at| since_label(at, now))
        .unwrap_or_default();
    if last.state.as_deref() == Some("running") {
        return format!("{class} — applying, started {when}");
    }
    let failed = last.failed.unwrap_or(0);
    let steps = last.steps.unwrap_or(0);
    let still = last.still_stale.unwrap_or(0);
    let mut line = if failed > 0 {
        format!("{class} — {failed} of {steps} failed")
    } else {
        format!(
            "{class} — {steps} step{} applied",
            if steps == 1 { "" } else { "s" }
        )
    };
    if still > 0 {
        line.push_str(&format!(", {still} still stale"));
    }
    line.push_str(&audit_label(last.audit.as_deref()));
    if !when.is_empty() {
        line.push_str(&format!(" · {when}"));
    }
    line
}

/// The audit verdict as the summary says it, or nothing when the receipt predates the field.
/// Every verdict is shown, `clean` included: the point of the line is to prove the audit ran at
/// all, so a silent clean would make a missing audit and a passing one look identical.
pub fn audit_label(verdict: Option<&str>) -> String {
    match verdict {
        None => String::new(),
        Some("clean") => " · audit clean".to_owned(),
        Some(verdict) => format!(" · audit {verdict} — run tools/audit"),
    }
}

/// True when the last apply's audit found something or could not run.
pub fn audit_needs_attention(verdict: Option<&str>) -> bool {
    matches!(verdict, Some(verdict) if verdict != "clean")
}

fn since_label(stamp: &str, now: DateTime<Local>) -> String {
    let Ok(at) = DateTime::parse_from_rfc3339(stamp) else {
        return String::new();
    };
    let mins = (now - at.with_timezone(&Local)).num_minutes().max(0);
    match mins {
        0 => "just now".to_owned(),
        m if m < 60 => format!("{m}m ago"),
        m if m < 60 * 24 => format!("{}h ago", m / 60),
        m => format!("{}d ago", m / (60 * 24)),
    }
}

// ── host watch ───────────────────────────────────────────────────────────────

/// `GET /api/sjel-status/host-watch`: the hourly watch's open findings.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Findings {
    #[serde(default)]
    pub findings: Vec<Finding>,
}

/// One condition the hourly watch still sees. `key` names the command, never the pid, so a
/// reboot does not look like a new problem.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Finding {
    pub key: String,
    #[serde(default)]
    pub title: String,
    /// What to run to look at it, and what to run if it is stuck. Multi-line.
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub first_seen: String,
}

// ── runtime profile ──────────────────────────────────────────────────────────

/// `GET /api/sjel-status/runtime` (`sjel_runtime::Status`).
#[derive(Clone, Debug, Deserialize)]
pub struct RuntimeStatus {
    #[serde(default)]
    pub device: String,
    /// False until a selection has been saved; auto is then the profile in force.
    #[serde(default)]
    pub configured: bool,
    /// `auto` · `normal` · `on-the-go`.
    #[serde(default = "auto_selection")]
    pub selection: String,
    #[serde(default = "normal_selection")]
    pub effective: String,
    /// `ac` · `battery` · `no-battery` · `unknown`.
    #[serde(default = "unknown_power")]
    pub power: String,
    #[serde(default)]
    pub power_fresh: bool,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub detail: Option<String>,
}

impl Default for RuntimeStatus {
    fn default() -> Self {
        Self {
            device: String::new(),
            configured: false,
            selection: auto_selection(),
            effective: normal_selection(),
            power: unknown_power(),
            power_fresh: false,
            allow: Vec::new(),
            revision: 0,
            detail: None,
        }
    }
}

// The Rust type's own defaults (`Selection::Auto`, `Selection::Normal`, `Power::Unknown`) are
// what an omitted field means, so the deserializer has to agree with them.
fn auto_selection() -> String {
    "auto".to_owned()
}

fn normal_selection() -> String {
    "normal".to_owned()
}

fn unknown_power() -> String {
    "unknown".to_owned()
}

/// The three selections, in the order a reader meets them.
pub const RUNTIME_SELECTIONS: [(&str, &str); 3] = [
    ("auto", "Auto"),
    ("normal", "Normal"),
    ("on-the-go", "On the go"),
];

/// The categories a saved profile can except while On the go.
pub const RUNTIME_CATEGORIES: [(&str, &str); 5] = [
    ("other-local-models", "Other local models"),
    ("remote-models", "Remote models"),
    ("bulk-indexing", "Bulk indexing"),
    ("transcription", "Transcription"),
    ("media-conversion", "Media conversion"),
];

pub fn selection_label(selection: &str) -> &'static str {
    RUNTIME_SELECTIONS
        .iter()
        .find(|(id, _)| *id == selection)
        .map_or("Auto", |(_, label)| label)
}

pub fn category_label(category: &str) -> &'static str {
    RUNTIME_CATEGORIES
        .iter()
        .find(|(id, _)| *id == category)
        .map_or("Unknown category", |(_, label)| label)
}

pub fn power_label(power: &str) -> &'static str {
    match power {
        "ac" => "AC",
        "battery" => "Battery",
        "no-battery" => "No battery",
        _ => "Power unknown",
    }
}

/// The profile as one line: what is in force, why, and what it admits.
#[derive(Clone, Debug, Default)]
pub struct RuntimeView {
    /// The profile in force, e.g. `On the go`.
    pub label: &'static str,
    /// `Not configured`, `Auto · AC` or `Manual`, with a stale power observation named.
    pub source: String,
    pub on_the_go: bool,
    /// On the go with neither local nor remote models allowed: Apple's own model only.
    pub afm_only: bool,
    pub exceptions: Vec<&'static str>,
}

impl RuntimeView {
    pub fn summary(&self) -> String {
        let exceptions = if self.exceptions.is_empty() {
            "No active exceptions".to_owned()
        } else {
            format!("Allowed: {}", self.exceptions.join(", "))
        };
        let afm = if self.afm_only { " · AFM only" } else { "" };
        format!("{} · {}{afm} · {exceptions}", self.label, self.source)
    }
}

impl RuntimeStatus {
    pub fn view(&self) -> RuntimeView {
        let on_the_go = self.effective == "on-the-go";
        let exceptions: Vec<&'static str> = if on_the_go {
            self.allow
                .iter()
                .map(|category| category_label(category))
                .collect()
        } else {
            Vec::new()
        };
        let afm_only = on_the_go
            && !self.allow.iter().any(|category| {
                matches!(category.as_str(), "other-local-models" | "remote-models")
            });
        let source = if !self.configured {
            "Not configured".to_owned()
        } else if self.selection == "auto" {
            format!(
                "Auto · {}{}",
                power_label(&self.power),
                if self.power_fresh { "" } else { " (stale)" }
            )
        } else {
            "Manual".to_owned()
        };
        RuntimeView {
            label: selection_label(&self.effective),
            source,
            on_the_go,
            afm_only,
            exceptions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(stamp: &str) -> DateTime<Local> {
        DateTime::parse_from_rfc3339(stamp)
            .expect("a fixed stamp")
            .with_timezone(&Local)
    }

    /// A real `macmon serve` sample, trimmed to the fields this page reads. The field names are
    /// the machine's own, not a shape invented here: a rename upstream has to fail this test
    /// rather than quietly empty the panel.
    const MACMON_SAMPLE: &str = r#"{
        "all_power": 1.0040725469589233,
        "ane_power": 0.0,
        "cpu_power": 0.9918359518051147,
        "cpu_usage_pct": 0.06693775951862335,
        "ecpu_usage": [1898, 0.16085276007652283],
        "gpu_power": 0.012236538343131542,
        "gpu_ram_power": 0.0,
        "gpu_usage": [338, 0.0],
        "memory": {"ram_total": 25769803776, "ram_usage": 18336169984,
                   "swap_total": 4294967296, "swap_usage": 3279814656},
        "pcpu_usage": [2575, 0.019980259239673615],
        "ram_power": 0.0,
        "sys_power": 9.936366081237793,
        "temp": {"cpu_temp_avg": 46.08387756347656, "gpu_temp_avg": 44.22336959838867},
        "timestamp": "2026-10-23T07:54:05.745853+00:00"
    }"#;

    #[test]
    fn a_live_macmon_sample_reads_as_the_panel_draws_it() {
        let sample: MacmonSample =
            serde_json::from_str(MACMON_SAMPLE).expect("the machine's own sample parses");
        assert_eq!(watts(sample.all_power), "1.00 W");
        assert_eq!(percent(sample.cpu_usage_pct), "6.7%");
        assert_eq!(gigabytes(sample.memory.ram_usage), "17.1 GB");
        assert_eq!(temperature(sample.temp.cpu_temp_avg), Temperature::Cool);
        assert!(sample.under_pressure(), "3.1 of 4.0 GB end in swap");
        assert!(
            (sample.ram_ratio() - 0.7115).abs() < 0.001,
            "a ratio, not a percentage"
        );
        assert_eq!(
            clock_label(&sample.timestamp).matches(':').count(),
            2,
            "a clock time, not the raw stamp: {}",
            clock_label(&sample.timestamp)
        );
    }

    #[test]
    fn a_temperature_reads_by_its_own_thresholds() {
        assert_eq!(temperature(41.5), Temperature::Cool);
        assert_eq!(temperature(60.0), Temperature::Warm);
        assert_eq!(temperature(79.9), Temperature::Warm);
        assert_eq!(temperature(80.0), Temperature::Hot);
    }

    #[test]
    fn macmon_bytes_print_as_gigabytes_and_watts_carry_two_decimals() {
        assert_eq!(gigabytes(34_359_738_368.0), "32.0 GB");
        assert_eq!(watts(4.5), "4.50 W");
        assert_eq!(percent(0.1234), "12.3%");
    }

    #[test]
    fn a_zero_total_never_divides_by_it() {
        let sample = MacmonSample::default();
        assert_eq!(sample.ram_ratio(), 0.0);
        assert_eq!(sample.swap_ratio(), 0.0);
        assert!(!sample.under_pressure(), "no swap is not pressure");
    }

    #[test]
    fn storage_bytes_switches_unit_exactly_where_the_tool_does() {
        assert_eq!(storage_bytes(0), "0 MB");
        assert_eq!(storage_bytes(1024 * 1024 * 1024 - 1), "1023 MB");
        assert_eq!(storage_bytes(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(signed_storage_bytes(0), "±0 MB");
        assert_eq!(signed_storage_bytes(3_221_225_472), "+3.0 GB");
        assert_eq!(signed_storage_bytes(-1_073_741_824), "-1.0 GB");
    }

    #[test]
    fn an_unknown_volume_state_reads_as_ok_rather_than_red() {
        assert_eq!(severity("critical"), Severity::Critical);
        assert_eq!(severity("warn"), Severity::Warn);
        assert_eq!(severity("ok"), Severity::Ok);
        assert_eq!(severity("something-new"), Severity::Ok);
    }

    fn class(name: &str, bytes: u64) -> StorageClass {
        StorageClass {
            name: name.to_owned(),
            bytes,
            applicable: true,
            ..StorageClass::default()
        }
    }

    #[test]
    fn the_view_sorts_classes_by_size_and_keeps_absent_ones_out_of_the_sized_list() {
        const MB: u64 = 1024 * 1024;
        let report = StorageReport {
            disk: Disk {
                total: 1000 * MB,
                used: 250 * MB,
                free: 750 * MB,
            },
            state: "warn".to_owned(),
            classes: vec![
                class("small", 10 * MB),
                StorageClass {
                    present: Some(false),
                    ..class("gone", 0)
                },
                class("big", 900 * MB),
                StorageClass {
                    measured: Some(false),
                    ..class("unreadable", 0)
                },
            ],
            ..StorageReport::default()
        };
        let view = report.view();
        let names: Vec<&str> = view.classes.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["big", "small"], "largest first, sized rows only");
        assert_eq!(view.classes_absent.len(), 1);
        assert_eq!(view.classes_absent[0].name, "gone");
        assert_eq!(view.unmeasured.len(), 1);
        assert_eq!(view.unmeasured[0].name, "unreadable");
        assert_eq!(view.used_pct, 0.25);
        assert_eq!(view.reclaimable_label, "910 MB");
        assert_eq!(
            view.scale_bytes,
            900 * MB,
            "the partial 0-byte class does not set the scale"
        );
        assert_eq!(view.severity, Severity::Warn);
        assert_eq!(view.state_label, "warn", "the tool's own word, verbatim");
        assert!(
            view.reconciliation.is_none(),
            "an older report omits the fields, so the line is omitted"
        );
    }

    #[test]
    fn a_protected_path_is_the_point_of_its_row_even_at_zero_bytes() {
        let report = StorageReport {
            protected: vec![
                StorageProtected {
                    path: "/gone".to_owned(),
                    bytes: 0,
                    reason: "kept".to_owned(),
                    present: Some(false),
                },
                StorageProtected {
                    path: "/empty".to_owned(),
                    bytes: 0,
                    reason: "kept".to_owned(),
                    present: Some(true),
                },
            ],
            ..StorageReport::default()
        };
        let view = report.view();
        assert_eq!(view.protected.len(), 2);
        assert_eq!(view.protected_absent, 1);
        assert_eq!(view.protected_absent_paths, ["/gone"]);
        assert_eq!(view.protected_unreadable, 1);
    }

    #[test]
    fn only_an_absent_service_is_named_and_unverifiable_is_not() {
        let report = StorageReport {
            expected_service: vec![
                ExpectedService {
                    name: "comms".to_owned(),
                    presence: Some("running".to_owned()),
                    ..ExpectedService::default()
                },
                ExpectedService {
                    name: "soundscape".to_owned(),
                    presence: Some("absent".to_owned()),
                    ..ExpectedService::default()
                },
                ExpectedService {
                    name: "probe".to_owned(),
                    presence: Some("unverifiable".to_owned()),
                    ..ExpectedService::default()
                },
            ],
            ..StorageReport::default()
        };
        assert_eq!(report.view().services_absent, ["soundscape"]);
    }

    fn history(samples: &[(&str, i64)], absent: Vec<StorageAbsence>) -> StorageHistory {
        StorageHistory {
            retention_days: 30,
            samples: samples
                .iter()
                .map(|(hour, used)| StorageSample {
                    hour: (*hour).to_owned(),
                    used: *used,
                    unattributed: 0,
                })
                .collect(),
            absent,
            classes: Vec::new(),
        }
    }

    #[test]
    fn one_sample_is_a_point_and_not_a_trend() {
        let one = history(&[("2026-10-12 21:00:00.000+00:00", 10)], Vec::new());
        let trend = one.trend().expect("a sample exists");
        assert!(!trend.enough);
        assert_eq!(trend.span_label, "0h");
        assert!(history(&[], Vec::new()).trend().is_none());
    }

    #[test]
    fn a_trend_reads_its_span_from_the_stamps_not_the_sample_count() {
        let gb: i64 = 1024 * 1024 * 1024;
        let rows = [
            ("2026-10-12 21:00:00.000+00:00", gb),
            ("2026-10-12 23:00:00.000+00:00", 2 * gb),
            ("2026-10-19 21:00:00.000+00:00", 4 * gb),
        ];
        let trend = history(&rows, Vec::new()).trend().expect("three samples");
        assert!(trend.enough);
        assert_eq!(trend.delta_label, "+3.0 GB");
        assert_eq!(trend.span_label, "7d");
        assert_eq!(trend.bars.len(), 3);
        assert_eq!(trend.bars[0], 0.25, "the window's smallest bar still shows");
        assert_eq!(trend.bars[2], 1.0);
    }

    #[test]
    fn a_flat_series_draws_half_bars_rather_than_nothing() {
        let rows = [
            ("2026-10-12 21:00:00.000+00:00", 500),
            ("2026-10-12 22:00:00.000+00:00", 500),
        ];
        let trend = history(&rows, Vec::new()).trend().expect("two samples");
        assert_eq!(trend.bars, [0.5, 0.5]);
        assert_eq!(trend.delta_label, "±0 MB");
    }

    #[test]
    fn an_absence_is_dated_only_by_the_history_that_knows_it() {
        let history = history(
            &[
                ("2026-10-12 21:00:00.000+00:00", 1),
                ("2026-10-12 22:00:00.000+00:00", 1),
            ],
            vec![
                StorageAbsence {
                    kind: "class".to_owned(),
                    name: "chrome-models".to_owned(),
                    since: "2026-10-12 21:00:00.000+00:00".to_owned(),
                    capped: false,
                },
                StorageAbsence {
                    kind: "protected".to_owned(),
                    name: "/old".to_owned(),
                    since: "2026-10-01 21:00:00.000+00:00".to_owned(),
                    capped: true,
                },
            ],
        );
        let names = vec!["chrome-models".to_owned(), "unknown".to_owned()];
        assert_eq!(
            dated_absences(&names, "class", Some(&history)),
            ["chrome-models (absent since 12 Oct)", "unknown"]
        );
        let paths = vec!["/old".to_owned()];
        assert_eq!(
            dated_absences(&paths, "protected", Some(&history)),
            ["/old (absent since 1 Oct or earlier)"]
        );
        assert_eq!(
            dated_absences(&names, "class", None),
            ["chrome-models", "unknown"],
            "no history is no date, never a guess"
        );
    }

    #[test]
    fn a_stamp_outside_the_canonical_shape_prints_as_itself() {
        assert_eq!(day_label("2026-10-12 21:00:00.000+00:00"), "12 Oct");
        assert_eq!(day_label("2026-13-01"), "2026-13-01");
        assert_eq!(day_label("yesterday"), "yesterday");
    }

    #[test]
    fn class_growth_needs_two_readings() {
        let mut history = history(
            &[
                ("2026-10-12 21:00:00.000+00:00", 1),
                ("2026-10-12 22:00:00.000+00:00", 1),
            ],
            Vec::new(),
        );
        history.classes = vec![
            StorageClassSample {
                name: "cargo-target".to_owned(),
                bytes: 1_073_741_824,
            },
            StorageClassSample {
                name: "cargo-target".to_owned(),
                bytes: 4_294_967_296,
            },
            StorageClassSample {
                name: "once".to_owned(),
                bytes: 5,
            },
        ];
        let deltas = history.class_deltas();
        assert_eq!(
            deltas.get("cargo-target").map(String::as_str),
            Some("+3.0 GB")
        );
        assert_eq!(deltas.get("once"), None);
    }

    fn surface(id: &str, owner: &str, actionable: bool) -> UpdateSurface {
        UpdateSurface {
            id: id.to_owned(),
            title: id.to_owned(),
            owner: owner.to_owned(),
            owner_detail: String::new(),
            actionable,
            why: String::new(),
        }
    }

    fn update_row(surface: &str, name: &str, status: &str, action: Option<&str>) -> UpdateRow {
        UpdateRow {
            surface: surface.to_owned(),
            name: name.to_owned(),
            status: status.to_owned(),
            action: action.map(str::to_owned),
            ..UpdateRow::default()
        }
    }

    #[test]
    fn groups_follow_the_owner_order_and_count_only_rows_a_verb_can_move() {
        let report = UpdatesReport {
            surfaces: vec![
                surface("self", "self", false),
                surface("brew", "scheduled", true),
            ],
            rows: vec![
                update_row("brew", "jq", "stale", Some("brew upgrade jq")),
                update_row("brew", "pinned", "stale", None),
                update_row("brew", "fresh", "current", None),
                update_row("self", "chrome", "current", None),
            ],
            ..UpdatesReport::default()
        };
        let view = report.view();
        let ids: Vec<&str> = view.groups.iter().map(|g| g.surface.id.as_str()).collect();
        assert_eq!(ids, ["brew", "self"], "scheduled before self-managed");
        assert_eq!(
            view.groups[0].actionable, 1,
            "a pinned row is not a promise"
        );
        assert_eq!(view.groups[1].actionable, 0);
        assert_eq!(view.stale, 2);
        assert!(!view.busy);
    }

    #[test]
    fn a_fifth_status_reads_as_unknown_and_is_never_counted_current() {
        let report = UpdatesReport {
            surfaces: vec![surface("cargo", "manual", true)],
            rows: vec![update_row("cargo", "macmon", "brand-new", None)],
            ..UpdatesReport::default()
        };
        let view = report.view();
        assert_eq!(view.stale, 0);
        assert_eq!(view.unknown, 1);
    }

    #[test]
    fn the_version_cell_shows_both_versions_only_when_they_differ() {
        let mut row = update_row("brew", "jq", "stale", None);
        assert_eq!(version_label(&row), "");
        row.installed = Some("0.7.0".to_owned());
        assert_eq!(version_label(&row), "0.7.0");
        row.latest = Some("0.7.0".to_owned());
        assert_eq!(version_label(&row), "0.7.0");
        row.latest = Some("0.8.2".to_owned());
        assert_eq!(version_label(&row), "0.7.0 → 0.8.2");
    }

    fn receipt(state: &str, audit: Option<&str>) -> ApplyReceipt {
        ApplyReceipt {
            at: Some("2026-10-22T09:00:00+00:00".to_owned()),
            class: Some("cargo".to_owned()),
            steps: Some(3),
            failed: Some(0),
            still_stale: Some(1),
            state: Some(state.to_owned()),
            audit: audit.map(str::to_owned),
        }
    }

    #[test]
    fn the_summary_says_what_the_last_run_did_and_never_hides_the_audit() {
        let now = local("2026-10-22T09:42:00+00:00");
        let view = UpdatesView {
            last_apply: Some(receipt("done", Some("clean"))),
            ..UpdatesView::default()
        };
        assert_eq!(
            apply_summary(&view, now),
            "cargo — 3 steps applied, 1 still stale · audit clean · 42m ago"
        );
        let failed = UpdatesView {
            last_apply: Some(ApplyReceipt {
                failed: Some(2),
                ..receipt("done", Some("2 findings"))
            }),
            ..UpdatesView::default()
        };
        assert_eq!(
            apply_summary(&failed, now),
            "cargo — 2 of 3 failed, 1 still stale · audit 2 findings — run tools/audit · 42m ago"
        );
        let running = UpdatesView {
            last_apply: Some(receipt("running", None)),
            ..UpdatesView::default()
        };
        assert_eq!(
            apply_summary(&running, now),
            "cargo — applying, started 42m ago"
        );
        assert_eq!(
            apply_summary(&UpdatesView::default(), now),
            "",
            "no receipt, no line"
        );
    }

    #[test]
    fn only_clean_is_good_news() {
        assert!(!audit_needs_attention(None));
        assert!(!audit_needs_attention(Some("clean")));
        assert!(audit_needs_attention(Some("1 finding")));
        assert!(audit_needs_attention(Some("scanner-missing")));
    }

    #[test]
    fn a_running_apply_disables_the_buttons() {
        let report = UpdatesReport {
            last_apply: Some(receipt("running", None)),
            ..UpdatesReport::default()
        };
        assert!(report.view().busy);
    }

    fn runtime(
        selection: &str,
        effective: &str,
        configured: bool,
        allow: &[&str],
    ) -> RuntimeStatus {
        RuntimeStatus {
            configured,
            selection: selection.to_owned(),
            effective: effective.to_owned(),
            power: "battery".to_owned(),
            power_fresh: true,
            allow: allow.iter().map(|c| (*c).to_owned()).collect(),
            ..RuntimeStatus::default()
        }
    }

    #[test]
    fn a_fresh_install_reads_as_not_configured_running_normal() {
        let view = runtime("auto", "normal", false, &[]).view();
        assert_eq!(view.label, "Normal");
        assert_eq!(view.source, "Not configured");
        assert!(!view.on_the_go);
        assert_eq!(
            view.summary(),
            "Normal · Not configured · No active exceptions"
        );
    }

    #[test]
    fn auto_names_the_power_it_resolved_from_and_flags_a_stale_observation() {
        let mut status = runtime("auto", "normal", true, &[]);
        assert_eq!(status.view().source, "Auto · Battery");
        status.power_fresh = false;
        assert_eq!(status.view().source, "Auto · Battery (stale)");
        status.power = "ac".to_owned();
        assert_eq!(status.view().source, "Auto · AC (stale)");
    }

    #[test]
    fn on_the_go_lists_its_exceptions_and_names_afm_only_when_no_model_is_allowed() {
        let view = runtime("on-the-go", "on-the-go", true, &["transcription"]).view();
        assert!(view.on_the_go);
        assert!(view.afm_only);
        assert_eq!(view.exceptions, ["Transcription"]);
        assert_eq!(
            view.summary(),
            "On the go · Manual · AFM only · Allowed: Transcription"
        );

        let with_models = runtime(
            "on-the-go",
            "on-the-go",
            true,
            &["other-local-models", "bulk-indexing"],
        )
        .view();
        assert!(!with_models.afm_only);
        assert_eq!(
            with_models.exceptions,
            ["Other local models", "Bulk indexing"]
        );
    }

    #[test]
    fn exceptions_only_exist_while_on_the_go() {
        let view = runtime("normal", "normal", true, &["transcription"]).view();
        assert!(view.exceptions.is_empty(), "normal is unrestricted");
    }

    #[test]
    fn a_runtime_status_answers_with_defaults_when_the_body_is_empty() {
        let status: RuntimeStatus = serde_json::from_str("{}").expect("an empty body parses");
        assert_eq!(status.selection, "auto");
        assert_eq!(status.view().label, "Normal");
    }
}
