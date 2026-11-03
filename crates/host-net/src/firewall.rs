//! What the macOS application firewall (ALF) is set to, and which of its rules point at
//! software that is no longer there.
//!
//! Five readings, all of which run unprivileged: `--getglobalstate`, `--getstealthmode`,
//! `--getallowsigned`, `--getblockall` and `--listapps`.
//!
//! Two getters are deliberately never called. `--getallowsignedapp` does not exist — measured
//! on the build host, it is absent from the tool's own usage string and exits 255 — and
//! `--getappblocked <path>` takes a path argument and it is unproven whether asking about an
//! unknown binary adds it to the list. A read command that might write is not a read command.

use serde::Serialize;
use std::path::Path;

use crate::sys::capture;

pub const SOCKETFILTERFW: &str = "/usr/libexec/ApplicationFirewall/socketfilterfw";

/// The four global switches, as the tool reports them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Switches {
    pub enabled: Option<bool>,
    pub stealth: Option<bool>,
    /// Two independent switches, both printed by `--getallowsigned`. A parser that reads only
    /// the first line reports the downloaded-software switch as unknown, and that is the one
    /// that lets a newly installed signed daemon bind a port with no prompt.
    pub allow_builtin_signed: Option<bool>,
    pub allow_downloaded_signed: Option<bool>,
    pub block_all: Option<bool>,
}

/// Why a rule's path does not resolve. The class is the actionable part: an entry stale because
/// Homebrew bumped a version number is routine churn, and one stale because an application was
/// deleted is a rule nobody will ever remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StaleClass {
    /// `/opt/homebrew/Cellar/<formula>/<version>/...`. Minted on every upgrade of a formula
    /// with a network binary, and the largest class on the build host: 8 of 32 stale entries
    /// measured 2026-09-02 (node at four different versions, plus syncthing, bun and python).
    HomebrewCellar,
    /// `/Library/SystemExtensions/<uuid>/...` for a uuid that has been replaced. Measured: both
    /// system-extension entries on the build host are stale, and the live extension directories
    /// have no ALF entry at all.
    SystemExtension,
    /// `/private/var/folders/.../AppTranslocation/<uuid>/...` — a randomised path Gatekeeper
    /// mints per launch, so the rule can never match the same binary twice.
    AppTranslocation,
    /// A path on a volume that is not mounted now, so the rule re-arms when the disk returns.
    ExternalVolume,
    /// The application is simply gone.
    Removed,
}

impl StaleClass {
    pub fn as_str(self) -> &'static str {
        match self {
            StaleClass::HomebrewCellar => "homebrew-cellar",
            StaleClass::SystemExtension => "system-extension",
            StaleClass::AppTranslocation => "app-translocation",
            StaleClass::ExternalVolume => "external-volume",
            StaleClass::Removed => "removed",
        }
    }
}

/// One entry of `socketfilterfw --listapps`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppRule {
    pub index: u32,
    pub path: String,
    /// `Some(true)` = allow incoming connections, `Some(false)` = block, `None` = the verdict
    /// line was missing.
    pub allow: Option<bool>,
    pub present: bool,
    pub stale_class: Option<StaleClass>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FirewallReport {
    pub switches: Switches,
    pub apps: Vec<AppRule>,
}

/// `%20` → a space. The ALF list stores at least one percent-encoded path on the build host
/// (`/System/Library/CoreServices/Problem%20Reporter.app/...`), and testing that string for
/// existence is the difference between 33 stale entries and the true 32. Decoding runs BEFORE
/// the existence test, which is why it is a parser rule rather than a display nicety.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse the five switch lines. Order-independent and tolerant of a getter that did not run:
/// an unread switch stays `None` and prints as `?`, never as `off`.
pub fn parse_switches(text: &str) -> Switches {
    let mut s = Switches::default();
    for line in text.lines() {
        let l = line.trim();
        let low = l.to_ascii_lowercase();
        if low.starts_with("firewall is ") {
            s.enabled = Some(low.contains("enabled"));
        } else if low.starts_with("firewall stealth mode is ") {
            s.stealth = Some(low.ends_with("on"));
        } else if low.starts_with("automatically allow built-in signed software") {
            s.allow_builtin_signed = Some(low.contains("enabled"));
        } else if low.starts_with("automatically allow downloaded signed software") {
            s.allow_downloaded_signed = Some(low.contains("enabled"));
        } else if low.starts_with("firewall has block all state set to") {
            s.block_all = Some(low.ends_with("enabled."));
        }
    }
    s
}

