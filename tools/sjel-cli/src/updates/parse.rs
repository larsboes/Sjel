//! The parsers `tools/updates` is built on, and the version arithmetic only cargo needs.
//!
//! Every format here belongs to a package manager, not to Sjel — cargo, npm, brew, rustup — so
//! these are the functions most likely to rot silently. `updates.test.ts` drove them against
//! captured output; the port keeps that, with each parser's fixtures and cases moved here.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

// ── cargo ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateEntry {
    pub name: String,
    pub version: String,
}

static CARGO_LIST_LINE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^([A-Za-z0-9_.-]+) v(\S+?):?\s*$").unwrap());

/// `cargo install --list`. A crate's binaries are indented continuation lines.
pub fn parse_cargo_install_list(text: &str) -> Vec<CrateEntry> {
    text.split('\n')
        .filter_map(|line| {
            CARGO_LIST_LINE.captures(line).map(|c| CrateEntry {
                name: c[1].to_owned(),
                version: c[2].to_owned(),
            })
        })
        .collect()
}

/// `cargo search <crate> --limit 1` → the newest release, or `None` when it says nothing.
pub fn parse_cargo_search(name: &str, text: &str) -> Option<String> {
    let re = regex::Regex::new(&format!(r#"(?m)^{}\s*=\s*"([^"]+)""#, regex::escape(name))).ok()?;
    re.captures(text).map(|c| c[1].to_owned())
}

/// crates.io's version list → the newest STABLE release, and the newest release of any kind.
///
/// The defect this exists to fix: `cargo search` returns only the maximum version, so for
/// `tauri-cli` it answered `3.0.0-alpha.4` while the released patch `2.12.1` sat one line
/// further down the list the registry had already sent. Yanked versions are dropped: they are
/// published but withdrawn, and offering one is the same mistake in the other direction.
pub fn parse_crates_io_versions(json: &str) -> (Option<String>, Option<String>) {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return (None, None);
    };
    let nums: Vec<String> = v
        .get("versions")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter(|e| e.get("yanked").and_then(Value::as_bool) != Some(true))
                .filter_map(|e| e.get("num").and_then(Value::as_str))
                .filter(|n| !n.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if nums.is_empty() {
        return (None, None);
    }
    let max = |list: &[String]| -> String {
        list.iter().fold(list[0].clone(), |a, b| {
            if version_newer(b, &a) {
                b.clone()
            } else {
                a
            }
        })
    };
    let stables: Vec<String> = nums.iter().filter(|n| !n.contains('-')).cloned().collect();
    let stable = (!stables.is_empty()).then(|| max(&stables));
    (stable, Some(max(&nums)))
}

// ── npm ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpmOutdated {
    pub name: String,
    pub current: String,
    pub latest: String,
}

/// A JSON object in the order its keys appear, because npm's NESTED dependency order is not
/// sorted and `serde_json`'s default map is. Only these parsers need it: the `preserve_order`
/// feature would change `serde_json::Map` for every crate in the workspace build.
#[derive(Debug, Default)]
pub struct OrderedMap<T>(Vec<(String, T)>);

impl<T> OrderedMap<T> {
    fn iter(&self) -> impl Iterator<Item = (&str, &T)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for OrderedMap<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for V<T> {
            type Value = OrderedMap<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut access: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some((k, v)) = access.next_entry::<String, T>()? {
                    out.push((k, v));
                }
                Ok(OrderedMap(out))
            }
        }
        d.deserialize_map(V(std::marker::PhantomData))
    }
}

/// `npm outdated -g --json`. npm exits 1 when it has something to report, which is not an error.
pub fn parse_npm_outdated(json: &str) -> Vec<NpmOutdated> {
    let Ok(entries) = serde_json::from_str::<OrderedMap<Value>>(json) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|(name, val)| {
            let o = val.as_object()?;
            let latest = o.get("latest").and_then(Value::as_str)?;
            let current = o
                .get("current")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_owned();
            Some(NpmOutdated {
                name: name.to_owned(),
                current,
                latest: latest.to_owned(),
            })
        })
        .collect()
}

