//! What is installed here, how big it is, and what this machine last said about its use.
//!
//! `report` answers "what is filling the disk" by path and `discover` answers "which of those
//! paths does the policy not name". Neither can answer "is this thing still used", because a
//! path has no memory of who touched it. On 2026-10-25 that question was asked of twenty app
//! bundles with a `for` loop over `mdls` in the session, and the answer was worth keeping: the
//! apps were 15 GB of a 294 GB disk, while four Rust build classes held 108 GB. A tool that
//! ranks by size alone invites deleting the wrong thing, and a tool that guesses "unused" from
//! a missing probe invites deleting something the person needs.
//!
//! So every row carries evidence in one of three states, and two of them are not "unused":
//!
//! * [`LastUsed::On`] — Spotlight recorded a use, and the report says when.
//! * [`LastUsed::Never`] — Spotlight indexed the bundle and recorded nothing. Real evidence,
//!   but it cannot see a launch through a helper process, an app that runs as an extension
//!   inside Safari, or use from another login. The report prints it as `never` and never as
//!   `unused`.
//! * [`LastUsed::Unknown`] — `mdls` failed, is missing, or printed nothing. The same rule
//!   `inuse.rs` follows for `lsof` and `service.rs` for a probe: a reading that did not happen
//!   must not be reported as a reading that came back negative, because "nothing uses it" is
//!   the answer that licenses deletion.
//!
//! The size is measured by the same `du` [`crate::measure::size_of`] uses for a policy class,
//! so a number here and a number in `report` are the same number.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use crate::measure::{first_line, fmt_bytes, size_of};

/// How long an app may sit untouched before the report calls it idle.
///
/// Ninety days because the two apps this machine would have flagged first were last used 108
/// and 106 days before the question was asked, and the ones that looked stale at 30 (Endel,
/// GeForceNOW) turned out to be a music app and a game launcher in ordinary use. A month is
/// noise; a quarter is a habit.
pub const DEFAULT_IDLE_DAYS: i64 = 90;

/// The names an app may be recorded under in `~/Library`, and the roots that hold them.
///
/// Four, because macOS splits one app's data across four conventions and an uninstalled app
/// leaves a different shape in each: a bundle-id directory under `Containers` and `Caches`, a
/// display-name directory under `Application Support`, and a team-prefixed one under
/// `Group Containers`.
const RESIDUAL_ROOTS: [&str; 4] = [
    "Library/Application Support",
    "Library/Containers",
    "Library/Group Containers",
    "Library/Caches",
];

/// When the machine last said this was used, and what it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LastUsed {
    /// Spotlight recorded a use. `days_ago` is measured against the caller's today, so a test
    /// can fix the clock instead of depending on when it runs.
    On { date: String, days_ago: i64 },
    /// Indexed, and no use on record.
    Never,
    /// The reading did not happen. Carries why, because "unavailable" alone is unactionable.
    Unknown(String),
}

impl LastUsed {
    /// The one-word state a `--json` reader keys on.
    pub fn state(&self) -> &'static str {
        match self {
            LastUsed::On { .. } => "on",
            LastUsed::Never => "never",
            LastUsed::Unknown(_) => "unknown",
        }
    }

    pub fn is_idle(&self, idle_days: i64) -> bool {
        matches!(self, LastUsed::On { days_ago, .. } if *days_ago >= idle_days)
    }

    fn to_json(&self) -> Value {
        match self {
            LastUsed::On { date, days_ago } => json!({
                "state": self.state(), "date": date, "days_ago": days_ago,
            }),
            LastUsed::Never => json!({ "state": self.state() }),
            LastUsed::Unknown(reason) => json!({ "state": self.state(), "reason": reason }),
        }
    }
}

/// One installed app bundle.
#[derive(Debug, Clone)]
pub struct App {
    pub name: String,
    pub path: String,
    pub bytes: u64,
    pub bundle_id: Option<String>,
    pub last_used: LastUsed,
    /// `~/Library` bytes attributed to this bundle, and where they are.
    pub residual_bytes: u64,
    pub residual_paths: Vec<String>,
    /// `du` could not read the whole bundle, so `bytes` is a lower bound.
    pub partial: bool,
}

/// A directory under one of [`RESIDUAL_ROOTS`], with its size and the root it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Child {
    pub name: String,
    pub path: String,
    pub bytes: u64,
    /// The residual root, because it changes what an unmatched directory means. `discord`
    /// under Application Support is an app's data; `go-build` under Caches is a tool's.
    pub root: String,
}

/// Whether an installed-software source could be asked about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceState {
    Measured,
    /// The tool is not on this machine, so there is nothing to count. Not a zero.
    Absent(String),
    /// The tool ran and did not answer. Also not a zero, and not the same as absent.
    Unverified(String),
}

