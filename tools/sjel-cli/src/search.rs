//! `sjel search <words...>` — commands, tools, capabilities and Packs, and a verdict.
//!
//! Moved from the bash launcher, tools/lib/tool-index.sh and tools/lib/capability-index.sh on
//! 2026-10-02. Matching is a case-insensitive substring test of the whole query, as `grep -iF`
//! was. A search that matches nothing exits 1 and says so on stderr: a search that cannot say
//! "no" is one whose silence a caller has to guess at.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::capability;
use crate::help::USAGE;
use crate::paths::Paths;

pub fn run(root: &Path, words: &[String]) -> ExitCode {
    if words.is_empty() {
        eprintln!("usage: sjel search <words...>");
        return ExitCode::from(1);
    }
    let query = words.join(" ");
    let needle = query.to_lowercase();
    let mut found = false;

    println!("Commands:");
    for line in USAGE.lines().filter(|l| l.to_lowercase().contains(&needle)) {
        println!("{line}");
        found = true;
    }
    println!("\nTools:");
    for row in search_tools(root, &needle) {
        println!("{row}");
        found = true;
    }
    println!("\nCapabilities:");
    // The registry hard-fails without a machine.toml. A checkout with none still searches the
    // other three sections. The overlay's own capabilities keep their README there; a machine
    // without an overlay still searches the public ones.
    let paths = Paths::from_shell(root).ok();
    let caps = paths
        .as_ref()
        .and_then(|p| capability::registry_with(p).ok())
        .unwrap_or_default();
    let overlay_caps = paths.as_ref().and_then(|p| p.overlay_caps_dir.clone());
    for line in search_capabilities(root, overlay_caps.as_deref(), &caps, &needle) {
        println!("{line}");
        found = true;
    }
    println!("\nPacks:");
    for line in packs(root)
        .lines()
        .filter(|l| l.to_lowercase().contains(&needle))
    {
        println!("{line}");
        found = true;
    }

    if found {
        ExitCode::SUCCESS
    } else {
        println!();
        eprintln!("sjel: nothing matches '{query}' in commands, tools, capabilities or Packs.");
        ExitCode::from(1)
    }
}

fn packs(root: &Path) -> String {
    // In-process, and through the same reader the adapters use: this was `bun run
    // tools/packs-opencode.ts list` until the adapters moved into this binary.
    match crate::packs::all_packs(root) {
        Ok(packs) => packs.into_iter().map(|pack| format!("{pack}\n")).collect(),
        Err(_) => String::new(),
    }
}

// ---- tools ----------------------------------------------------------------------------------