/// Parse `socketfilterfw --listapps`: a numbered `N : <path>` line, then an indented verdict.
///
/// `exists` is injected so the whole classification is testable without touching the host
/// filesystem, which is what lets this run in CI on a machine that has no ALF at all.
pub fn parse_listapps(text: &str, exists: &dyn Fn(&str) -> bool) -> Vec<AppRule> {
    let mut out: Vec<AppRule> = Vec::new();
    for line in text.lines() {
        if let Some((index, path)) = line.split_once(" : ") {
            let Ok(index) = index.trim().parse::<u32>() else {
                continue;
            };
            let path = percent_decode(path.trim());
            let present = exists(&path);
            out.push(AppRule {
                index,
                allow: None,
                stale_class: if present {
                    None
                } else {
                    Some(classify_stale(&path))
                },
                present,
                path,
            });
            continue;
        }
        let l = line.trim();
        if let Some(rule) = out.last_mut() {
            if l.starts_with("(Allow") {
                rule.allow = Some(true);
            } else if l.starts_with("(Block") {
                rule.allow = Some(false);
            }
        }
    }
    out
}

/// Which structural class a missing path belongs to. Ordered most specific first.
pub fn classify_stale(path: &str) -> StaleClass {
    if path.contains("/Cellar/") {
        StaleClass::HomebrewCellar
    } else if path.starts_with("/Library/SystemExtensions/") {
        StaleClass::SystemExtension
    } else if path.contains("/AppTranslocation/") {
        StaleClass::AppTranslocation
    } else if path.starts_with("/Volumes/") {
        StaleClass::ExternalVolume
    } else {
        StaleClass::Removed
    }
}

/// Read the firewall. macOS only.
pub fn collect() -> Result<FirewallReport, String> {
    let switches = capture(
        SOCKETFILTERFW,
        &[
            "--getglobalstate",
            "--getstealthmode",
            "--getallowsigned",
            "--getblockall",
        ],
    )?;
    let apps = capture(SOCKETFILTERFW, &["--listapps"])?;
    Ok(FirewallReport {
        switches: parse_switches(&switches),
        apps: parse_listapps(&apps, &|p| Path::new(p).exists()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTAPPS: &str = include_str!("../fixtures/socketfilterfw-listapps.txt");
    const SWITCHES: &str = include_str!("../fixtures/socketfilterfw-switches.txt");

    /// Only these fixture paths "exist". Everything else in the fixture is stale, which is what
    /// each class assertion below is measured against.
    fn exists(p: &str) -> bool {
        matches!(
            p,
            "/Applications/Example.app"
                | "/Applications/Problem Reporter.app/Contents/MacOS/Problem Reporter"
                | "/opt/homebrew/Cellar/node/26.7.0/bin/node"
        )
    }

    #[test]
    fn both_signed_software_switches_are_read() {
        let s = parse_switches(SWITCHES);
        assert_eq!(s.enabled, Some(true));
        assert_eq!(s.stealth, Some(true));
        assert_eq!(s.allow_builtin_signed, Some(true));
        assert_eq!(s.allow_downloaded_signed, Some(true));
        assert_eq!(s.block_all, Some(false));
    }

    /// A getter that did not run leaves its switch unknown. `?` and `off` are different claims.
    #[test]
    fn an_unread_switch_stays_unknown() {
        let s = parse_switches("Firewall is enabled. (State = 1)\n");
        assert_eq!(s.enabled, Some(true));
        assert_eq!(s.stealth, None);
        assert_eq!(s.allow_downloaded_signed, None);
    }

    #[test]
    fn a_percent_encoded_path_is_present_once_decoded() {
        let rules = parse_listapps(LISTAPPS, &exists);
        let decoded = rules
            .iter()
            .find(|r| r.path.contains("Problem Reporter.app"))
            .expect("the percent-encoded fixture entry");
        assert!(
            !decoded.path.contains('%'),
            "the path is decoded before the test"
        );
        assert!(decoded.present, "without the decode this counts as stale");
    }

    #[test]
    fn the_verdict_line_is_read_for_each_entry() {
        let rules = parse_listapps(LISTAPPS, &exists);
        assert_eq!(rules[0].allow, Some(true));
        let blocked = rules.iter().find(|r| r.allow == Some(false)).unwrap();
        assert_eq!(blocked.path, "/Applications/Example Blocked.app");
    }

    /// Homebrew version churn is the largest stale class on the build host: 8 of 32 entries
    /// measured 2026-09-02. Reporting those as ordinary removals hides that the class is minted
    /// again on the next upgrade.
    #[test]
    fn stale_entries_carry_their_structural_class() {
        let rules = parse_listapps(LISTAPPS, &exists);
        let class_of = |needle: &str| {
            rules
                .iter()
                .find(|r| r.path.contains(needle))
                .unwrap_or_else(|| panic!("fixture entry for {needle}"))
                .stale_class
        };
        assert_eq!(
            class_of("Cellar/node/26.5.0"),
            Some(StaleClass::HomebrewCellar)
        );
        assert_eq!(
            class_of("SystemExtensions"),
            Some(StaleClass::SystemExtension)
        );
        assert_eq!(
            class_of("AppTranslocation"),
            Some(StaleClass::AppTranslocation)
        );
        assert_eq!(class_of("/Volumes/"), Some(StaleClass::ExternalVolume));
        assert_eq!(class_of("Example Deleted.app"), Some(StaleClass::Removed));
        // A live Cellar path is not stale at all, which is what makes the class churn and not
        // a permanent count.
        assert_eq!(class_of("Cellar/node/26.7.0"), None);
    }
}