/// One package manager or toolchain, and what it holds.
#[derive(Debug, Clone)]
pub struct Source {
    pub name: String,
    pub state: SourceState,
    pub count: usize,
    pub bytes: u64,
    /// The one detail that changes what a reader does: leaves against dependencies, for
    /// instance, because 101 of this machine's 146 formulae are things another formula
    /// installed and `brew autoremove` is the verb for them.
    pub detail: String,
}

impl Source {
    fn absent(name: &str, why: String) -> Self {
        Self::of(name, SourceState::Absent(why))
    }

    fn of(name: &str, state: SourceState) -> Self {
        Self {
            name: name.to_string(),
            state,
            count: 0,
            bytes: 0,
            detail: String::new(),
        }
    }

    fn to_json(&self) -> Value {
        let (state, note) = match &self.state {
            SourceState::Measured => ("measured", String::new()),
            SourceState::Absent(why) => ("absent", why.clone()),
            SourceState::Unverified(why) => ("unverified", why.clone()),
        };
        json!({
            "name": self.name, "state": state, "note": note,
            "count": self.count, "bytes": self.bytes, "detail": self.detail,
        })
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date, by Howard Hinnant's civil algorithm.
///
/// `chrono` is not a dependency of this crate and one date-to-days function is not why it
/// should become one. This is the whole of what "how long ago" needs: `mdls` prints UTC and
/// the comparison is between two UTC days.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = (i64::from(month) + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146097 + day_of_era - 719468
}

/// Today in days since the epoch, UTC — the same clock `mdls` prints dates in.
pub fn today_days() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs() / 86_400) as i64)
        .unwrap_or(0)
}

/// `2026-08-28 16:10:03 +0000` → days since the epoch. Anything else → `None`.
pub fn parse_mdls_date(text: &str) -> Option<i64> {
    let date = text.trim().get(..10)?;
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

/// What `mdls -name kMDItemLastUsedDate` answered, in the three states above.
///
/// Split from the command so the parse is tested rather than lived in: `mdls` prints the
/// literal string `(null)` for a metadata key it holds nothing under, and printing that as a
/// date, or as `unknown`, would be two different lies.
pub fn parse_last_used(stdout: &str, stderr: &str, success: bool, today: i64) -> LastUsed {
    if !success {
        let why = first_line(stderr);
        return LastUsed::Unknown(if why.is_empty() {
            "mdls exited non-zero".to_string()
        } else {
            why
        });
    }
    let text = stdout.trim();
    if text.is_empty() {
        return LastUsed::Unknown("mdls printed nothing".to_string());
    }
    if text == "(null)" {
        return LastUsed::Never;
    }
    match parse_mdls_date(text) {
        Some(days) => LastUsed::On {
            date: text.chars().take(10).collect(),
            days_ago: today - days,
        },
        None => LastUsed::Unknown(format!(
            "unreadable date '{}'",
            text.chars().take(40).collect::<String>()
        )),
    }
}

/// `du -d 1` output into one child per line, dropping the root's own total.
///
/// `du` prints the children first and the directory itself last, so the last line has to go
/// or a root's size is added to the sum of its parts. Filtering on the path rather than on
/// the position keeps this right when `du` prints nothing else, which is what an unreadable
/// root looks like.
pub fn parse_du_children(text: &str, root: &str) -> Vec<Child> {
    text.lines()
        .filter_map(|line| {
            let (size, path) = line.split_once('\t')?;
            let bytes = size.trim().parse::<u64>().ok()? * 1024;
            let path = path.trim();
            if path == root || path.is_empty() {
                return None;
            }
            let name = Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string());
            Some(Child {
                name,
                path: path.to_string(),
                bytes,
                root: String::new(),
            })
        })
        .collect()
}

