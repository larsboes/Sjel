//! The doctor's pure cores, ported from doctor.ts with every case of tools/doctor.test.ts.
//! Each one carries the reasoning its TypeScript original stated; the sections in checks.rs
//! do the I/O around them.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

use super::{js_round, to_fixed};

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("a doctor pattern compiles"))
}

/// Every `.rs` under `root`, sorted, so the bind policy covers multi-file binary roots such as
/// `src/server/main.rs`, not only flat crates.
pub fn find_rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.filter_map(Result::ok) {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                pending.push(e.path());
            } else if ft.is_file() && e.file_name().to_string_lossy().ends_with(".rs") {
                out.push(e.path());
            }
        }
    }
    out.sort();
    out
}

/// Which `[[state_mount]]` tools have a systems.toml identity, and which do not.
pub fn check_state_mount_coverage(
    mounts: &[String],
    ids: &BTreeSet<String>,
) -> (Vec<String>, Vec<String>) {
    let (mut covered, mut uncovered) = (Vec::new(), Vec::new());
    for m in mounts {
        if ids.contains(m) {
            covered.push(m.clone())
        } else {
            uncovered.push(m.clone())
        }
    }
    (covered, uncovered)
}

/// Candidate sibling-repo names in a blob: the LAST segment of a `$HOME/Developer/...` or
/// `~/Developer/...` path (nested repos are real), skipping paths rooted at one of `self_names`,
/// the checkout and the overlay. Matching the root segment, not the basename, is the point:
/// `~/Developer/<overlay>/config` is inside the overlay, not a repo named `config`.
pub fn extract_sibling_repo_refs(text: &str, self_names: &[String]) -> Vec<String> {
    static P: OnceLock<Regex> = OnceLock::new();
    let p = re(
        &P,
        r"(?:\$HOME|~)/Developer/((?:[A-Za-z0-9._-]+/)*[A-Za-z0-9._-]+)",
    );
    p.captures_iter(text)
        .filter_map(|c| {
            let segs: Vec<&str> = c[1].split('/').collect();
            (!self_names.iter().any(|s| s == segs[0])).then(|| segs[segs.len() - 1].to_owned())
        })
        .collect()
}

/// Exempt from the hardcoded-path sweep by what the file IS: the sanctioned indirection, the
/// bootstrap that names overlay locations, the sweep's own fixtures, a `.example` template whose
/// job is to show a path, and a generated artifact that says so in its first lines.
pub fn is_sweep_exempt(rel: &str, text: &str) -> bool {
    if matches!(
        rel,
        "tools/lib/paths.sh"
            | "tools/install.sh"
            | "tools/doctor.test.ts"
            | "tools/sjel-cli/src/doctor/pure.rs"
    ) {
        return true;
    }
    if rel.ends_with(".example") {
        return true;
    }
    static P: OnceLock<Regex> = OnceLock::new();
    let head: Vec<&str> = text.split('\n').take(6).collect();
    re(&P, r"(?i)auto-generated\b").is_match(&head.join("\n"))
}

/// The prefixes a why-block reference may be written against, derived from tracked files: an
/// owner, a unit inside it, and that unit's `src/` when it has one. Never invented, and never
/// from an untracked build tree.
pub fn why_block_bases(tracked: &[String]) -> Vec<String> {
    let mut bases = BTreeSet::from([String::new()]);
    for f in tracked {
        let seg: Vec<&str> = f.split('/').collect();
        if seg.len() > 1 {
            bases.insert(format!("{}/", seg[0]));
        }
        if seg.len() > 2 {
            bases.insert(format!("{}/{}/", seg[0], seg[1]));
        }
        if seg.len() > 3 && seg[2] == "src" {
            bases.insert(format!("{}/{}/src/", seg[0], seg[1]));
        }
    }
    bases.into_iter().collect()
}