/// `npm view <pkg> deprecated` → the deprecation message, or `None` when the package is live.
pub fn parse_npm_deprecated(text: &str) -> Option<String> {
    let line = text.trim().split('\n').next().unwrap_or("").trim();
    if line.is_empty() || line.starts_with("npm error") || line.starts_with("npm warn") {
        return None;
    }
    Some(line.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameVersion {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentVersion {
    pub parent: String,
    pub version: String,
}

/// One node of `npm ls -g --json --all`. Unknown fields (`overridden`, `resolved`, …) are ignored.
#[derive(Debug, Default, Deserialize)]
struct NpmNode {
    version: Option<String>,
    #[serde(default)]
    dependencies: OrderedMap<NpmNode>,
}

#[derive(Debug, Default, Deserialize)]
struct NpmRoot {
    #[serde(default)]
    dependencies: OrderedMap<NpmNode>,
}

#[derive(Debug, Default)]
pub struct NpmTree {
    pub installed: Vec<NameVersion>,
    /// Every node in the tree, not just the top level — the CVE inventory scans this. Deduped by
    /// name@version, so two copies of one package at different versions both appear while a
    /// package required by five parents appears once.
    pub tree: Vec<NameVersion>,
    /// For each top-level package, the parents that carry it in their subtree — with the version
    /// that parent resolved it to, which is what separates a pin from a leftover.
    pub required_by: BTreeMap<String, Vec<ParentVersion>>,
}

/// `npm ls -g --json --all` → the inventory AND which globals constrain which.
///
/// The constraint map is the fix for a defect this tool shipped for one day: `npm outdated -g`
/// reports the newest release of every package it can see, including packages that exist at the
/// top level only because another global hoisted them. A range that constrains one package can
/// belong to a different package, and npm's own `wanted` column cannot see that.
///
/// Object key order is npm's, and npm does not sort the NESTED ones — so this reads them in the
/// order the document has them, which is what the TypeScript version did and what the inventory
/// `tools/audit` scans is compared against.
pub fn parse_npm_global_tree(json: &str) -> NpmTree {
    let Ok(root) = serde_json::from_str::<NpmRoot>(json) else {
        return NpmTree::default();
    };
    let root = &root.dependencies;

    let installed: Vec<NameVersion> = root
        .iter()
        .filter_map(|(name, node)| {
            let version = node.version.as_deref().filter(|v| !v.is_empty())?;
            Some(NameVersion {
                name: name.to_owned(),
                version: version.to_owned(),
            })
        })
        .collect();

    let mut seen = BTreeSet::new();
    let mut tree: Vec<NameVersion> = Vec::new();
    for (name, node) in root.iter() {
        note(name, node.version.as_deref(), &mut seen, &mut tree);
    }

    let names: Vec<&str> = root.iter().map(|(name, _)| name).collect();
    let mut required_by: BTreeMap<String, Vec<ParentVersion>> = BTreeMap::new();
    for (parent, node) in root.iter() {
        let mut sub: BTreeMap<String, String> = BTreeMap::new();
        collect(node, &mut sub, &mut seen, &mut tree);
        for name in &names {
            if *name == parent {
                continue;
            }
            let Some(version) = sub.get(*name) else {
                continue;
            };
            required_by
                .entry((*name).to_owned())
                .or_default()
                .push(ParentVersion {
                    parent: parent.to_owned(),
                    version: version.clone(),
                });
        }
    }
    for v in required_by.values_mut() {
        v.sort_by(|a, b| a.parent.cmp(&b.parent));
    }

    NpmTree {
        installed,
        tree,
        required_by,
    }
}

/// Add a node to the deduped inventory, skipping one with no version: a node a registry cannot
/// be asked about is not a thing to scan.
fn note(
    name: &str,
    version: Option<&str>,
    seen: &mut BTreeSet<String>,
    tree: &mut Vec<NameVersion>,
) {
    let Some(version) = version.filter(|v| !v.is_empty()) else {
        return;
    };
    if seen.insert(format!("{name}@{version}")) {
        tree.push(NameVersion {
            name: name.to_owned(),
            version: version.to_owned(),
        });
    }
}

/// Walk one node's subtree, first writer wins: a parent that resolves a name twice is a tree npm
/// does not build.
fn collect(
    node: &NpmNode,
    sub: &mut BTreeMap<String, String>,
    seen: &mut BTreeSet<String>,
    tree: &mut Vec<NameVersion>,
) {
    for (name, child) in node.dependencies.iter() {
        sub.entry(name.to_owned())
            .or_insert_with(|| child.version.clone().unwrap_or_else(|| "?".to_owned()));
        note(name, child.version.as_deref(), seen, tree);
        collect(child, sub, seen, tree);
    }
}

// ── brew and rustup ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrewOutdated {
    pub name: String,
    pub installed: String,
    pub latest: String,
}

/// `brew outdated --json=v2`.
pub fn parse_brew_outdated(json: &str) -> Vec<BrewOutdated> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let one = |e: &Value, cask: bool| BrewOutdated {
        name: format!(
            "{}{}",
            js_string(e.get("name"), "?"),
            if cask { " (cask)" } else { "" }
        ),
        installed: e
            .get("installed_versions")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .map(|x| js_string(Some(x), "?"))
            .unwrap_or_else(|| "?".to_owned()),
        latest: js_string(e.get("current_version"), "?"),
    };
    let list = |key: &str, cask: bool| -> Vec<BrewOutdated> {
        v.get(key)
            .and_then(Value::as_array)
            .map(|arr| arr.iter().map(|e| one(e, cask)).collect())
            .unwrap_or_default()
    };
    let mut out = list("formulae", false);
    out.extend(list("casks", true));
    out
}

/// `brew list --formula` — used only to spot a global npm package that shadows a brew formula.
pub fn parse_brew_formulae(text: &str) -> BTreeSet<String> {
    text.split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustupComponent {
    pub component: String,
    pub installed: String,
    pub latest: Option<String>,
}

static RUSTUP_UPDATE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)^(\S+)\s+-\s+update available\s*:\s*(\S+)\s*->\s*(\S+)").unwrap()
});
static RUSTUP_OK: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?i)^(\S+)\s+-\s+up to date\s*:\s*(\S+)").unwrap());