/// Letters and digits only, lowercased.
///
/// `Application Support/Google` and `Google Chrome.app` have to meet, and so do
/// `Group Containers/group.net.whatsapp.WhatsApp.shared` and `WhatsApp.app`. Punctuation is
/// what differs between the two sides in every one of these cases.
pub fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// How strongly a residual directory looks like it belongs to an app, 0 for "no match".
///
/// A bundle id is decisive because macOS derived both sides from it. Names are a heuristic in
/// two directions, and the direction matters: `Google` is a prefix of `Google Chrome`, so the
/// app's own name is the longer one and the residual is a container for it. The reverse —
/// residual longer than app name — is weaker, so it needs a longer stem before it counts.
fn affinity(app: &App, residual: &Child) -> u32 {
    let dir = normalize(&residual.name);
    let app_name = normalize(&app.name);
    if let Some(bundle_id) = &app.bundle_id {
        let bundle = normalize(bundle_id);
        if !bundle.is_empty() && dir.starts_with(&bundle) {
            return 1000 + bundle.len() as u32;
        }
        // A group container carries the owning team prefix first:
        // `HUAQ24HBR6.dev.orbstack` belongs to `dev.orbstack` and starts with neither. Found
        // by running this against the real machine, where 6 GB of OrbStack's data and 3 GB of
        // WhatsApp's were reported as belonging to removed apps.
        if !bundle.is_empty() && dir.contains(&bundle) && bundle.len() >= 6 {
            return 900 + bundle.len() as u32;
        }
    }
    if dir == app_name && app_name.len() >= 3 {
        return 500 + app_name.len() as u32;
    }
    // The app's own name inside the directory. OrbStack is why this exists: its bundle
    // identifier is `dev.kdrag0n.MacVirt` — inherited from the project it started as — so 6 GB
    // of `HUAQ24HBR6.dev.orbstack` matched nothing until the name was allowed to carry it.
    if app_name.len() >= 5 && dir.contains(&app_name) {
        return 700 + app_name.len() as u32;
    }
    if dir.starts_with(&app_name) && app_name.len() >= 3 {
        return 200 + app_name.len() as u32;
    }
    if dir.len() >= 5 && app_name.starts_with(&dir) {
        return 100 + dir.len() as u32;
    }
    0
}

/// What a leftover `~/Library` directory looks like it belongs to.
///
/// The first version of this listed every unmatched directory as "data whose app is gone" and
/// put 1347 entries and 16.3 GB behind that heading, starting with OrbStack's 6 GB and Apple's
/// own `mediaanalysisd`. A heading that names the wrong three things first is worse than no
/// heading, so unmatched directories are sorted before they are printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidualKind {
    /// A reverse-DNS name that matches no installed bundle: an app was here.
    App,
    /// `com.apple.*` and `group.com.apple.*` — the OS maintains these, and no app bundle in
    /// `/Applications` ever owned them. Not a deletion candidate and not the reader's.
    System,
    /// A cache a tool fills: `Homebrew`, `go-build`, `.wasm-pack`. The tool's own verb is what
    /// reclaims it (`brew cleanup`, `go clean -cache`).
    Tooling,
    /// No installed app answers to this name, and nothing about the name says an app once did:
    /// `AddressBook`, `CloudDocs`, `NVIDIA`. Reported as unmatched rather than as orphaned,
    /// because a name that could be an app's is not evidence that an app is gone.
    Unmatched,
}

pub fn classify_residual(name: &str, root: &str) -> ResidualKind {
    let lower = name.to_lowercase();
    if lower.starts_with("com.apple.") || lower.starts_with("group.com.apple.") {
        return ResidualKind::System;
    }
    let dotted = name.contains('.') && !name.starts_with('.');
    if root.ends_with("Library/Caches") {
        // `discord` under Application Support is an app's data and has no dot either; the root
        // is what separates the two, which is why it travels with the entry.
        return if dotted {
            ResidualKind::App
        } else {
            ResidualKind::Tooling
        };
    }
    if dotted {
        ResidualKind::App
    } else {
        ResidualKind::Unmatched
    }
}

/// Hand each residual directory to the app it most looks like, and return the ones that look
/// like nothing installed — an app that was removed and left its data behind.
///
/// One residual goes to at most one app, chosen by the strongest affinity rather than the
/// first: `Application Support/Google` is `Google Chrome`'s and must not be counted again
/// against any other app that happens to start with those letters.
pub fn attribute(apps: &mut [App], residuals: Vec<Child>) -> Vec<Child> {
    let mut orphans = Vec::new();
    for residual in residuals {
        let mut best: Option<(usize, u32)> = None;
        for (index, app) in apps.iter().enumerate() {
            let score = affinity(app, &residual);
            if score > 0 && best.is_none_or(|(_, top)| score > top) {
                best = Some((index, score));
            }
        }
        match best {
            Some((index, _)) => {
                apps[index].residual_bytes += residual.bytes;
                apps[index].residual_paths.push(residual.path.clone());
            }
            None => orphans.push(residual),
        }
    }
    orphans.sort_by_key(|r| std::cmp::Reverse(r.bytes));
    orphans
}

/// The `~/Library` directories that may belong to an app, each with its size.
fn scan_residuals(home: &Path) -> (Vec<Child>, Vec<String>) {
    let mut all = Vec::new();
    let mut errors = Vec::new();
    for relative in RESIDUAL_ROOTS {
        let root = home.join(relative);
        let root_text = root.to_string_lossy().to_string();
        match Command::new("du")
            .arg("-x")
            .arg("-k")
            .arg("-d")
            .arg("1")
            .arg(&root)
            .output()
        {
            Ok(output) => {
                let mut children =
                    parse_du_children(&String::from_utf8_lossy(&output.stdout), &root_text);
                for child in &mut children {
                    child.root = relative.to_string();
                }
                all.extend(children);
                if !output.status.success() {
                    let why = first_line(&String::from_utf8_lossy(&output.stderr));
                    if !why.is_empty() {
                        errors.push(format!("du: {why}"));
                    }
                }
            }
            Err(e) => errors.push(format!("du {root_text}: {e}")),
        }
    }
    (all, errors)
}