/// The first `n` lines of a file, or nothing.
fn head(path: &Path, n: usize) -> Vec<String> {
    fs::read(path)
        .map(|b| {
            String::from_utf8_lossy(&b)
                .lines()
                .take(n)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// A file name without its `.ts`, then its `.sh`, as the launcher-to-implementation pairing
/// reads it: `gadget` execs `gadget.ts`.
fn base_name(name: &str) -> &str {
    let n = name.strip_suffix(".ts").unwrap_or(name);
    n.strip_suffix(".sh").unwrap_or(n)
}

/// Not a tool to run for a task: a test, a fixture, a document or a manifest.
fn is_not_a_tool(name: &str) -> bool {
    [".test.sh", ".test.ts", ".example", ".md", ".json", ".toml"]
        .iter()
        .any(|s| name.ends_with(s))
        || name.contains(".example.")
}

/// `# tools/<name> — <summary>` (or `//`, or `--` or `-` for the dash) → the summary.
fn summary_after_tools_path<'a>(line: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    let rest = prefixes.iter().find_map(|p| line.strip_prefix(p))?;
    let rest = rest.strip_prefix("tools/")?;
    let name_len = rest
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        .count();
    if name_len == 0 {
        return None;
    }
    let rest = rest[name_len..].strip_prefix(' ')?;
    let rest = ["—", "--", "-"].iter().find_map(|d| rest.strip_prefix(d))?;
    Some(rest.trim_start_matches(' '))
}

/// The implementation a launcher execs, when one sits beside it under the same base name.
fn implementation(root: &Path, file: &Path, name: &str) -> Option<PathBuf> {
    let ts = root.join("tools").join(format!("{}.ts", base_name(name)));
    (ts.is_file() && ts != file).then_some(ts)
}

/// A tool's one-line description: its own `tools/<name> — …` header, else its implementation's,
/// else the first comment line of lines 2 to 6.
fn tool_summary(root: &Path, file: &Path, name: &str) -> String {
    let own = head(file, 25);
    if let Some(s) = own
        .iter()
        .find_map(|l| summary_after_tools_path(l, &["# ", "// "]))
    {
        return s.to_owned();
    }
    if let Some(ts) = implementation(root, file, name) {
        if let Some(s) = head(&ts, 25)
            .iter()
            .find_map(|l| summary_after_tools_path(l, &["// "]))
        {
            return s.to_owned();
        }
    }
    own.iter()
        .skip(1)
        .take(5)
        .find_map(|l| l.strip_prefix("# ").or_else(|| l.strip_prefix("// ")))
        .unwrap_or("")
        .to_owned()
}

/// One row per tool whose name or first 25 header lines contain `needle` (already lowercased).
/// A launcher and the implementation it execs are one tool, listed under the launcher.
pub fn search_tools(root: &Path, needle: &str) -> Vec<String> {
    let Ok(dir) = fs::read_dir(root.join("tools")) else {
        return Vec::new();
    };
    let mut files: Vec<(String, PathBuf)> = dir
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    files.sort();

    let mut rows = Vec::new();
    for (name, file) in &files {
        if is_not_a_tool(name) {
            continue;
        }
        if let Some(base) = name.strip_suffix(".ts") {
            if root.join("tools").join(base).is_file() {
                continue;
            }
        }
        let mut haystack = name.clone();
        for l in head(file, 25) {
            haystack.push('\n');
            haystack.push_str(&l);
        }
        if let Some(ts) = implementation(root, file, name) {
            for l in head(&ts, 25) {
                haystack.push('\n');
                haystack.push_str(&l);
            }
        }
        if haystack.to_lowercase().contains(needle) {
            rows.push(format!(
                "  tools/{name:<24} {}",
                tool_summary(root, file, name)
            ));
        }
    }
    rows
}

// ---- capabilities ---------------------------------------------------------------------------

/// The README's opening paragraph: the lines after the title, up to the first blank line or
/// the next heading, joined by one space.
fn readme_summary(dir: &Path) -> String {
    let Ok(body) = fs::read_to_string(dir.join("README.md")) else {
        return String::new();
    };
    let mut lines = body.lines().skip_while(|l| !l.starts_with('#')).skip(1);
    let mut parts: Vec<&str> = Vec::new();
    for l in lines.by_ref() {
        if l.starts_with('#') {
            break;
        }
        if l.trim().is_empty() {
            if parts.is_empty() {
                continue;
            }
            break;
        }
        parts.push(l);
    }
    parts.join(" ")
}

/// `r("METHOD", "/path", "description")` calls in a capability's Rust and TypeScript sources,
/// as `METHOD /path  description`. That call form is what route manifests use
/// (libs/route-manifest), and capabilities/knowledge-graph/server.ts writes its routes that way
/// so this index can read them.
fn routes(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    collect_sources(dir, &mut files);
    files.sort();
    files
        .iter()
        .filter_map(|f| fs::read_to_string(f).ok())
        .flat_map(|src| route_calls(&src))
        .collect()
}

fn collect_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.filter_map(Result::ok) {
        let p = e.path();
        let name = e.file_name();
        if p.is_dir() {
            if name != "node_modules" && name != "target" {
                collect_sources(&p, out);
            }
        } else if p.extension().is_some_and(|x| x == "rs" || x == "ts") {
            out.push(p);
        }
    }
}

/// Every match of `\br\(\s*"(METHOD)",\s*"([^"]*)",\s*"((?:[^"\\]|\\.)*)"`, scanned by hand
/// rather than through a regex crate for one pattern.
fn route_calls(src: &str) -> Vec<String> {
    const METHODS: [&str; 6] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = src[i..].find("r(") {
        let at = i + off;
        i = at + 1;
        let word_before = at > 0 && (b[at - 1].is_ascii_alphanumeric() || b[at - 1] == b'_');
        if word_before {
            continue;
        }
        let mut p = Parser {
            s: src,
            pos: at + 2,
        };
        let parsed = (|| {
            p.ws();
            let method = p.quoted_plain()?;
            if !METHODS.contains(&method) {
                return None;
            }
            p.lit(",")?;
            p.ws();
            let path = p.quoted_plain()?;
            p.lit(",")?;
            p.ws();
            let desc = p.quoted_escaped()?;
            Some((format!("{method} {path}  {desc}"), p.pos))
        })();
        if let Some((line, end)) = parsed {
            out.push(line);
            i = end;
        }
    }
    out
}

struct Parser<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        let rest = &self.s[self.pos..];
        self.pos += rest.len() - rest.trim_start().len();
    }
    fn lit(&mut self, l: &str) -> Option<()> {
        self.s[self.pos..]
            .starts_with(l)
            .then(|| self.pos += l.len())
    }
    /// `"…"` with no quote inside.
    fn quoted_plain(&mut self) -> Option<&'a str> {
        self.lit("\"")?;
        let len = self.s[self.pos..].find('"')?;
        let v = &self.s[self.pos..self.pos + len];
        self.pos += len + 1;
        Some(v)
    }
    /// `"…"` where a backslash escapes the next character. Returned as written, escapes kept.
    fn quoted_escaped(&mut self) -> Option<&'a str> {
        self.lit("\"")?;
        let start = self.pos;
        let mut chars = self.s[start..].char_indices();
        while let Some((k, c)) = chars.next() {
            match c {
                '\\' => {
                    chars.next()?;
                }
                '"' => {
                    self.pos = start + k + 1;
                    return Some(&self.s[start..start + k]);
                }
                _ => {}
            }
        }
        None
    }
}