/// `rustup check`: "… - up to date: 1.99.0" or "… - Update available : 1.98.0 -> 1.99.0".
pub fn parse_rustup_check(text: &str) -> Vec<RustupComponent> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        if let Some(c) = RUSTUP_UPDATE.captures(line) {
            out.push(RustupComponent {
                component: c[1].to_owned(),
                installed: c[2].to_owned(),
                latest: Some(c[3].to_owned()),
            });
            continue;
        }
        if let Some(c) = RUSTUP_OK.captures(line) {
            out.push(RustupComponent {
                component: c[1].to_owned(),
                installed: c[2].to_owned(),
                latest: None,
            });
        }
    }
    out
}

// ── receipts ──────────────────────────────────────────────────────────────────

/// `<overlay>/data/host-patch/last.json` and its container-refresh sibling.
#[derive(Debug, Clone, Default)]
pub struct Receipt {
    pub at: Option<String>,
    pub ran: Option<String>,
    /// Part of the receipt shape `tools/host-patch.sh` writes; nothing in this report reads it.
    #[allow(dead_code)]
    pub skipped: Option<String>,
    pub failed: Option<String>,
    pub audit: Option<String>,
}

pub fn parse_receipt(json: &str) -> Option<Receipt> {
    let v: Value = serde_json::from_str(json).ok()?;
    if !v.is_object() && !v.is_array() {
        return None;
    }
    Some(Receipt {
        at: field(&v, "at"),
        ran: field(&v, "ran"),
        skipped: field(&v, "skipped"),
        failed: field(&v, "failed"),
        audit: field(&v, "audit"),
    })
}

/// A field as `Date`/template literals would read it: a string stays one, any other scalar takes
/// its JSON text, and `null` or a missing key is absent.
fn field(v: &Value, key: &str) -> Option<String> {
    match v.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => Some(other.to_string()),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReceiptSummary {
    pub age_h: f64,
    pub audit: String,
    pub failed: String,
    pub ran: String,
}

/// Where `apply` records what it is doing, and what it did. The same shape as the two scheduled
/// jobs' receipts and for the same reason: a caller that cannot wait for the answer — the
/// dashboard's apply button, whose cargo step compiles for minutes — needs a file to read rather
/// than a request to hold open.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApplyReceipt {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    #[serde(rename = "class", skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<u64>,
    #[serde(rename = "stillStale", skip_serializing_if = "Option::is_none")]
    pub still_stale: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// `tools/audit`'s verdict, taken immediately after the steps above. Absent on a receipt
    /// written before this field existed, which is why it is optional rather than defaulted. A
    /// string rather than the enum: a receipt written by a newer tool must still render here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit: Option<String>,
}