/// One app bundle's bundle identifier, read with `defaults` rather than by parsing a plist.
///
/// A binary plist parser is a dependency; `defaults read` is the OS's own reader for the same
/// file, already installed, and its output is one line.
fn bundle_id(app: &Path) -> Option<String> {
    let info = app.join("Contents").join("Info");
    let output = Command::new("defaults")
        .arg("read")
        .arg(info)
        .arg("CFBundleIdentifier")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn last_used(app: &Path, today: i64) -> LastUsed {
    match Command::new("mdls")
        .arg("-raw")
        .arg("-name")
        .arg("kMDItemLastUsedDate")
        .arg(app)
        .output()
    {
        Ok(output) => parse_last_used(
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
            output.status.success(),
            today,
        ),
        Err(e) => LastUsed::Unknown(format!("mdls: {e}")),
    }
}

/// Every `.app` bundle directly inside the given directories.
fn scan_apps(dirs: &[PathBuf], today: i64) -> Vec<App> {
    let mut apps = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("app") {
                continue;
            }
            let Some(name) = path.file_stem().map(|n| n.to_string_lossy().to_string()) else {
                continue;
            };
            let path_text = path.to_string_lossy().to_string();
            let sizing = size_of(std::slice::from_ref(&path_text));
            apps.push(App {
                name,
                path: path_text,
                bytes: sizing.bytes,
                bundle_id: bundle_id(&path),
                last_used: last_used(&path, today),
                residual_bytes: 0,
                residual_paths: Vec::new(),
                partial: !sizing.complete,
            });
        }
    }
    apps.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
    apps
}

/// Lines of a command's stdout, or which of the two kinds of "no" this was.
///
/// A tool that is not installed and a tool that ran and failed are different findings, and the
/// second is not the first with a worse message: `brew --prefix` failing on a machine that has
/// brew is a broken package manager, and reporting it as absent would hide that.
fn stdout_lines(program: &str, args: &[&str]) -> Result<Vec<String>, SourceState> {
    let output = Command::new(program).args(args).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SourceState::Absent(format!("{program} is not installed"))
        } else {
            SourceState::Unverified(format!("{program}: {e}"))
        }
    })?;
    if !output.status.success() {
        let why = first_line(&String::from_utf8_lossy(&output.stderr));
        return Err(SourceState::Unverified(if why.is_empty() {
            format!("{program} exited non-zero")
        } else {
            why
        }));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

fn entries_in(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .count()
        })
        .unwrap_or(0)
}

fn du_of(path: &Path) -> u64 {
    if path.exists() {
        size_of(&[path.to_string_lossy().to_string()]).bytes
    } else {
        0
    }
}

/// The package managers and toolchains this machine runs, each measured against itself.
///
/// Homebrew's own verbs are used for the counts that matter and `du` is not used to decide
/// what a dependency is: `brew leaves` and `brew autoremove --dry-run` already answer that,
/// and a second implementation of the same rule would disagree with the first.
fn scan_sources(home: &Path) -> Vec<Source> {
    let mut sources = Vec::new();

    match stdout_lines("brew", &["--prefix"]) {
        Err(state) => sources.push(Source::of("homebrew", state)),
        Ok(prefix) => {
            let prefix = prefix.first().cloned().unwrap_or_default();
            let formulae = stdout_lines("brew", &["list", "--formula"]).unwrap_or_default();
            let casks = stdout_lines("brew", &["list", "--cask"]).unwrap_or_default();
            let leaves = stdout_lines("brew", &["leaves"]).unwrap_or_default();
            let deps = formulae.len().saturating_sub(leaves.len());
            let bytes = du_of(&Path::new(&prefix).join("Cellar"))
                + du_of(&Path::new(&prefix).join("Caskroom"));
            sources.push(Source {
                name: "homebrew".to_string(),
                state: SourceState::Measured,
                count: formulae.len() + casks.len(),
                bytes,
                detail: format!(
                    "{} formulae ({} leaves, {} pulled in as dependencies) · {} casks",
                    formulae.len(),
                    leaves.len(),
                    deps,
                    casks.len()
                ),
            });
        }
    }

    match stdout_lines("npm", &["root", "-g"]) {
        Err(state) => sources.push(Source::of("npm-global", state)),
        Ok(root) => {
            let dir = PathBuf::from(root.first().cloned().unwrap_or_default());
            let count = entries_in(&dir);
            sources.push(Source {
                name: "npm-global".to_string(),
                state: SourceState::Measured,
                count,
                bytes: du_of(&dir),
                detail: format!("{count} global packages"),
            });
        }
    }

    match stdout_lines("uv", &["tool", "dir"]) {
        Err(state) => sources.push(Source::of("uv-tools", state)),
        Ok(dir) => {
            let dir = PathBuf::from(dir.first().cloned().unwrap_or_default());
            let count = entries_in(&dir);
            sources.push(Source {
                name: "uv-tools".to_string(),
                state: SourceState::Measured,
                count,
                bytes: du_of(&dir),
                detail: format!("{count} tools"),
            });
        }
    }

    let rustup = home.join(".rustup");
    let toolchains = rustup.join("toolchains");
    if toolchains.exists() {
        let names = entries_in(&toolchains);
        sources.push(Source {
            name: "rustup-toolchains".to_string(),
            state: SourceState::Measured,
            count: names,
            bytes: du_of(&toolchains),
            detail: format!("{names} toolchains"),
        });
    } else {
        sources.push(Source::absent(
            "rustup-toolchains",
            "no ~/.rustup/toolchains".to_string(),
        ));
    }

    // Crate sources, not builds: `rust-debug-cache` and friends are what a build leaves, and
    // the registry is what the build downloaded. Different verbs remove them.
    let registry = home.join(".cargo").join("registry");
    if registry.exists() {
        sources.push(Source {
            name: "cargo-registry".to_string(),
            state: SourceState::Measured,
            count: entries_in(&registry),
            bytes: du_of(&registry),
            detail: "downloaded crate sources and their caches".to_string(),
        });
    }

    sources
}