/// Its directory: the public root first, then the overlay's.
fn capability_dir(root: &Path, overlay_caps: Option<&Path>, name: &str) -> Option<PathBuf> {
    let core = root.join("capabilities").join(name);
    if core.is_dir() {
        return Some(core);
    }
    overlay_caps.map(|d| d.join(name)).filter(|d| d.is_dir())
}

/// A capability matches on its name, kind, README opening paragraph or a route. Up to three
/// matching routes are shown under it.
pub fn search_capabilities(
    root: &Path,
    overlay_caps: Option<&Path>,
    caps: &[capability::Row],
    needle: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    for cap in caps.iter().filter(|c| !c.name.is_empty()) {
        let (summary, routes) = match capability_dir(root, overlay_caps, &cap.name) {
            Some(dir) => (readme_summary(&dir), routes(&dir)),
            None => (String::new(), Vec::new()),
        };
        let head = format!("{} {}\n{summary}", cap.name, cap.kind).to_lowercase();
        let hits: Vec<&String> = routes
            .iter()
            .filter(|r| r.to_lowercase().contains(needle))
            .collect();
        if !head.contains(needle) && hits.is_empty() {
            continue;
        }
        let summary = if summary.chars().count() > 100 {
            format!("{}...", summary.chars().take(97).collect::<String>())
        } else {
            summary
        };
        out.push(format!("  {:<20} {:<8} {summary}", cap.name, cap.kind));
        out.extend(hits.iter().take(3).map(|r| format!("      {r}")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Moved from tools/tool-index.test.sh and tools/capability-index.test.sh: the same
    // fixtures, the same assertions, and the same two checks against the real checkout.

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("sjel-cli-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn write(&self, rel: &str, body: &str) {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn real_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
    }

    fn tools_fixture() -> Scratch {
        let s = Scratch::new("tools");
        s.write(
            "tools/widget.sh",
            "#!/bin/bash\n# tools/widget.sh — polishes a widget until it shines.\n",
        );
        // A launcher and the file it execs: one tool, named by the launcher.
        s.write(
            "tools/gadget",
            "#!/usr/bin/env bash\n# Thin launcher: gadget's logic lives in gadget.ts.\n",
        );
        s.write("tools/gadget.ts", "// tools/gadget.ts — counts every gadget on this machine and says which are unowned.\n");
        s.write(
            "tools/legacy.sh",
            "#!/bin/bash\n# Restore a thing, carefully, without writing into live state.\n",
        );
        s.write(
            "tools/widget.test.sh",
            "#!/bin/bash\n# tools/widget.test.sh — planted cases for the widget polisher.\n",
        );
        s.write(
            "tools/widget.env.example",
            "# tools/widget.env.example — a widget's settings, with every value blank.\n",
        );
        s.write("tools/README.md", "# tools/README.md — what lives here.\n");
        s
    }

    fn says(rows: &[String], text: &str) -> bool {
        rows.iter().any(|r| r.contains(text))
    }

    #[test]
    fn tool_index_reads_headers() {
        let s = tools_fixture();
        let find = |q: &str| search_tools(&s.0, &q.to_lowercase());

        let r = find("polish");
        assert!(
            says(&r, "tools/widget.sh"),
            "the matching tool is named: {r:?}"
        );
        assert!(
            says(&r, "polishes a widget until it shines."),
            "the summary is the tool's own line"
        );
        assert!(!find("WIDGET").is_empty(), "matching is case-insensitive");
        assert!(!find("widg").is_empty(), "a partial name matches");

        let r = find("unowned");
        assert!(
            says(&r, "tools/gadget "),
            "the launcher is what is named: {r:?}"
        );
        assert!(
            !says(&r, "tools/gadget.ts"),
            "launcher and implementation are one tool"
        );
        assert!(
            says(&r, "counts every gadget"),
            "the launcher's summary comes from its implementation"
        );

        let r = find("Restore a thing");
        assert!(
            says(&r, "Restore a thing, carefully"),
            "the fallback summary is the first comment line: {r:?}"
        );

        assert!(find("planted").is_empty(), "a *.test.sh is not a tool");
        assert!(
            find("with every value blank").is_empty(),
            "an .example fixture is not a tool"
        );
        assert!(
            find("what lives here").is_empty(),
            "a document is not a tool"
        );
        assert!(
            find("nothing-in-this-tree-says-this").is_empty(),
            "a miss prints no rows"
        );
    }

    #[test]
    fn tool_index_reads_the_real_directory() {
        let root = real_root();
        let rows = search_tools(&root, "");
        assert!(
            rows.len() >= 40,
            "the empty query indexes only {} tools",
            rows.len()
        );
        assert!(says(&search_tools(&root, "backup"), "tools/backup.sh"));
    }

    fn caps_fixture() -> Scratch {
        let s = Scratch::new("caps");
        s.write("capabilities/inbox/README.md", "# inbox\n\nReads the postbox and proposes what to keep.\n\n## Store\n\nThe store has a backup contract.\n");
        s.write(
            "capabilities/inbox/src/server/main.rs",
            "const ROUTES: &[route_manifest::Route] = &[\n    r(\"GET\", \"/health\", \"Liveness.\"),\n    r(\n        \"POST\",\n        \"/letters/{id}/shred\",\n        \"Shred one letter for good.\",\n    ),\n];\n",
        );
        s.write(
            "capabilities/clock/README.md",
            "# clock\n\nTells the time.\n",
        );
        s.write(
            "capabilities/clock/src/main.rs",
            "const ROUTES: &[Route] = &[r(\"GET\", \"/now\", \"The time, in \\\"UTC\\\".\")];\n",
        );
        s.write(
            "overlay/capabilities/secret-one/README.md",
            "# secret-one\n\nA private capability that keeps its README in the overlay.\n",
        );
        fs::create_dir_all(s.0.join("capabilities/bare")).unwrap();
        s
    }

    fn cap(name: &str) -> capability::Row {
        capability::Row {
            name: name.into(),
            kind: "process".into(),
            ..Default::default()
        }
    }

    #[test]
    fn capability_index_reads_readmes_and_routes() {
        let s = caps_fixture();
        let overlay = s.0.join("overlay/capabilities");
        let caps: Vec<_> = ["inbox", "clock", "secret-one", "bare"]
            .into_iter()
            .map(cap)
            .collect();
        let find = |q: &str| search_capabilities(&s.0, Some(&overlay), &caps, &q.to_lowercase());

        let r = find("postbox");
        assert!(
            says(&r, "inbox"),
            "a word from the opening paragraph matches: {r:?}"
        );
        assert!(
            says(&r, "Reads the postbox and proposes what to keep."),
            "the summary is the README's line"
        );
        let r = find("shred");
        assert!(
            says(&r, "POST /letters/{id}/shred  Shred one letter for good."),
            "a route split over lines: {r:?}"
        );
        assert!(
            says(&find("utc"), "clock"),
            "a description with escaped quotes matches"
        );
        assert!(says(&find("TIME"), "clock"), "matching ignores case");
        assert!(
            says(&find("private"), "secret-one"),
            "an overlay README is read"
        );
        assert!(
            says(&find("bare"), "bare"),
            "no README or source still matches by name"
        );
        assert!(
            !says(&find("backup"), "inbox"),
            "a word below the opening paragraph does not match"
        );
        assert!(
            find("nothing-says-this").is_empty(),
            "no match prints nothing"
        );
    }

    #[test]
    fn capability_index_finds_comms_for_mail() {
        // ISA ISC-33, against the real checkout.
        let caps = [cap("comms"), cap("calendar")];
        let r = search_capabilities(&real_root(), None, &caps, "mail");
        assert!(r.iter().any(|l| l.starts_with("  comms ")), "{r:?}");
    }

    #[test]
    fn summary_header_forms() {
        assert_eq!(
            summary_after_tools_path("# tools/x.sh — a", &["# "]),
            Some("a")
        );
        assert_eq!(
            summary_after_tools_path("// tools/x -- b", &["// "]),
            Some("b")
        );
        assert_eq!(
            summary_after_tools_path("# tools/x-y - c", &["# "]),
            Some("c")
        );
        assert_eq!(summary_after_tools_path("# tools/ — c", &["# "]), None);
        assert_eq!(summary_after_tools_path("# see tools/x — c", &["# "]), None);
    }
}