/// Where the item a `#[cfg(test)]` attribute applies to ends: its closing brace, or its `;`
/// before any body. Comments, strings, raw strings and char literals are skipped.
fn rust_cfg_item_end(src: &[u8], start: usize) -> usize {
    let n = src.len();
    let mut depth = 0i32;
    let mut body = false;
    let mut i = start;
    while i < n {
        let at = |s: &str| src[i..].starts_with(s.as_bytes());
        if at("//") {
            match src[i + 2..].iter().position(|&b| b == b'\n') {
                Some(p) => i = i + 2 + p,
                None => return n,
            }
            i += 1;
            continue;
        }
        if at("/*") {
            let mut d = 1;
            i += 2;
            while i < n && d > 0 {
                if src[i..].starts_with(b"/*") {
                    d += 1;
                    i += 2;
                } else if src[i..].starts_with(b"*/") {
                    d -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        // r"..", r#".."#, br".."
        let raw_at = if at("br") {
            Some(i + 2)
        } else if src[i] == b'r' {
            Some(i + 1)
        } else {
            None
        };
        if let Some(mut j) = raw_at {
            let hashes_start = j;
            while j < n && src[j] == b'#' {
                j += 1;
            }
            if j < n && src[j] == b'"' {
                let mut term = vec![b'"'];
                term.extend(std::iter::repeat_n(b'#', j - hashes_start));
                match src[j + 1..]
                    .windows(term.len())
                    .position(|w| w == term.as_slice())
                {
                    Some(p) => {
                        i = j + 1 + p + term.len();
                        continue;
                    }
                    None => return n,
                }
            }
        }
        if src[i] == b'"' {
            i += 1;
            while i < n {
                if src[i] == b'\\' {
                    i += 2;
                } else if src[i] == b'"' {
                    break;
                } else {
                    i += 1;
                }
            }
            i += 1;
            continue;
        }
        if src[i] == b'\'' {
            // A char literal: 'x', '\n', '\''. Anything else is a lifetime.
            let rest = &src[i..];
            let char_len = if rest.len() >= 4 && rest[1] == b'\\' && rest[3] == b'\'' {
                Some(4)
            } else if rest.len() >= 3
                && rest[1] != b'\\'
                && rest[1] != b'\''
                && rest[1] != b'\n'
                && rest[2] == b'\''
            {
                Some(3)
            } else {
                None
            };
            if let Some(len) = char_len {
                i += len;
                continue;
            }
        }
        match src[i] {
            b'{' => {
                body = true;
                depth += 1;
            }
            b'}' if body => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            b';' if !body => return i + 1,
            _ => {}
        }
        i += 1;
    }
    n
}

/// `#[cfg(test)]` items blanked out, newlines kept, so production-only policy cannot be met or
/// broken by test code.
pub fn strip_rust_cfg_test_items(source: &str) -> String {
    static P: OnceLock<Regex> = OnceLock::new();
    let p = re(&P, r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]");
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut from = 0;
    while let Some(m) = p.find_at(source, from) {
        let end = rust_cfg_item_end(bytes, m.end());
        for b in &mut out[m.start()..end] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
        from = end.max(m.end());
        if from >= source.len() {
            break;
        }
    }
    // Blanking multi-byte characters byte by byte leaves valid ASCII spaces.
    String::from_utf8_lossy(&out).into_owned()
}

/// Listener constructs in production code: a capability binds through sjel_server, never by
/// hand.
pub fn find_production_listener_constructs(source: &str) -> Vec<&'static str> {
    static SERVE: OnceLock<Regex> = OnceLock::new();
    static BIND: OnceLock<Regex> = OnceLock::new();
    let prod = strip_rust_cfg_test_items(source);
    let mut out = Vec::new();
    if re(&SERVE, r"\baxum::serve\s*\(").is_match(&prod) {
        out.push("axum::serve");
    }
    if re(&BIND, r"TcpListener::bind\s*\(").is_match(&prod) {
        out.push("TcpListener::bind");
    }
    out
}

/// `KEY=VALUE` lines of an env template: quotes and ` #` comments stripped, blanks and
/// comment lines skipped.
pub fn parse_env_template_lines(text: &str) -> Vec<(String, String)> {
    static P: OnceLock<Regex> = OnceLock::new();
    let p = re(&P, r"^([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$");
    let mut out = Vec::new();
    for raw in text.split('\n') {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let stripped = match line.find(" #") {
            Some(h) => line[..h].trim(),
            None => line,
        };
        let Some(c) = p.captures(stripped) else {
            continue;
        };
        let mut value = c[2].trim().to_owned();
        if value.len() >= 2
            && ((value.starts_with('\'') && value.ends_with('\''))
                || (value.starts_with('"') && value.ends_with('"')))
        {
            value = value[1..value.len() - 1].to_owned();
        }
        out.push((c[1].to_owned(), value));
    }
    out
}

fn is_env_value_placeholder(v: &str) -> bool {
    static P: OnceLock<Regex> = OnceLock::new();
    let v = v.trim();
    v.is_empty() || re(&P, r"(?i)^(?:<[^>]+>$|\$\{[^}]+\}$|required:|example|placeholder|changeme|change me|replace me)").is_match(v)
}

fn likely_raw_secret(v: &str) -> bool {
    static B64: OnceLock<Regex> = OnceLock::new();
    let v = v.trim();
    if v.is_empty() || is_env_value_placeholder(v) || v.chars().count() < 16 {
        return false;
    }
    let b64 = re(&B64, r"^[A-Za-z0-9+/=]+$").is_match(v)
        && v.bytes().any(|b| b.is_ascii_alphabetic())
        && v.bytes().any(|b| b.is_ascii_digit());
    // Token hashes belong in the overlay, never in a template.
    b64 || v.starts_with("$argon2")
}

/// Keys whose name looks sensitive and whose value looks like a real secret.
pub fn find_plaintext_secrets_in_env_template(text: &str) -> Vec<String> {
    static HINT: OnceLock<Regex> = OnceLock::new();
    let hint = re(
        &HINT,
        r"(?i)(PASS|PASSWORD|TOKEN|SECRET|KEY|CREDENTIAL|BEARER|HASH|SIGNATURE|PRIVATE)",
    );
    parse_env_template_lines(text)
        .into_iter()
        .filter(|(k, v)| hint.is_match(k) && likely_raw_secret(v))
        .map(|(k, _)| k)
        .collect()
}

pub struct RotEntry {
    pub slug: String,
    pub text: String,
    pub asserts_absent: Vec<String>,
    pub dir: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Rot {
    pub slug: String,
    pub path: String,
    pub missing: bool,
}

/// Path rot in decision prose: a backticked repo path that no longer resolves, or a path an
/// entry declares deliberately absent that exists again. A reference resolves against the
/// referencing document's own directory first, then the given bases.
pub fn find_decision_path_rot(
    entries: &[RotEntry],
    exists: &dyn Fn(&str) -> bool,
    bases: &[String],
) -> Vec<Rot> {
    static LINK: OnceLock<Regex> = OnceLock::new();
    static TICK: OnceLock<Regex> = OnceLock::new();
    static TLD: OnceLock<Regex> = OnceLock::new();
    let link = re(&LINK, r"\[[^\]]*\]\((?:https?|mailto):[^)]*\)");
    let tick = re(&TICK, r"`([A-Za-z0-9_./-]+/[A-Za-z0-9_./-]+)`");
    let tld = re(&TLD, r"\.(com|org|io|dev|net)/");
    let norm = |p: &str| {
        let mut stack: Vec<&str> = Vec::new();
        for seg in p.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    stack.pop();
                }
                s => stack.push(s),
            }
        }
        stack.join("/")
    };
    let mut out = Vec::new();
    for e in entries {
        let mut doc_bases: Vec<String> = Vec::new();
        if let Some(d) = &e.dir {
            doc_bases.push(format!("{d}/"));
        }
        doc_bases.extend(bases.iter().cloned());
        // A backticked slug labelling an external link names that resource, not a repo path.
        let scanned = link.replace_all(&e.text, " ");
        let mut named: Vec<String> = Vec::new();
        for c in tick.captures_iter(&scanned) {
            let p = c[1].split('#').next().unwrap_or("");
            let p = p.trim_end_matches(['.', ',', ';', ':', ')']);
            let p = p.strip_suffix('/').unwrap_or(p);
            if p.is_empty()
                || p.starts_with(['/', '~', '$', '<', '*'])
                || p.contains("://")
                || p.starts_with("origin/")
            {
                continue;
            }
            if tld.is_match(p) {
                continue;
            }
            if !named.iter().any(|n| n == p) {
                named.push(p.to_owned());
            }
        }
        for p in &named {
            if e.asserts_absent.contains(p) {
                continue;
            }
            if !doc_bases.iter().any(|b| exists(&norm(&format!("{b}{p}")))) {
                out.push(Rot {
                    slug: e.slug.clone(),
                    path: p.clone(),
                    missing: true,
                });
            }
        }
        for p in &e.asserts_absent {
            if exists(p) {
                out.push(Rot {
                    slug: e.slug.clone(),
                    path: p.clone(),
                    missing: false,
                });
            }
        }
    }
    out
}