/// What `installed` reports, assembled before it is printed or serialised.
pub struct Inventory {
    pub roots: Vec<String>,
    pub idle_days: i64,
    pub apps: Vec<App>,
    pub sources: Vec<Source>,
    /// Unmatched `~/Library` directories whose name says an app once owned them.
    pub orphans: Vec<Child>,
    /// Unmatched directories no installed app answers to, where the name proves nothing.
    pub unmatched: Vec<Child>,
    /// Unmatched directories this verb will not call orphaned, counted so that dropping them
    /// from the list is visible rather than silent: `(count, bytes)`.
    pub system_residuals: (usize, u64),
    pub tooling_residuals: (usize, u64),
    pub errors: Vec<String>,
}

impl Inventory {
    pub fn to_json(&self) -> Value {
        let sum = |pick: &dyn Fn(&App) -> bool| -> u64 {
            self.apps.iter().filter(|a| pick(a)).map(|a| a.bytes).sum()
        };
        json!({
            "roots": self.roots,
            "idle_days": self.idle_days,
            "apps": self.apps.iter().map(|a| json!({
                "name": a.name,
                "path": a.path,
                "bytes": a.bytes,
                "partial": a.partial,
                "bundle_id": a.bundle_id,
                "last_used": a.last_used.to_json(),
                "residual_bytes": a.residual_bytes,
                "residual_paths": a.residual_paths,
            })).collect::<Vec<_>>(),
            "apps_total_bytes": self.apps.iter().map(|a| a.bytes).sum::<u64>(),
            "idle_bytes": sum(&|a| a.last_used.is_idle(self.idle_days)),
            "never_bytes": sum(&|a| matches!(a.last_used, LastUsed::Never)),
            "unknown_bytes": sum(&|a| matches!(a.last_used, LastUsed::Unknown(_))),
            "residual_total_bytes": self.apps.iter().map(|a| a.residual_bytes).sum::<u64>(),
            "sources": self.sources.iter().map(Source::to_json).collect::<Vec<_>>(),
            "orphans": self.orphans.iter().map(|o| json!({
                "name": o.name, "path": o.path, "bytes": o.bytes,
            })).collect::<Vec<_>>(),
            "unmatched": self.unmatched.iter().map(|o| json!({
                "name": o.name, "path": o.path, "bytes": o.bytes,
            })).collect::<Vec<_>>(),
            "system_residuals": {
                "count": self.system_residuals.0, "bytes": self.system_residuals.1,
            },
            "tooling_residuals": {
                "count": self.tooling_residuals.0, "bytes": self.tooling_residuals.1,
            },
            "errors": self.errors,
        })
    }
}