/// `tools/audit`'s own exit contract, restated as a value: 0 clean · 1 a finding · 2 a scanner is
/// not installed. Anything else is the audit failing to run at all, which is a third thing and
/// must not be folded into either of the first two — an audit that did not run is not evidence
/// that the machine is clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditVerdict {
    Clean,
    Finding,
    ScannerMissing,
    CouldNotRun,
}

impl AuditVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditVerdict::Clean => "clean",
            AuditVerdict::Finding => "finding(s)",
            AuditVerdict::ScannerMissing => "scanner-missing",
            AuditVerdict::CouldNotRun => "could not run",
        }
    }

    /// What to do about a verdict, in the one line a receipt can carry.
    pub fn advice(self) -> &'static str {
        match self {
            AuditVerdict::Clean => "",
            AuditVerdict::Finding => " — run tools/audit for the detail",
            AuditVerdict::ScannerMissing => " — a scanner is not installed, so nothing was scanned",
            AuditVerdict::CouldNotRun => " — tools/audit did not run",
        }
    }
}

pub fn audit_verdict(code: i32) -> AuditVerdict {
    match code {
        0 => AuditVerdict::Clean,
        1 => AuditVerdict::Finding,
        2 => AuditVerdict::ScannerMissing,
        _ => AuditVerdict::CouldNotRun,
    }
}

// ── version arithmetic ────────────────────────────────────────────────────────
// Only cargo needs it: brew, npm and rustup are asked "what is outdated" and answer it
// themselves. Deliberately not a full semver implementation — it compares release segments
// numerically and treats a pre-release as older than its release.

pub fn version_newer(latest: &str, installed: &str) -> bool {
    let a = segment(latest);
    let b = segment(installed);
    for i in 0..a.parts.len().max(b.parts.len()) {
        let x = a.parts.get(i).copied().unwrap_or(0);
        let y = b.parts.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    if a.pre == b.pre {
        return false;
    }
    a.pre.is_empty()
}

struct Segment {
    parts: Vec<i64>,
    pre: String,
}

fn segment(v: &str) -> Segment {
    let v = v.strip_prefix('v').unwrap_or(v);
    // `split("-", 2)` keeps the first two fields and discards the rest, so "1-2-3" has pre "2".
    let core = v.split('-').next().unwrap_or("");
    let pre = v.split('-').nth(1).unwrap_or("");
    Segment {
        parts: core.split('.').map(js_parse_int).collect(),
        pre: pre.to_owned(),
    }
}

/// `parseInt(s, 10) || 0`: the leading integer, or 0 when there is none.
fn js_parse_int(s: &str) -> i64 {
    let s = s.trim_start();
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return 0;
    }
    let v: i64 = digits.parse().unwrap_or(0);
    if neg {
        -v
    } else {
        v
    }
}