/// `## Why this shape...` blocks of a README, up to the next `## ` heading or the end, with any
/// `<!-- asserts-absent: a, b -->` they carry.
pub fn collect_why_blocks(file: &str, text: &str) -> Vec<RotEntry> {
    static ABSENT: OnceLock<Regex> = OnceLock::new();
    let absent = re(&ABSENT, r"<!--\s*asserts-absent:([^>]*?)-->");
    let dir = file
        .rfind('/')
        .map(|i| file[..i].to_owned())
        .unwrap_or_default();
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(topic) = lines[i].strip_prefix("## Why this shape") else {
            i += 1;
            continue;
        };
        // The body runs to the next line that starts a `## ` heading.
        let mut j = i + 1;
        while j < lines.len() && !lines[j].starts_with("## ") {
            j += 1;
        }
        // The heading line's own newline is not part of the body; the last line's is.
        let mut body = lines[i + 1..j].join("\n");
        if j < lines.len() {
            body.push('\n');
        }
        let asserts_absent = absent
            .captures(&body)
            .map(|c| {
                c[1].split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let t = topic.trim();
        let slug = if t.is_empty() {
            file.to_owned()
        } else {
            let t = t.strip_prefix(':').map(str::trim_start).unwrap_or(t).trim();
            format!("{file} ({t})")
        };
        out.push(RotEntry {
            slug,
            text: body,
            asserts_absent,
            dir: Some(dir.clone()),
        });
        i = j;
    }
    out
}

/// Citations of a `decisions/<slug>` directory that no longer exists, skipping HTTP routes
/// (`api/`, a `${base}/` template, an http(s) URL) and the live `benchmarks/decisions/` dataset.
pub fn find_dangling_decision_refs(
    files: &[(String, String)],
    slug_exists: &dyn Fn(&str) -> bool,
) -> Vec<(String, String)> {
    static REF: OnceLock<Regex> = OnceLock::new();
    static ROUTE: OnceLock<Regex> = OnceLock::new();
    static BENCH: OnceLock<Regex> = OnceLock::new();
    let r = re(&REF, r"decisions/([a-z0-9][a-z0-9-]*)");
    let route = re(&ROUTE, r#"(api/|\$\{[^}]*\}/|https?://[^\s"'`]*/)$"#);
    let bench = re(&BENCH, r"(?:^|[^A-Za-z0-9_-])benchmarks/$");
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for (path, text) in files {
        // A Rust `#[cfg(test)]` item is the same category as a `*.test.ts` file, which the
        // caller already excludes: code that exists to exercise a rule must be free to name
        // the thing the rule forbids. The caller cannot see this one, because it is content
        // rather than a filename, and this check's own fixtures live in such an item -- which
        // is why it used to fail on itself forever.
        let stripped;
        let text: &str = if path.ends_with(".rs") {
            stripped = strip_rust_cfg_test_items(text);
            &stripped
        } else {
            text.as_str()
        };
        for c in r.captures_iter(text) {
            let at = c.get(0).map_or(0, |m| m.start());
            // 64 UTF-16 units before the match in doctor.ts; chars here, the same on ASCII.
            let before: String = text[..at]
                .chars()
                .rev()
                .take(64)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if route.is_match(&before) || bench.is_match(&before) {
                continue;
            }
            // Only a path-shaped reference is a citation. An English phrase that joins two
            // nouns with a slash -- "decisions and groups", as the vendored keel-lite comment
            // had it -- is not one, and writing the rule down in the files it governs would
            // otherwise be impossible without tripping it. So a reference counts only when it
            // is the head of a path (the slug is followed by a slash) or is a bare token the
            // author wrapped in backticks to say that is what they meant.
            let m = c.get(0).expect("whole match");
            let followed_by_slash = text[m.end()..].starts_with('/');
            let backticked = text[..at].ends_with('`');
            if !followed_by_slash && !backticked {
                continue;
            }
            let slug = c[1].to_owned();
            let key = format!("{path}::{slug}");
            if seen.contains(&key) || slug_exists(&slug) {
                continue;
            }
            seen.insert(key);
            out.push((path.clone(), slug));
        }
    }
    out
}

pub fn format_version(describe: &str, commit_date: &str) -> String {
    if describe.is_empty() {
        "(unknown — not a git checkout?)".to_owned()
    } else if commit_date.is_empty() {
        describe.to_owned()
    } else {
        format!("{describe} ({commit_date})")
    }
}

pub fn format_fetch_age(fetch: Option<i64>, now: i64) -> String {
    let Some(f) = fetch else {
        return "no fetch recorded".to_owned();
    };
    let age = (now - f).max(0);
    if age < 60 {
        return "fetched just now".to_owned();
    }
    let minutes = age / 60;
    if minutes < 60 {
        return format!("fetched {minutes} minute(s) ago");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("fetched {hours} hour(s) ago");
    }
    format!("fetched {} day(s) ago", hours / 24)
}

/// `20260906T210709Z`, the stamp backup.sh writes, as UTC epoch seconds. Any other shape is no
/// usable receipt, never a guess.
pub fn parse_receipt_timestamp(stamp: &str) -> Option<i64> {
    static P: OnceLock<Regex> = OnceLock::new();
    let c = re(&P, r"^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$").captures(stamp)?;
    let n = |i: usize| c[i].parse::<i64>().unwrap_or(-1);
    let (y, mo, d, h, mi, s) = (n(1), n(2), n(3), n(4), n(5), n(6));
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s)
}

/// Days since 1970-01-01 for a proleptic Gregorian date; out-of-range days roll over as
/// `Date.UTC` rolls them (31 February is 3 March).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `Date.parse` for the ISO-8601 forms receipts carry: `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`
/// and a bare `YYYY-MM-DD` (UTC). Epoch milliseconds, or None where Date.parse gives NaN.
pub fn parse_iso_ms(s: &str) -> Option<f64> {
    static P: OnceLock<Regex> = OnceLock::new();
    let p = re(
        &P,
        r"^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,9}))?)?(Z|[+-]\d{2}:?\d{2})?)?$",
    );
    let c = p.captures(s.trim())?;
    let n = |i: usize| {
        c.get(i)
            .map_or(0, |m| m.as_str().parse::<i64>().unwrap_or(0))
    };
    let (y, mo, d) = (n(1), n(2), n(3));
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || n(4) > 24 || n(5) > 59 || n(6) > 59 {
        return None;
    }
    let mut secs = days_from_civil(y, mo, d) * 86_400 + n(4) * 3600 + n(5) * 60 + n(6);
    if let Some(tz) = c.get(8).map(|m| m.as_str()).filter(|t| *t != "Z") {
        let sign = if tz.starts_with('-') { -1 } else { 1 };
        let digits: String = tz[1..].chars().filter(char::is_ascii_digit).collect();
        let (h, m) = (
            digits[..2].parse::<i64>().unwrap_or(0),
            digits[2..].parse::<i64>().unwrap_or(0),
        );
        secs -= sign * (h * 3600 + m * 60);
    } else if c.get(4).is_some() && c.get(8).is_none() {
        // A date-time with no offset is LOCAL time in JavaScript. Receipts write Z; this path
        // is only reached by a hand-written one, and reading it as UTC is off by the zone.
    }
    let frac = c.get(7).map_or(0.0, |m| {
        format!("0.{}", m.as_str()).parse::<f64>().unwrap_or(0.0)
    });
    Some((secs as f64 + frac) * 1000.0)
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum AgeState {
    Never,
    Overdue,
    Due,
    Unknown,
    Ok,
}

/// The two thresholds mean different things, and `never` outranks both.
pub fn backup_age_state(age: Option<i64>, advise: f64, stale: f64) -> AgeState {
    let Some(age) = age else {
        return AgeState::Never;
    };
    let days = (age / 86_400) as f64;
    if stale.is_finite() && days >= stale {
        return AgeState::Overdue;
    }
    if advise.is_finite() && days >= advise {
        return AgeState::Due;
    }
    if !stale.is_finite() && !advise.is_finite() {
        return AgeState::Unknown;
    }
    AgeState::Ok
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Level {
    Ok,
    Warn,
    Bad,
}

/// A receipt's archive at a local destination: gone or short is a failure, an iCloud-evicted
/// (`dataless`) file is a warning this offline check cannot settle.
pub fn classify_archive_at_target(
    exists: bool,
    size: Option<u64>,
    flags: &str,
    receipt_bytes: f64,
) -> (Level, String) {
    if !exists {
        return (
            Level::Bad,
            "the archive the receipt names is not at the destination".to_owned(),
        );
    }
    let size_matches = size.is_some_and(|s| s as f64 == receipt_bytes);
    if flags.split(',').any(|f| f == "dataless") {
        let note = if size_matches {
            "its name and size remain local"
        } else {
            "even its reported size differs from the receipt"
        };
        return (
            Level::Warn,
            format!(
                "the archive is offloaded: {note}, but recovery requires an online download and verification; this offline check cannot prove the cloud copy is recoverable"
            ),
        );
    }
    if !size_matches {
        let shown = size.map_or("null".to_owned(), |s| s.to_string());
        return (
            Level::Bad,
            format!(
                "the archive holds {shown} bytes, the receipt recorded {}",
                super::js_num(receipt_bytes)
            ),
        );
    }
    (
        Level::Ok,
        format!(
            "{} bytes, present at the destination",
            super::js_num(receipt_bytes)
        ),
    )
}

/// A failed attempt is reported even while the last good receipt still looks fresh.
pub fn attempt_finding(exit_code: f64, at_epoch: f64, detail: &str, now: f64) -> String {
    let since = (now - at_epoch).max(0.0);
    let reason = if detail.trim().is_empty() {
        String::new()
    } else {
        format!(": {}", detail.trim())
    };
    format!(
        "the last attempt FAILED {}h ago (exit {}){reason}",
        to_fixed(since / 3600.0, 1),
        super::js_num(exit_code)
    )
}

/// The capability a LaunchAgent file belongs to, under com.sjel or the pre-2026-09-26 com.axon.
pub fn launchd_unit_capability(file: &str) -> Option<String> {
    let stem = file.strip_suffix(".plist")?;
    ["com.sjel.", "com.axon."]
        .iter()
        .find_map(|p| stem.strip_prefix(p))
        .map(str::to_owned)
}

pub struct Job {
    // Parsed because launchctl prints it and the parser's tests pin it; no section reads it.
    #[allow(dead_code)]
    pub pid: Option<i64>,
    pub last_exit: Option<i64>,
}

/// `launchctl list`: PID, Status, Label per tab-separated line, header and dashes and all.
pub fn parse_launchd_jobs(text: &str) -> HashMap<String, Job> {
    let num = |c: &str| {
        let t = c.trim();
        let digits = t.strip_prefix('-').unwrap_or(t);
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .then(|| t.parse::<i64>().ok())
            .flatten()
    };
    let mut jobs = HashMap::new();
    for line in text.split('\n') {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 3 {
            continue;
        }
        let label = cols[2].trim();
        if label.is_empty() || label == "Label" {
            continue;
        }
        jobs.insert(
            label.to_owned(),
            Job {
                pid: num(cols[0]),
                last_exit: num(cols[1]),
            },
        );
    }
    jobs
}

pub struct LaunchdSchedule {
    pub interval: Option<f64>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

pub fn parse_launchd_schedule(plist: &str) -> LaunchdSchedule {
    static INTERVAL: OnceLock<Regex> = OnceLock::new();
    static OUT: OnceLock<Regex> = OnceLock::new();
    static ERR: OnceLock<Regex> = OnceLock::new();
    let s = |r: &Regex| r.captures(plist).map(|c| c[1].to_owned());
    LaunchdSchedule {
        interval: re(
            &INTERVAL,
            r"<key>StartInterval</key>\s*<integer>(\d+)</integer>",
        )
        .captures(plist)
        .and_then(|c| c[1].parse::<f64>().ok()),
        stdout: s(re(
            &OUT,
            r"<key>StandardOutPath</key>\s*<string>([^<]*)</string>",
        )),
        stderr: s(re(
            &ERR,
            r"<key>StandardErrorPath</key>\s*<string>([^<]*)</string>",
        )),
    }
}

/// An age in the unit a person would use.
pub fn format_age(seconds: f64) -> String {
    if seconds < 90.0 {
        format!("{}s", super::js_num(js_round(seconds)))
    } else if seconds < 3600.0 {
        format!("{}m", super::js_num(js_round(seconds / 60.0)))
    } else if seconds < 172_800.0 {
        format!("{}h", to_fixed(seconds / 3600.0, 1))
    } else {
        format!("{}d", to_fixed(seconds / 86_400.0, 1))
    }
}

pub struct Producer {
    pub name: String,
    pub unit_installed: bool,
    pub loaded: bool,
    pub last_exit: Option<i64>,
    pub interval: Option<f64>,
    pub last_output_age: Option<f64>,
}

/// Did a scheduled producer run. A failed run outranks a healthy age (a fast failure still
/// touches the log); one interval late warns, three is a fault.
pub fn classify_scheduled_producer(p: &Producer) -> (Level, String) {
    let name = &p.name;
    let seen = p
        .last_output_age
        .map_or("no output on record".to_owned(), |a| {
            format!("last output {} ago", format_age(a))
        });
    if !p.unit_installed {
        return (
            Level::Ok,
            format!("{name} — no unit installed; the boot-persistence check above owns that"),
        );
    }
    if !p.loaded {
        return (Level::Warn, format!("{name} — its unit is installed and launchd has not loaded it, so the timer cannot fire ({seen})"));
    }
    if let Some(e) = p.last_exit.filter(|e| *e != 0) {
        return (
            Level::Bad,
            format!("{name} — its last scheduled run exited {e} ({seen})"),
        );
    }
    let Some(interval) = p.interval else {
        return (Level::Warn, format!("{name} — its unit declares no StartInterval, so there is no cadence to judge it against ({seen})"));
    };
    let every = format_age(interval);
    let Some(age_s) = p.last_output_age else {
        return (
            Level::Warn,
            format!(
                "{name} — runs every {every} and has written no output this machine still holds"
            ),
        );
    };
    let age = format_age(age_s);
    if age_s >= interval * 3.0 {
        return (Level::Bad, format!("{name} — runs every {every} and has produced nothing for {age}; it has missed at least two runs"));
    }
    if age_s >= interval {
        return (
            Level::Warn,
            format!("{name} — runs every {every}, last produced {age} ago"),
        );
    }
    (
        Level::Ok,
        format!("{name} — runs every {every}, produced {age} ago"),
    )
}

pub const PROBE_TIMEOUT_MS: f64 = 5000.0;

#[derive(Debug, PartialEq)]
pub enum Target {
    Probe {
        id: String,
        url: String,
        timeout_ms: f64,
    },
    Skip {
        id: String,
        why: String,
    },
}

/// Enough of WHATWG URL parsing to answer what `new URL()` answered here: the scheme, whether
/// the authority carries credentials, and whether an http(s) URL parses at all.
fn parse_url(url: &str) -> Option<(String, bool)> {
    static SCHEME: OnceLock<Regex> = OnceLock::new();
    let c = re(&SCHEME, r"^([A-Za-z][A-Za-z0-9+.-]*):(.*)$").captures(url)?;
    let scheme = c[1].to_ascii_lowercase();
    let rest = &c[2];
    let special = matches!(
        scheme.as_str(),
        "http" | "https" | "ws" | "wss" | "ftp" | "file"
    );
    let Some(after) = rest.strip_prefix("//").or_else(|| {
        if special {
            rest.strip_prefix('/').or(Some(rest))
        } else {
            None
        }
    }) else {
        return Some((scheme, false));
    };
    let authority = &after[..after.find(['/', '?', '#']).unwrap_or(after.len())];
    let (creds, host) = match authority.rfind('@') {
        Some(i) => (true, &authority[i + 1..]),
        None => (false, authority),
    };
    if host.contains(char::is_whitespace) {
        return None;
    }
    let hostname = if let Some(v6) = host.strip_prefix('[') {
        let close = v6.find(']')?;
        let inner = &v6[..close];
        if inner.is_empty()
            || !inner
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.')
        {
            return None;
        }
        &host[..close + 2]
    } else {
        host.split(':').next().unwrap_or("")
    };
    if special && scheme != "file" && hostname.is_empty() {
        return None;
    }
    if let Some(port) = host
        .strip_prefix(hostname)
        .and_then(|p| p.strip_prefix(':'))
    {
        if !port.is_empty()
            && (!port.bytes().all(|b| b.is_ascii_digit())
                || port.parse::<u32>().map_or(true, |p| p > 65535))
        {
            return None;
        }
    }
    Some((scheme, creds))
}

/// Which declared systems to probe, and why the rest are skipped. A private system's url comes
/// from the overlay by the same id; the overlay can opt one out or raise its timeout.
pub fn resolve_probe_targets(
    systems: &[(String, toml::Value)],
    overlay: &toml::Table,
) -> Vec<Target> {
    let mut out = Vec::new();
    for (id, entry) in systems {
        let Some(entry) = entry.as_table() else {
            continue;
        };
        let ov = overlay.get(id).and_then(toml::Value::as_table);
        let ov_str = |k: &str| {
            ov.and_then(|t| t.get(k))
                .and_then(toml::Value::as_str)
                .unwrap_or("")
        };
        let skip = |why: &str| Target::Skip {
            id: id.clone(),
            why: why.to_owned(),
        };
        if ov_str("probe") == "no" {
            out.push(skip("overlay declares probe = \"no\""));
            continue;
        }
        let mut url = entry
            .get("url")
            .and_then(toml::Value::as_str)
            .unwrap_or("")
            .to_owned();
        if url == "overlay:systems.local.toml" {
            if ov_str("url").is_empty() {
                out.push(skip("private system, no url in the overlay"));
                continue;
            }
            url = ov_str("url").to_owned();
        }
        if url.is_empty() {
            out.push(skip("no url declared"));
            continue;
        }
        if url == "local" {
            out.push(skip("url = \"local\" — not an endpoint"));
            continue;
        }
        let Some((scheme, creds)) = parse_url(&url) else {
            out.push(skip("url is not parseable"));
            continue;
        };
        if scheme != "http" && scheme != "https" {
            out.push(skip(&format!("{scheme} endpoint — only http(s) is probed")));
            continue;
        }
        if creds {
            out.push(skip("url embeds credentials — not probed"));
            continue;
        }
        let declared = super::js_number(ov.and_then(|t| t.get("probe_timeout_ms")));
        let timeout_ms = if declared.is_finite() && declared > 0.0 {
            declared
        } else {
            PROBE_TIMEOUT_MS
        };
        out.push(Target::Probe {
            id: id.clone(),
            url,
            timeout_ms,
        });
    }
    out
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Outcome {
    Refused,
    Timeout,
    Unavailable,
}

/// A failed curl probe by its exit code: 7 is a refused connection, 28 a timeout, and
/// everything else (DNS, TLS, no route) unavailable rather than guessed at. The TypeScript
/// version classified fetch() errors into the same three.
pub fn classify_probe_outcome(curl_exit: i32) -> Outcome {
    match curl_exit {
        7 => Outcome::Refused,
        28 => Outcome::Timeout,
        _ => Outcome::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    // Moved from tools/doctor.test.ts, describe block by describe block.
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    // ---- production Rust server policy

    #[test]
    fn nested_binary_roots_remain_inside_the_scan() {
        let root = std::env::temp_dir().join(format!("sjel-doctor-rust-{}", std::process::id()));
        std::fs::create_dir_all(root.join("server")).unwrap();
        std::fs::write(root.join("lib.rs"), "pub fn library() {}\n").unwrap();
        std::fs::write(root.join("server/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("server/notes.txt"), "not Rust\n").unwrap();
        let found: Vec<String> = find_rust_sources(&root)
            .iter()
            .map(|p| p.strip_prefix(&root).unwrap().display().to_string())
            .collect();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(found, ["lib.rs", "server/main.rs"]);
    }

    #[test]
    fn test_only_listener_constructs_are_excluded() {
        let source = "
fn build_router() -> Router { Router::new() }
fn main() { sjel_server::serve_local(\"fixture\", 1234, build_router()); }

#[cfg(test)]
mod tests {
  async fn serve() {
    let listener = tokio::net::TcpListener::bind(\"127.0.0.1:0\").await.unwrap();
    axum::serve(listener, build_router()).await.unwrap();
  }
}";
        assert!(find_production_listener_constructs(source).is_empty());
        assert!(strip_rust_cfg_test_items(source).contains("sjel_server::serve_local"));
    }

    #[test]
    fn production_listener_constructs_remain_findings() {
        let source = "
fn build_router() -> Router { Router::new() }
async fn main() {
  let listener = tokio::net::TcpListener::bind(\"127.0.0.1:0\").await.unwrap();
  axum::serve(listener, build_router()).await.unwrap();
}";
        assert_eq!(
            find_production_listener_constructs(source),
            ["axum::serve", "TcpListener::bind"]
        );
    }

    #[test]
    fn a_test_module_cannot_hide_a_production_listener() {
        let source = "
async fn main() {
  let listener = tokio::net::TcpListener::bind(\"127.0.0.1:0\").await.unwrap();
}

#[cfg(test)]
mod tests {
  async fn serve() { axum::serve(listener, app).await.unwrap(); }
}";
        assert_eq!(
            find_production_listener_constructs(source),
            ["TcpListener::bind"]
        );
    }

    // ---- checkStateMountCoverage

    #[test]
    fn state_mount_coverage() {
        let ids: BTreeSet<String> = s(&["mach-mono", "knowledge-base", "backup-target"])
            .into_iter()
            .collect();
        assert_eq!(
            check_state_mount_coverage(&s(&["mach-mono", "knowledge-base"]), &ids),
            (s(&["mach-mono", "knowledge-base"]), vec![])
        );
        let ids: BTreeSet<String> = s(&["mach-mono"]).into_iter().collect();
        assert_eq!(
            check_state_mount_coverage(&s(&["mach-mono", "some-new-tool"]), &ids),
            (s(&["mach-mono"]), s(&["some-new-tool"]))
        );
        assert_eq!(check_state_mount_coverage(&[], &ids), (vec![], vec![]));
    }

    // ---- extractSiblingRepoRefs

    #[test]
    fn sibling_repo_refs() {
        let x = |t: &str| extract_sibling_repo_refs(t, &[]);
        assert_eq!(
            x("see ~/Developer/mach-mono for the Swift monorepo"),
            ["mach-mono"]
        );
        assert_eq!(
            x(r#"{"source":{"path":"~/Developer/private-knowledge"}}"#),
            ["private-knowledge"]
        );
        assert_eq!(
            x(r#""path": "~/Developer/Personal/Knowledge-Base""#),
            ["Knowledge-Base"]
        );
        assert_eq!(x("~/Developer/Collab/VBB"), ["VBB"]);
        assert_eq!(x("$HOME/Developer/pi-agent"), ["pi-agent"]);
        assert_eq!(
            x("~/Developer/Axon and ~/Developer/axon-overlay and $HOME/Developer/mach-mono"),
            ["Axon", "axon-overlay", "mach-mono"]
        );
        assert!(x("nothing relevant here, just prose").is_empty());
        let me = s(&["example-overlay"]);
        assert!(
            extract_sibling_repo_refs("see ~/Developer/example-overlay/config", &me).is_empty()
        );
        assert!(
            extract_sibling_repo_refs("~/Developer/example-repo", &s(&["example-repo"])).is_empty()
        );
        assert_eq!(
            extract_sibling_repo_refs(
                "~/Developer/example-overlay/config and ~/Developer/mach-mono",
                &me
            ),
            ["mach-mono"]
        );
        assert_eq!(x("~/Developer/example-overlay/config"), ["config"]);
        assert!(x("/opt/Developer/something-else").is_empty());
    }

    // ---- formatVersion, formatFetchAge

    #[test]
    fn versions_and_fetch_ages() {
        assert_eq!(
            format_version("v1.2-3-gabc1234", "2026-07-16"),
            "v1.2-3-gabc1234 (2026-07-16)"
        );
        assert_eq!(
            format_version("abc1234", "2026-07-16"),
            "abc1234 (2026-07-16)"
        );
        assert_eq!(
            format_version("abc1234-dirty", "2026-07-16"),
            "abc1234-dirty (2026-07-16)"
        );
        assert_eq!(format_version("abc1234", ""), "abc1234");
        assert_eq!(
            format_version("", "2026-07-16"),
            "(unknown — not a git checkout?)"
        );
        let now = 1_800_000_000;
        assert_eq!(format_fetch_age(Some(now - 5), now), "fetched just now");
        assert_eq!(
            format_fetch_age(Some(now - 25 * 60), now),
            "fetched 25 minute(s) ago"
        );
        assert_eq!(
            format_fetch_age(Some(now - 3 * 3600 - 40), now),
            "fetched 3 hour(s) ago"
        );
        assert_eq!(
            format_fetch_age(Some(now - 2 * 86_400 - 3600), now),
            "fetched 2 day(s) ago"
        );
        assert_eq!(format_fetch_age(None, now), "no fetch recorded");
        assert_eq!(format_fetch_age(Some(now + 120), now), "fetched just now");
    }

    // ---- findDecisionPathRot

    fn entry(slug: &str, text: &str, absent: &[&str]) -> RotEntry {
        RotEntry {
            slug: slug.into(),
            text: text.into(),
            asserts_absent: s(absent),
            dir: None,
        }
    }

    fn rot(entries: &[RotEntry], real: &[&str], bases: &[&str]) -> Vec<(String, String, bool)> {
        let real = s(real);
        find_decision_path_rot(entries, &|p| real.iter().any(|r| r == p), &s(bases))
            .into_iter()
            .map(|r| (r.slug, r.path, r.missing))
            .collect()
    }

    #[test]
    fn decision_path_rot() {
        let st = |a: &str, b: &str, c: bool| (a.to_owned(), b.to_owned(), c);
        assert_eq!(
            rot(
                &[entry("stale", "consumes `apps/dashboard` over HTTP", &[])],
                &["dashboard"],
                &[""]
            ),
            [st("stale", "apps/dashboard", true)]
        );
        assert_eq!(
            rot(
                &[entry(
                    "forbids",
                    "no `tools/topology` binary",
                    &["tools/topology"]
                )],
                &["tools/topology"],
                &[""]
            ),
            [st("forbids", "tools/topology", false)]
        );
        assert!(rot(
            &[entry(
                "ok",
                "no `tools/topology` binary",
                &["tools/topology"]
            )],
            &[],
            &[""]
        )
        .is_empty());
        assert!(rot(
            &[entry("ok", "an arm in `sources/mod.rs`", &[])],
            &["capabilities/scouting/src/sources/mod.rs"],
            &["", "capabilities/scouting/src/"]
        )
        .is_empty());
        assert!(rot(&[entry("ok", "`https://a.com/b` `/usr/local/bin/x` `origin/main` `<vault>/Atlas` `~/Developer/x`", &[])], &[], &[""]).is_empty());
        assert!(rot(&[entry("ok", "[`org/model-name` at the audited commit](https://huggingface.co/org/model-name/tree/abc)", &[])], &[], &[""]).is_empty());
        assert_eq!(
            rot(
                &[entry(
                    "stale",
                    "[docs](https://a.com/b) and `apps/dashboard`",
                    &[]
                )],
                &[],
                &[""]
            ),
            [st("stale", "apps/dashboard", true)]
        );
    }

    // ---- collectWhyBlocks

    #[test]
    fn why_blocks() {
        let doc = [
            "# punctuality",
            "",
            "Some prose naming `capabilities/other/thing.rs`.",
            "",
            "## Why this shape: Rust over a second engine",
            "",
            "It reads parquet from `src/aggregate.rs`.",
            "",
            "## Considered and declined",
            "",
            "Naming `nope/gone.rs` here must not be swept.",
            "",
        ]
        .join("\n");
        let blocks = collect_why_blocks("capabilities/punctuality/README.md", &doc);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].text.contains("src/aggregate.rs"));
        assert!(
            !blocks[0].text.contains("nope/gone.rs")
                && !blocks[0].text.contains("capabilities/other/thing.rs")
        );
        assert_eq!(blocks[0].dir.as_deref(), Some("capabilities/punctuality"));
        assert_eq!(
            collect_why_blocks("a/README.md", &doc)[0].slug,
            "a/README.md (Rust over a second engine)"
        );
        let b = collect_why_blocks("a/README.md", "## Why this shape: x\n\n<!-- asserts-absent: apps/dashboard, tools/topology -->\nno `apps/dashboard` here.\n");
        assert_eq!(
            b[0].asserts_absent,
            s(&["apps/dashboard", "tools/topology"])
        );
        assert!(
            collect_why_blocks("a/README.md", "# a\n\nplain prose with `some/path.rs`.\n")
                .is_empty()
        );
        assert_eq!(
            collect_why_blocks(
                "a/README.md",
                "## Why this shape: one\n\na\n\n## Why this shape: two\n\nb\n"
            )
            .len(),
            2
        );
    }

    // ---- env templates

    #[test]
    fn env_templates() {
        let parsed = parse_env_template_lines(
            &[
                "FOO=bar",
                "BAZ=\"quoted value\" # inline comment",
                "  # ignored comment",
                "",
                "X= # value can be blank",
            ]
            .join("\n"),
        );
        assert_eq!(
            parsed,
            [
                ("FOO".into(), "bar".into()),
                ("BAZ".into(), "quoted value".into()),
                ("X".into(), String::new())
            ]
        );
        let leaks = find_plaintext_secrets_in_env_template(
            &[
                "DOMAIN=example.local",
                "ADMIN_TOKEN=$argon2id$v=19$m=65536,t=3,p=4$...",
                "POSTGRES_PASSWORD=<required: private password>",
                "HA_TOKEN=<required: private token>",
                "DB_KEY=abc",
            ]
            .join("\n"),
        );
        assert_eq!(leaks, ["ADMIN_TOKEN"]);
    }

    // ---- findDanglingDecisionRefs

    fn dangling(path: &str, text: &str) -> Vec<(String, String)> {
        find_dangling_decision_refs(&[(path.to_owned(), text.to_owned())], &|s| {
            s == "root-is-the-spine-three-nouns"
        })
    }

    #[test]
    fn dangling_decision_refs() {
        let d = |f: &str, slug: &str| vec![(f.to_owned(), slug.to_owned())];
        assert!(dangling(
            "capabilities/finance/src/server.rs",
            "\"/api/decisions/run\""
        )
        .is_empty());
        assert!(dangling("tools/demo-seed.ts", "post(`${base}/decisions/run`, {})").is_empty());
        assert_eq!(
            dangling(
                "README.md",
                "`/api/decisions/run` and `decisions/gone/README.md`"
            ),
            d("README.md", "gone")
        );
        assert_eq!(
            dangling("docs/guide.md", "See [x](./decisions/gone/README.md)."),
            d("docs/guide.md", "gone")
        );
        assert_eq!(
            dangling(
                "ARCHITECTURE.md",
                "- `Knowledge-Base/decisions/gone/README.md`"
            ),
            d("ARCHITECTURE.md", "gone")
        );
        assert!(dangling("tools/x.ts", "http://127.0.0.1:8084/decisions/run").is_empty());
        assert_eq!(
            dangling("README.md", "See `decisions/dissolved-entry/README.md`."),
            d("README.md", "dissolved-entry")
        );
        assert!(dangling("research/local-decision-models.md", "`benchmarks/decisions/results/` and `research/benchmarks/decisions/cases.jsonl` and `[benchmarks/decisions/results/](benchmarks/decisions/results/)`").is_empty());
        assert!(dangling(
            "README.md",
            "See `CONTRIBUTING.md#three-architectural-nouns`."
        )
        .is_empty());
        assert_eq!(
            dangling("a.md", "decisions/gone and again decisions/gone/README.md").len(),
            1
        );
        assert_eq!(
            dangling("tools/gen.sh", "echo \"See decisions/gone/README.md.\""),
            d("tools/gen.sh", "gone")
        );
        // Prose is not a citation. "decisions/groups" means "decisions and groups", and it is
        // also how this rule has to be written down in the tree it governs -- so neither the
        // vendored comment that first tripped it nor these three doc files may be flagged.
        assert!(dangling(
            "conductors/keel-lite.ts",
            "\t/** Forget decisions/groups whose blocks or groups vanished. */"
        )
        .is_empty());
        assert!(dangling("upstreams.toml", "reads \"decisions and groups\" where upstream wrote \"decisions/groups\": the sweep parses that").is_empty());
        assert!(dangling(
            "LICENSE",
            "citations of decisions/groups, and of decisions/ especially"
        )
        .is_empty());
        // A bare token the author backticked still is one: they meant the directory.
        assert_eq!(
            dangling(
                "README.md",
                "was recorded at `decisions/gone` before it dissolved"
            ),
            d("README.md", "gone")
        );
        // A Rust `#[cfg(test)]` fixture is blanked inside the scan, which is what clears this
        // check on its own file rather than leaving it red forever -- and is why the exclusion
        // has to live in the scan rather than in the caller's filename filter.
        assert!(dangling(
            "tools/x.rs",
            "pub fn f() {}\n#[cfg(test)]\nmod tests {\n    const F: &str = \"decisions/gone/README.md\";\n}\n"
        )
        .is_empty());
        assert_eq!(
            dangling(
                "tools/x.rs",
                "pub fn f() { let p = \"decisions/gone/README.md\"; }\n#[cfg(test)]\nmod tests {}\n"
            ),
            d("tools/x.rs", "gone")
        );
    }

    // ---- isSweepExempt, whyBlockBases

    #[test]
    fn sweep_exemptions() {
        let generated = "# Fixture Architecture\n\n> Auto-generated by tools/generate-fixture.sh. Do not edit manually.\n\n`~/Developer/example-repo/x`\n";
        assert!(is_sweep_exempt("FIXTURE.md", generated));
        assert!(!is_sweep_exempt(
            "notes.md",
            &format!(
                "{}This file is auto-generated, honest.\n",
                "filler\n".repeat(20)
            )
        ));
        assert!(is_sweep_exempt(
            "fixture.local.toml.example",
            "overlay = \"~/Developer/example-overlay\"\n"
        ));
        for f in [
            "tools/lib/paths.sh",
            "tools/install.sh",
            "tools/doctor.test.ts",
        ] {
            assert!(is_sweep_exempt(f, "~/Developer/example-overlay"));
        }
        assert!(!is_sweep_exempt(
            "axon.toml",
            "overlay = \"~/Developer/example-overlay\"\n"
        ));
        assert!(!is_sweep_exempt(
            "docs/example.md",
            "~/Developer/example-repo"
        ));
    }

    #[test]
    fn why_block_bases_come_from_tracked_files() {
        assert_eq!(
            why_block_bases(&s(&[
                "capabilities/example/README.md",
                "capabilities/example/src/lib.rs",
                "capabilities/example/src/sources/mod.rs"
            ])),
            s(&[
                "",
                "capabilities/",
                "capabilities/example/",
                "capabilities/example/src/"
            ])
        );
        assert!(!why_block_bases(&s(&[
            "Packs/example/pack.toml",
            "Packs/example/skills/thing/SKILL.md"
        ]))
        .contains(&"Packs/example/src/".to_owned()));
        let b = why_block_bases(&s(&[
            "dashboard/src/routes/+page.svelte",
            "schemas/service.toml.example",
            "tools/doctor.ts",
        ]));
        for x in ["dashboard/", "schemas/", "tools/"] {
            assert!(b.contains(&x.to_owned()));
        }
        assert_eq!(
            why_block_bases(&s(&["dashboard/package.json"])),
            s(&["", "dashboard/"])
        );
        assert_eq!(why_block_bases(&s(&["README.md", "axon.toml"])), s(&[""]));
    }

    // ---- systems reachability

    fn sys(src: &str) -> Vec<(String, toml::Value)> {
        super::super::ordered(src).unwrap()
    }

    fn probe(id: &str, url: &str, t: f64) -> Target {
        Target::Probe {
            id: id.into(),
            url: url.into(),
            timeout_ms: t,
        }
    }

    fn skip(id: &str, why: &str) -> Target {
        Target::Skip {
            id: id.into(),
            why: why.into(),
        }
    }

    #[test]
    fn probe_targets() {
        let none = toml::Table::new();
        let ov = |s: &str| s.parse::<toml::Table>().unwrap();
        assert_eq!(
            resolve_probe_targets(&sys("[pub]\nurl = \"https://example.test/health\""), &none),
            [probe(
                "pub",
                "https://example.test/health",
                PROBE_TIMEOUT_MS
            )]
        );
        assert_eq!(
            resolve_probe_targets(
                &sys("[priv]\nurl = \"overlay:systems.local.toml\""),
                &ov("[priv]\nurl = \"https://private.test\"")
            ),
            [probe("priv", "https://private.test", PROBE_TIMEOUT_MS)]
        );
        assert_eq!(
            resolve_probe_targets(&sys("[priv]\nurl = \"overlay:systems.local.toml\""), &none),
            [skip("priv", "private system, no url in the overlay")]
        );
        assert_eq!(
            resolve_probe_targets(
                &sys("[thing]\nurl = \"https://example.test\""),
                &ov("[thing]\nprobe = \"no\"\nurl = \"https://example.test\"")
            ),
            [skip("thing", "overlay declares probe = \"no\"")]
        );
        assert_eq!(
            resolve_probe_targets(&sys("[axon]\nurl = \"local\""), &none),
            [skip("axon", "url = \"local\" — not an endpoint")]
        );
        assert_eq!(
            resolve_probe_targets(&sys("[broken]\nurl = \"http://[not a url\""), &none),
            [skip("broken", "url is not parseable")]
        );
        assert_eq!(
            resolve_probe_targets(&sys("[box]\nurl = \"ssh://host.test\""), &none),
            [skip("box", "ssh endpoint — only http(s) is probed")]
        );
        let leaky = resolve_probe_targets(
            &sys("[leaky]\nurl = \"https://user:pw@example.test\""),
            &none,
        );
        assert_eq!(
            leaky,
            [skip("leaky", "url embeds credentials — not probed")]
        );
        assert!(!format!("{leaky:?}").contains("pw@"));
        assert_eq!(
            resolve_probe_targets(&sys("[bare]\nkind = \"service\""), &none),
            [skip("bare", "no url declared")]
        );
        assert_eq!(
            resolve_probe_targets(
                &sys("[slow]\nurl = \"https://slow.test\""),
                &ov("[slow]\nprobe_timeout_ms = 12000")
            ),
            [probe("slow", "https://slow.test", 12000.0)]
        );
        for bad in ["\"soon\"", "0", "-1"] {
            assert_eq!(
                resolve_probe_targets(
                    &sys("[s]\nurl = \"https://x.test\""),
                    &ov(&format!("[s]\nprobe_timeout_ms = {bad}"))
                ),
                [probe("s", "https://x.test", PROBE_TIMEOUT_MS)]
            );
        }
    }

    #[test]
    fn probe_failures_classify_by_curl_exit() {
        assert_eq!(classify_probe_outcome(28), Outcome::Timeout);
        assert_eq!(classify_probe_outcome(7), Outcome::Refused);
        assert_eq!(
            classify_probe_outcome(60),
            Outcome::Unavailable,
            "a TLS failure"
        );
        assert_eq!(
            classify_probe_outcome(6),
            Outcome::Unavailable,
            "a DNS failure"
        );
    }

    // ---- backup receipts

    #[test]
    fn receipt_timestamps() {
        assert_eq!(
            parse_receipt_timestamp("20260906T210709Z"),
            Some(1_788_728_829)
        );
        for bad in [
            "2026-09-06T21:07:09Z",
            "20260906T210709",
            "20261306T210709Z",
            "",
            "never",
        ] {
            assert_eq!(parse_receipt_timestamp(bad), None, "{bad}");
        }
    }

    #[test]
    fn backup_thresholds() {
        let day = 86_400.0;
        assert_eq!(backup_age_state(None, 1.0, 2.0), AgeState::Never);
        assert_eq!(backup_age_state(Some(2 * 3600), 1.0, 2.0), AgeState::Ok);
        assert_eq!(
            backup_age_state(Some((1.8 * day) as i64), 1.0, 2.0),
            AgeState::Due
        );
        assert_eq!(
            backup_age_state(Some((2.1 * day) as i64), 1.0, 2.0),
            AgeState::Overdue
        );
        assert_eq!(
            backup_age_state(Some((400.0 * day) as i64), f64::NAN, f64::NAN),
            AgeState::Unknown
        );
        assert_eq!(backup_age_state(Some(60), 0.0, 0.0), AgeState::Overdue);
    }

    #[test]
    fn archives_at_the_target() {
        assert_eq!(
            classify_archive_at_target(false, None, "", 2361.0).0,
            Level::Bad
        );
        assert_eq!(
            classify_archive_at_target(true, Some(12), "", 2361.0).0,
            Level::Bad
        );
        assert_eq!(
            classify_archive_at_target(true, Some(2361), "-", 2361.0).0,
            Level::Ok
        );
        let (l, d) =
            classify_archive_at_target(true, Some(39_973_563), "compressed,dataless", 39_973_563.0);
        assert_eq!(l, Level::Warn);
        assert!(d.contains("offloaded") && d.contains("cannot prove the cloud copy"));
        let (l, d) = classify_archive_at_target(true, Some(0), "dataless", 39_973_563.0);
        assert_eq!(l, Level::Warn);
        assert!(d.contains("size differs"));
        assert_eq!(
            classify_archive_at_target(true, Some(10), "compressed", 10.0).0,
            Level::Ok
        );
    }

    #[test]
    fn failed_attempts() {
        let f = attempt_finding(
            1.0,
            1_000_000.0,
            "icloud-item: upload failed: Couldn't access your iCloud account",
            1_000_000.0 + 3.0 * 3600.0,
        );
        assert!(
            f.contains("FAILED 3.0h ago") && f.contains("exit 1") && f.contains("iCloud account")
        );
        let terse = attempt_finding(23.0, 1_000_000.0, "", 1_000_000.0);
        assert!(terse.contains("exit 23") && terse.ends_with(')'));
    }

    // ---- scheduled producers

    #[test]
    fn launchctl_table_and_plist() {
        let jobs = parse_launchd_jobs(
            &[
                "PID\tStatus\tLabel",
                "-\t0\tcom.axon.sparpreis-watch",
                "787\t0\tcom.axon.sjel-status",
                "-\t1\tcom.axon.backup",
            ]
            .join("\n"),
        );
        assert_eq!(jobs.len(), 3);
        assert_eq!(
            (
                jobs["com.axon.backup"].pid,
                jobs["com.axon.backup"].last_exit
            ),
            (None, Some(1))
        );
        assert_eq!(
            (
                jobs["com.axon.sjel-status"].pid,
                jobs["com.axon.sjel-status"].last_exit
            ),
            (Some(787), Some(0))
        );
        assert!(!jobs.contains_key("Label") && !jobs.contains_key("com.axon.host-patch"));
        let plist = "<plist version=\"1.0\">\n<dict>\n  <key>StartInterval</key>\n  <integer>21600</integer>\n  <key>StandardOutPath</key>\n  <string>/tmp/axon-feed-sweep-schedule.log</string>\n  <key>StandardErrorPath</key>\n  <string>/tmp/axon-feed-sweep-schedule.err</string>\n</dict>\n</plist>";
        let p = parse_launchd_schedule(plist);
        assert_eq!(p.interval, Some(21600.0));
        assert_eq!(
            p.stdout.as_deref(),
            Some("/tmp/axon-feed-sweep-schedule.log")
        );
        assert_eq!(
            p.stderr.as_deref(),
            Some("/tmp/axon-feed-sweep-schedule.err")
        );
        assert_eq!(
            parse_launchd_schedule("<dict><key>KeepAlive</key><true/></dict>").interval,
            None
        );
        assert_eq!(
            launchd_unit_capability("com.sjel.comms.plist").as_deref(),
            Some("comms")
        );
        assert_eq!(
            launchd_unit_capability("com.axon.comms.plist").as_deref(),
            Some("comms")
        );
        assert_eq!(launchd_unit_capability("com.other.comms.plist"), None);
    }

    fn producer(f: impl FnOnce(&mut Producer)) -> (Level, String) {
        let mut p = Producer {
            name: "feed-sweep".into(),
            unit_installed: true,
            loaded: true,
            last_exit: Some(0),
            interval: Some(21600.0),
            last_output_age: Some(600.0),
        };
        f(&mut p);
        classify_scheduled_producer(&p)
    }

    #[test]
    fn scheduled_producer_verdicts() {
        assert_eq!(producer(|_| {}).0, Level::Ok);
        assert_eq!(
            producer(|p| p.last_output_age = Some(21_599.0)).0,
            Level::Ok
        );
        assert_eq!(
            producer(|p| p.last_output_age = Some(21_600.0)).0,
            Level::Warn
        );
        assert_eq!(
            producer(|p| p.last_output_age = Some(64_799.0)).0,
            Level::Warn
        );
        let (l, m) = producer(|p| p.last_output_age = Some(64_800.0));
        assert!(l == Level::Bad && m.contains("missed at least two runs"));
        let (l, m) = producer(|p| {
            p.last_exit = Some(1);
            p.last_output_age = Some(5.0);
        });
        assert!(l == Level::Bad && m.contains("exited 1"));
        let (l, m) = producer(|p| {
            p.loaded = false;
            p.last_output_age = Some(60.0);
        });
        assert!(l == Level::Warn && m.contains("launchd has not loaded it"));
        let (l, m) = producer(|p| p.last_output_age = None);
        assert!(l == Level::Warn && m.contains("no output this machine still holds"));
        let (l, m) = producer(|p| {
            p.name = "finance-prices".into();
            p.unit_installed = false;
            p.loaded = false;
        });
        assert!(l == Level::Ok && m.contains("no unit installed"));
        assert_eq!(format_age(9.0), "9s");
        assert_eq!(format_age(600.0), "10m");
        assert_eq!(format_age(21_600.0), "6.0h");
        assert_eq!(format_age(345_600.0), "4.0d");
    }

    #[test]
    fn iso_dates_parse_as_date_parse_does() {
        assert_eq!(
            parse_iso_ms("2026-09-06T21:07:09Z"),
            Some(1_788_728_829_000.0)
        );
        assert_eq!(
            parse_iso_ms("2026-09-06T23:07:09+02:00"),
            Some(1_788_728_829_000.0)
        );
        assert_eq!(parse_iso_ms("not a date"), None);
    }
}