/// Measure the machine: app bundles, their `~/Library` data, and the package sources.
pub fn measure(roots: Vec<String>, idle_days: i64) -> Inventory {
    let home = std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_default();
    let home = PathBuf::from(home);
    let today = today_days();

    let dirs: Vec<PathBuf> = roots.iter().map(PathBuf::from).collect();
    let mut apps = scan_apps(&dirs, today);
    let (residuals, dup_errors) = scan_residuals(&home);
    let unmatched_children = attribute(&mut apps, residuals);

    // Unmatched is not the same as orphaned. A `~/Library` directory the OS maintains or a
    // tool fills is not evidence that an app was removed, so the three are counted apart.
    let mut orphans = Vec::new();
    let mut unmatched = Vec::new();
    let mut system_residuals = (0usize, 0u64);
    let mut tooling_residuals = (0usize, 0u64);
    for child in unmatched_children {
        match classify_residual(&child.name, &child.root) {
            ResidualKind::App => orphans.push(child),
            ResidualKind::Unmatched => unmatched.push(child),
            ResidualKind::System => {
                system_residuals.0 += 1;
                system_residuals.1 += child.bytes;
            }
            ResidualKind::Tooling => {
                tooling_residuals.0 += 1;
                tooling_residuals.1 += child.bytes;
            }
        }
    }
    unmatched.sort_by_key(|r| std::cmp::Reverse(r.bytes));

    let mut errors = dup_errors;
    for app in &apps {
        if let LastUsed::Unknown(why) = &app.last_used {
            // One line per distinct reason: twenty apps from one unindexed volume is one
            // finding, and twenty copies of it is noise a reader skips.
            let line = format!("{}: {}", app.name, why);
            if !errors.contains(&line) {
                errors.push(line);
            }
        }
    }

    Inventory {
        roots,
        idle_days,
        apps,
        sources: scan_sources(&home),
        orphans,
        unmatched,
        system_residuals,
        tooling_residuals,
        errors,
    }
}

/// The text report. Sections in the order a reader acts on them: what is idle, what is
/// installed, what is left over.
pub fn print(inventory: &Inventory, top: usize) {
    println!(
        "Sjel storage installed · {} (idle from {} days, by kMDItemLastUsedDate)",
        inventory.roots.join(", "),
        inventory.idle_days
    );

    let idle: Vec<&App> = inventory
        .apps
        .iter()
        .filter(|a| a.last_used.is_idle(inventory.idle_days))
        .collect();
    let never: Vec<&App> = inventory
        .apps
        .iter()
        .filter(|a| matches!(a.last_used, LastUsed::Never))
        .collect();
    let unknown: Vec<&App> = inventory
        .apps
        .iter()
        .filter(|a| matches!(a.last_used, LastUsed::Unknown(_)))
        .collect();

    let total: u64 = inventory.apps.iter().map(|a| a.bytes).sum();
    println!(
        "\n{} app bundles · {} · {} of ~/Library data belongs to them",
        inventory.apps.len(),
        fmt_bytes(total),
        fmt_bytes(inventory.apps.iter().map(|a| a.residual_bytes).sum()),
    );

    print_group(
        &format!("Idle — last used {} or more days ago", inventory.idle_days),
        &idle,
        top,
    );
    print_group("Never used, as far as Spotlight knows", &never, top);
    print_group(
        "Unknown — the machine did not answer, which is not the same as unused",
        &unknown,
        top,
    );

    let busy: Vec<&App> = inventory
        .apps
        .iter()
        .filter(|a| {
            !a.last_used.is_idle(inventory.idle_days)
                && !matches!(a.last_used, LastUsed::Never | LastUsed::Unknown(_))
        })
        .collect();
    if !busy.is_empty() {
        let shown = busy.iter().take(top);
        println!(
            "\nIn use ({} bundles, {}):",
            busy.len(),
            fmt_bytes(busy.iter().map(|a| a.bytes).sum())
        );
        for app in shown {
            println!("  {:>9}  {}", fmt_bytes(app.bytes), app.name);
        }
        if busy.len() > top {
            println!("  … {} more", busy.len() - top);
        }
    }

    println!("\nPackage sources");
    for source in &inventory.sources {
        match &source.state {
            SourceState::Measured => {
                println!(
                    "  {:>9}  {:<20} {}",
                    fmt_bytes(source.bytes),
                    source.name,
                    source.detail
                );
            }
            SourceState::Absent(why) | SourceState::Unverified(why) => {
                println!("  {:>9}  {:<20} {}", "—", source.name, why);
            }
        }
    }

    if !inventory.orphans.is_empty() {
        let total: u64 = inventory.orphans.iter().map(|o| o.bytes).sum();
        println!(
            "\nResidual ~/Library data whose app is not installed ({}, {}):",
            fmt_bytes(total),
            inventory.orphans.len()
        );
        for orphan in inventory.orphans.iter().take(top) {
            println!("  {:>9}  {}", fmt_bytes(orphan.bytes), orphan.path);
        }
        if inventory.orphans.len() > top {
            println!("  … {} more", inventory.orphans.len() - top);
        }
    }
    if !inventory.unmatched.is_empty() {
        let total: u64 = inventory.unmatched.iter().map(|o| o.bytes).sum();
        println!(
            "\nNo installed app answers to these names ({}, {}) — not claimed as orphaned:",
            fmt_bytes(total),
            inventory.unmatched.len()
        );
        for child in inventory.unmatched.iter().take(3) {
            println!("  {:>9}  {}", fmt_bytes(child.bytes), child.path);
        }
        if inventory.unmatched.len() > 3 {
            println!("  … {} more", inventory.unmatched.len() - 3);
        }
    }

    // Printed rather than dropped: a reader who sees 16 GB of `~/Library` in `discover` and
    // fewer rows here has to be able to tell that the rest was classified, not missed.
    let (system_count, system_bytes) = inventory.system_residuals;
    let (tooling_count, tooling_bytes) = inventory.tooling_residuals;
    if system_count > 0 || tooling_count > 0 {
        println!(
            "\n  Not listed: {} the OS maintains ({}) and {} tool caches ({}). Neither is an \
             app's, and each is reclaimed by its owner: `brew cleanup`, `go clean -cache`.",
            system_count,
            fmt_bytes(system_bytes),
            tooling_count,
            fmt_bytes(tooling_bytes)
        );
    }

    if !inventory.errors.is_empty() {
        println!("\nUnmeasured — the path or the probe could not be read");
        let mut seen = BTreeSet::new();
        for line in inventory.errors.iter().take(top) {
            if seen.insert(line.clone()) {
                println!("  {line}");
            }
        }
    }

    println!(
        "\nNothing here is removed by this verb. What the policy names, and its reclaim verbs, \
         is `sjel storage report`."
    );
}