/// `String(value ?? fallback)` for the scalar shapes a package manager prints.
fn js_string(v: Option<&Value>, fallback: &str) -> String {
    match v {
        None | Some(Value::Null) => fallback.to_owned(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Output captured from `cargo install --list` on this host, 2026-10-01. The indented lines
    // are each crate's binaries; a parser that folded them in would report a crate named "btm".
    const CARGO_LIST: &str = "bottom v0.14.7:\n    btm\nmacmon v0.7.0:\n    macmon\ntauri-cli v2.12.0:\n    cargo-tauri\nxberg-cli v1.0.14:\n    xberg\n";

    fn nv(name: &str, version: &str) -> NameVersion {
        NameVersion {
            name: name.to_owned(),
            version: version.to_owned(),
        }
    }

    #[test]
    fn cargo_install_list_reads_one_crate_per_stanza() {
        assert_eq!(
            parse_cargo_install_list(CARGO_LIST),
            vec![
                CrateEntry {
                    name: "bottom".into(),
                    version: "0.14.7".into()
                },
                CrateEntry {
                    name: "macmon".into(),
                    version: "0.7.0".into()
                },
                CrateEntry {
                    name: "tauri-cli".into(),
                    version: "2.12.0".into()
                },
                CrateEntry {
                    name: "xberg-cli".into(),
                    version: "1.0.14".into()
                },
            ]
        );
        assert_eq!(
            parse_cargo_install_list("foo v1.2.3\n"),
            vec![CrateEntry {
                name: "foo".into(),
                version: "1.2.3".into()
            }]
        );
        assert_eq!(parse_cargo_install_list(""), vec![]);
    }

    #[test]
    fn cargo_search_reads_the_crate_asked_about() {
        let out = "other = \"9.9.9\"   # a different crate\nmacmon = \"0.8.2\"   # Apple Silicon monitor\n";
        assert_eq!(parse_cargo_search("macmon", out).as_deref(), Some("0.8.2"));
        assert_eq!(
            parse_cargo_search("a.b+c", "a.b+c = \"1.0.0\"").as_deref(),
            Some("1.0.0")
        );
        assert_eq!(
            parse_cargo_search("macmon", "something-else = \"1.0.0\""),
            None
        );
    }

    #[test]
    fn crates_io_finds_the_newest_stable_behind_a_prerelease() {
        let versions = |list: &str| format!("{{\"versions\":[{list}],\"meta\":{{}}}}");
        let json = versions(
            r#"{"num":"3.0.0-alpha.4"},{"num":"2.12.1"},{"num":"2.12.0"},{"num":"2.11.5"}"#,
        );
        assert_eq!(
            parse_crates_io_versions(&json),
            (Some("2.12.1".into()), Some("3.0.0-alpha.4".into()))
        );
        let numeric = versions(r#"{"num":"2.9.0"},{"num":"2.10.0"},{"num":"2.12.1"}"#);
        assert_eq!(
            parse_crates_io_versions(&numeric).0.as_deref(),
            Some("2.12.1")
        );
        let yanked = versions(r#"{"num":"2.12.1","yanked":true},{"num":"2.12.0"}"#);
        assert_eq!(
            parse_crates_io_versions(&yanked).0.as_deref(),
            Some("2.12.0")
        );
        let pre_only = versions(r#"{"num":"1.0.0-beta.1"},{"num":"0.9.0-rc.2"}"#);
        assert_eq!(
            parse_crates_io_versions(&pre_only),
            (None, Some("1.0.0-beta.1".into()))
        );
        assert_eq!(parse_crates_io_versions("<html>403</html>"), (None, None));
        assert_eq!(parse_crates_io_versions(&versions("")), (None, None));
    }

    #[test]
    fn npm_outdated_reads_current_and_latest() {
        let json = r#"{"@marckrenn/pi-sub-bar":{"current":"1.4.0","wanted":"1.5.0","latest":"1.5.0"},"pnpm":{"current":"10.23.0","wanted":"12.8.1","latest":"12.8.1"}}"#;
        assert_eq!(
            parse_npm_outdated(json),
            vec![
                NpmOutdated {
                    name: "@marckrenn/pi-sub-bar".into(),
                    current: "1.4.0".into(),
                    latest: "1.5.0".into()
                },
                NpmOutdated {
                    name: "pnpm".into(),
                    current: "10.23.0".into(),
                    latest: "12.8.1".into()
                },
            ]
        );
        // npm prints warnings on stderr and sometimes on stdout; a throw here would take the
        // whole report down over a package nothing is wrong with.
        assert_eq!(parse_npm_outdated("npm warn something\n"), vec![]);
    }

    #[test]
    fn npm_deprecated_reads_the_registry_message_and_ignores_npm_errors() {
        assert_eq!(
            parse_npm_deprecated("please use @earendil-works/pi-agent-core instead going forward")
                .as_deref(),
            Some("please use @earendil-works/pi-agent-core instead going forward")
        );
        assert_eq!(parse_npm_deprecated(""), None);
        assert_eq!(parse_npm_deprecated("npm error code E404"), None);
        assert_eq!(parse_npm_deprecated("npm warn something"), None);
    }

    #[test]
    fn npm_global_tree_reads_the_version_map() {
        let json = r#"{"dependencies":{"@marckrenn/pi-sub-bar":{"version":"1.4.0"},"@earendil-works/pi-coding-agent":{"version":"0.99.2"},"uv":{"version":"1.4.0"}}}"#;
        let tree = parse_npm_global_tree(json);
        let names: Vec<&str> = tree.installed.iter().map(|p| p.name.as_str()).collect();
        // npm's top-level order is the document's; the fixture is in that order, not sorted.
        assert_eq!(
            names,
            vec![
                "@marckrenn/pi-sub-bar",
                "@earendil-works/pi-coding-agent",
                "uv"
            ]
        );
    }

    #[test]
    fn npm_tree_reaches_the_nested_nodes_installed_stops_short_of() {
        let json = r#"{"dependencies":{"@marckrenn/pi-sub-bar":{"version":"1.5.0","dependencies":{"ghost":{"version":"1.0.0","dependencies":{"deep":{"version":"2.0.0"}}}}},"uv":{"version":"1.4.0"}}}"#;
        let tree = parse_npm_global_tree(json);
        let mut got: Vec<String> = tree
            .tree
            .iter()
            .map(|p| format!("{}@{}", p.name, p.version))
            .collect();
        got.sort();
        let mut want = vec![
            "@marckrenn/pi-sub-bar@1.5.0".to_owned(),
            "deep@2.0.0".to_owned(),
            "ghost@1.0.0".to_owned(),
            "uv@1.4.0".to_owned(),
        ];
        want.sort();
        assert_eq!(got, want);

        // A node with no version is not a thing a registry can be asked about, and a node
        // reached twice is one package: the same name@version is never emitted twice.
        let dupes = r#"{"dependencies":{"a":{"version":"1.0.0","dependencies":{"shared":{"version":"3.0.0"},"nonode":{}}},"b":{"version":"1.0.0","dependencies":{"shared":{"version":"3.0.0"}}}}}"#;
        let t = parse_npm_global_tree(dupes);
        assert_eq!(
            t.tree
                .iter()
                .filter(|p| p.name == "shared")
                .collect::<Vec<_>>(),
            vec![&nv("shared", "3.0.0")]
        );
        assert!(!t.tree.iter().any(|p| p.name == "nonode"));
    }

    #[test]
    fn npm_tree_a_flat_tree_constrains_nothing() {
        let json = r#"{"dependencies":{"a":{"version":"1.0.0"},"b":{"version":"2.0.0"}}}"#;
        assert!(parse_npm_global_tree(json).required_by.is_empty());
    }

    #[test]
    fn npm_tree_separates_a_pin_from_a_leftover_by_version() {
        // `@marckrenn/pi-sub-bar@1.5.0` bundles its OWN `@mariozechner/pi-coding-agent@0.73.1`,
        // so the top-level 0.52.12 satisfies nobody.
        let tree = r#"{"dependencies":{"@marckrenn/pi-sub-bar":{"version":"1.5.0","dependencies":{"@mariozechner/pi-coding-agent":{"version":"0.73.1","dependencies":{"@mariozechner/pi-agent-core":{"version":"0.73.1"}}}}},"@mariozechner/pi-agent-core":{"version":"0.52.12"}}}"#;
        assert_eq!(
            parse_npm_global_tree(tree)
                .required_by
                .get("@mariozechner/pi-agent-core"),
            Some(&vec![ParentVersion {
                parent: "@marckrenn/pi-sub-bar".into(),
                version: "0.73.1".into()
            }])
        );
    }

    #[test]
    fn npm_tree_names_the_parents_that_pin_a_hoisted_package() {
        let tree = r#"{"dependencies":{"claude-agent-sdk-pi":{"version":"1.0.16","dependencies":{"@mariozechner/pi-coding-agent":{"version":"0.52.12","dependencies":{"@mariozechner/pi-agent-core":{"version":"0.52.12"}}}}},"@marckrenn/pi-sub-bar":{"version":"1.4.0","dependencies":{"@sinclair/typebox":{"version":"0.34.48"}}},"@mariozechner/pi-agent-core":{"version":"0.52.12"},"@sinclair/typebox":{"version":"0.34.48"},"defuddle":{"version":"0.19.3"}}}"#;
        let t = parse_npm_global_tree(tree);
        assert_eq!(
            t.required_by.get("@mariozechner/pi-agent-core"),
            Some(&vec![ParentVersion {
                parent: "claude-agent-sdk-pi".into(),
                version: "0.52.12".into()
            }])
        );
        assert_eq!(
            t.required_by.get("@sinclair/typebox"),
            Some(&vec![ParentVersion {
                parent: "@marckrenn/pi-sub-bar".into(),
                version: "0.34.48".into()
            }])
        );
        assert!(!t.required_by.contains_key("defuddle"));
        assert!(!t.required_by.contains_key("claude-agent-sdk-pi"));
        // A broken tree still yields what it could read.
        assert_eq!(parse_npm_global_tree("npm error unmet").installed, vec![]);
    }

    #[test]
    fn brew_outdated_reads_formulae_and_casks() {
        assert_eq!(parse_brew_outdated(r#"{"formulae":[],"casks":[]}"#), vec![]);
        let json = r#"{"formulae":[{"name":"nettle","installed_versions":["3.10"],"current_version":"3.10.1"}],"casks":[{"name":"tuist","installed_versions":["1.0"],"current_version":"1.1"}]}"#;
        assert_eq!(
            parse_brew_outdated(json),
            vec![
                BrewOutdated {
                    name: "nettle".into(),
                    installed: "3.10".into(),
                    latest: "3.10.1".into()
                },
                BrewOutdated {
                    name: "tuist (cask)".into(),
                    installed: "1.0".into(),
                    latest: "1.1".into()
                },
            ]
        );
    }

    #[test]
    fn rustup_check_reads_both_lines() {
        let up = "stable-aarch64-apple-darwin - up to date: 1.99.0 (b940084d7 2026-09-28)\nrustup - up to date : 1.29.1\n";
        assert_eq!(
            parse_rustup_check(up),
            vec![
                RustupComponent {
                    component: "stable-aarch64-apple-darwin".into(),
                    installed: "1.99.0".into(),
                    latest: None
                },
                RustupComponent {
                    component: "rustup".into(),
                    installed: "1.29.1".into(),
                    latest: None
                },
            ]
        );
        let behind = "stable-aarch64-apple-darwin - Update available : 1.98.0 -> 1.99.0\nrustup - up to date : 1.29.1\n";
        assert_eq!(
            parse_rustup_check(behind)[0],
            RustupComponent {
                component: "stable-aarch64-apple-darwin".into(),
                installed: "1.98.0".into(),
                latest: Some("1.99.0".into())
            }
        );
    }

    #[test]
    fn version_newer_compares_numerically_and_ranks_a_release_above_its_prerelease() {
        assert!(version_newer("0.14.9", "0.14.7"));
        assert!(version_newer("1.3.0", "1.0.14"));
        assert!(!version_newer("0.14.7", "0.14.9"));
        assert!(version_newer("2.13.0", "2.13.0-beta.1"));
        assert!(!version_newer("2.13.0-beta.1", "2.13.0"));
        assert!(!version_newer("v1.2.3", "1.2.3"));
    }

    #[test]
    fn a_receipt_that_is_not_json_is_absent_rather_than_silently_fresh() {
        assert!(parse_receipt("not json").is_none());
        assert!(parse_receipt("null").is_none());
    }

    #[test]
    fn the_audit_verdict_is_the_audits_own_exit_contract() {
        assert_eq!(audit_verdict(0), AuditVerdict::Clean);
        assert_eq!(audit_verdict(1), AuditVerdict::Finding);
        assert_eq!(audit_verdict(2), AuditVerdict::ScannerMissing);
        // 127 is the shell's "command not found" — the code that once made this repository
        // report fabricated findings.
        assert_eq!(audit_verdict(127), AuditVerdict::CouldNotRun);
        assert_eq!(audit_verdict(-1), AuditVerdict::CouldNotRun);
        assert_eq!(AuditVerdict::Clean.advice(), "");
        for v in [
            AuditVerdict::Finding,
            AuditVerdict::ScannerMissing,
            AuditVerdict::CouldNotRun,
        ] {
            assert!(v.advice().len() > 5);
        }
    }
}