fn print_group(title: &str, apps: &[&App], top: usize) {
    if apps.is_empty() {
        return;
    }
    let bytes: u64 = apps.iter().map(|a| a.bytes).sum();
    println!(
        "\n{} ({} bundles, {}):",
        title,
        apps.len(),
        fmt_bytes(bytes)
    );
    for app in apps.iter().take(top) {
        let evidence = match &app.last_used {
            LastUsed::On { date, days_ago } => format!("last used {date} ({days_ago}d)"),
            LastUsed::Never => "never".to_string(),
            LastUsed::Unknown(_) => "unknown".to_string(),
        };
        let residual = if app.residual_bytes > 0 {
            format!(" · {} in ~/Library", fmt_bytes(app.residual_bytes))
        } else {
            String::new()
        };
        let partial = if app.partial { " (partial)" } else { "" };
        println!(
            "  {:>9}  {:<24} {}{}{}",
            fmt_bytes(app.bytes),
            app.name,
            evidence,
            residual,
            partial
        );
    }
    if apps.len() > top {
        println!("  … {} more", apps.len() - top);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, bundle_id: Option<&str>) -> App {
        App {
            name: name.to_string(),
            path: format!("/Applications/{name}.app"),
            bytes: 0,
            bundle_id: bundle_id.map(|b| b.to_string()),
            last_used: LastUsed::Never,
            residual_bytes: 0,
            residual_paths: Vec::new(),
            partial: false,
        }
    }

    fn child(name: &str, bytes: u64) -> Child {
        child_in(name, bytes, "Library/Application Support")
    }

    fn child_in(name: &str, bytes: u64, root: &str) -> Child {
        Child {
            name: name.to_string(),
            path: format!("~/Library/{name}"),
            bytes,
            root: root.to_string(),
        }
    }

    #[test]
    fn civil_days_are_the_epoch_anchor() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        // 2000-03-01 crosses the leap-year rule the algorithm exists to get right.
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
        assert_eq!(
            days_from_civil(2026, 10, 25) - days_from_civil(2026, 10, 24),
            1
        );
    }

    #[test]
    fn mdls_date_parses_and_garbage_does_not() {
        assert_eq!(
            parse_mdls_date("2026-08-28 16:10:03 +0000"),
            Some(days_from_civil(2026, 8, 28))
        );
        assert_eq!(parse_mdls_date("(null)"), None);
        assert_eq!(parse_mdls_date("2026-13-01 00:00:00 +0000"), None);
        assert_eq!(parse_mdls_date(""), None);
    }

    #[test]
    fn last_used_keeps_never_and_unknown_apart() {
        let today = days_from_civil(2026, 10, 25);
        assert_eq!(parse_last_used("(null)", "", true, today), LastUsed::Never);
        assert_eq!(
            parse_last_used("2026-08-28 16:10:03 +0000", "", true, today),
            LastUsed::On {
                date: "2026-08-28".to_string(),
                days_ago: 58
            }
        );
        // A failed probe is never `Never`: that is the answer that licenses deletion.
        assert!(matches!(
            parse_last_used("", "boom", false, today),
            LastUsed::Unknown(reason) if reason == "boom"
        ));
        assert!(matches!(
            parse_last_used("   ", "", true, today),
            LastUsed::Unknown(_)
        ));
    }

    #[test]
    fn idle_is_a_threshold_on_the_measurement() {
        let used = LastUsed::On {
            date: "2026-07-09".to_string(),
            days_ago: 108,
        };
        assert!(used.is_idle(90));
        assert!(!used.is_idle(120));
        assert!(!LastUsed::Never.is_idle(90));
        assert!(!LastUsed::Unknown("x".to_string()).is_idle(90));
    }

    #[test]
    fn du_children_drop_the_root_total() {
        let text = "1024\t/root/a\n2048\t/root/b\n4096\t/root\n";
        let children = parse_du_children(text, "/root");
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].name, "a");
        assert_eq!(children[1].bytes, 2048 * 1024);
        // An unreadable root prints only its own line, which must not become a child.
        assert!(parse_du_children("0\t/root\n", "/root").is_empty());
    }

    #[test]
    fn bundle_id_decides_and_names_break_ties() {
        let mut apps = vec![
            app("Google Chrome", Some("com.google.Chrome")),
            app("Obsidian", Some("md.obsidian")),
            app("OBS", None),
        ];
        let orphans = attribute(
            &mut apps,
            vec![
                child("Google", 100),
                child("md.obsidian", 200),
                child("obs-studio", 300),
                child("SomeRemovedApp", 400),
            ],
        );
        assert_eq!(apps[0].residual_bytes, 100, "Application Support/Google");
        assert_eq!(apps[1].residual_bytes, 200, "bundle-id directory");
        assert_eq!(apps[2].residual_bytes, 300, "display-name prefix");
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].name, "SomeRemovedApp");
    }

    #[test]
    fn one_residual_belongs_to_one_app() {
        // `Google` is a prefix of both names; the more specific match takes it, once.
        let mut apps = vec![
            app("Google Chrome", None),
            app("Google Chrome Helper", None),
        ];
        let orphans = attribute(&mut apps, vec![child("Google", 500)]);
        assert!(orphans.is_empty());
        let total: u64 = apps.iter().map(|a| a.residual_bytes).sum();
        assert_eq!(total, 500, "counted once, not once per app");
    }

    #[test]
    fn a_team_prefixed_group_container_belongs_to_its_bundle() {
        // `HUAQ24HBR6.dev.orbstack` starts with a team id, so a prefix test alone reported
        // OrbStack's 6 GB as belonging to a removed app.
        let mut apps = vec![app("OrbStack", Some("dev.orbstack"))];
        let orphans = attribute(&mut apps, vec![child("HUAQ24HBR6.dev.orbstack", 6_000)]);
        assert!(orphans.is_empty());
        assert_eq!(apps[0].residual_bytes, 6_000);
    }

    #[test]
    fn unmatched_is_not_the_same_as_orphaned() {
        let support = "Library/Application Support";
        let caches = "Library/Caches";
        assert_eq!(
            classify_residual("com.apple.MobileSMS", support),
            ResidualKind::System
        );
        assert_eq!(
            classify_residual("group.com.apple.SiriTTS", support),
            ResidualKind::System
        );
        assert_eq!(classify_residual("Homebrew", caches), ResidualKind::Tooling);
        assert_eq!(classify_residual("go-build", caches), ResidualKind::Tooling);
        assert_eq!(
            classify_residual("ms-playwright", caches),
            ResidualKind::Tooling
        );
        assert_eq!(
            classify_residual("UBF8T346G9.Office", support),
            ResidualKind::App
        );
        // A dotless name outside Caches carries no evidence either way. It is unmatched, not
        // orphaned: Apple's own AddressBook and CloudDocs live in the same shape.
        assert_eq!(
            classify_residual("discord", support),
            ResidualKind::Unmatched
        );
        assert_eq!(
            classify_residual("CloudDocs", support),
            ResidualKind::Unmatched
        );
        assert_eq!(
            classify_residual("ai.perplexity.mac", support),
            ResidualKind::App
        );
        assert_eq!(
            classify_residual(".wasm-pack", caches),
            ResidualKind::Tooling
        );
    }

    #[test]
    fn json_carries_the_state_a_reader_keys_on() {
        let inventory = Inventory {
            roots: vec!["/Applications".to_string()],
            idle_days: 90,
            apps: vec![app("Idle", Some("com.example.idle"))],
            sources: vec![Source::absent("pipx", "not installed".to_string())],
            orphans: vec![child("group.com.monkeytaps.facts", 42)],
            unmatched: vec![child("CloudDocs", 10)],
            system_residuals: (3, 900),
            tooling_residuals: (2, 300),
            errors: Vec::new(),
        };
        let json = inventory.to_json();
        assert_eq!(json["apps"][0]["last_used"]["state"], "never");
        assert_eq!(json["never_bytes"], 0);
        assert_eq!(json["sources"][0]["state"], "absent");
        assert_eq!(json["orphans"][0]["bytes"], 42);
    }
}
