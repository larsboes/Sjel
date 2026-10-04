//! `tools/audit` — the one command, and the only two scanners Sjel still runs itself.
//!
//! Ported from the bash script on 2026-10-04. The launcher keeps its path and execs this binary
//! under the script's own name, so the three callers do not change: `sjel update apply` runs it
//! as its last step (`updates/report.rs`'s `run_audit`), `tools/doctor` reads the verdict its exit
//! code put into the host-patch receipt, and `toolchain.toml`'s `workflow:audit` scope names the
//! two scanners it needs.
//!
//! ## What it runs
//!
//! `gitleaks` over this checkout's git history AND the private overlay's. The overlay half is the
//! scan nothing else does: GitHub secret scanning with push protection covers the public repo,
//! and a private repository needs a paid plan for it.
//!
//! `osv-scanner` twice. Once over every lockfile here, reading `osv-scanner.toml`'s dated
//! exceptions. Then, in a second pass, over software installed OUTSIDE this checkout — every node
//! in the global npm tree, and every `cargo install`ed crate with the transitive tree its own
//! published `Cargo.lock` pins. That second pass reads `osv-scanner-installed.toml`, and the two
//! files hold opposite policies on purpose: a known vulnerability stays blocking in this
//! checkout, and installed software that no command here can move is accepted with a reason and
//! a date. That second pass exists because nothing else looks at that class:
//! osv-scanner's own `directory` plugin extracts nothing from installed software (measured
//! 2026-10-03 on 2.6.0: 0 Extract calls, even against a valid dpkg status file), and Dependabot
//! reads lockfiles in this repository only. The inventory is `tools/updates --json --offline
//! --inventory`, so no second reader of `cargo install --list` appears here.
//!
//! ## Exit codes
//!
//! 0 clean · 1 a finding · 2 a scanner is not installed. A missing binary is a setup error, never
//! a finding: it must not read as clean and must not read as a leak. A finding outranks a missing
//! scanner, so a run with both exits 1.
//!
//! ## The one deliberate difference from the script
//!
//! The SBOM handed to the second `osv-scanner` pass is built here instead of by `jq`, so this
//! tool no longer needs `jq` on PATH — and a missing `jq` can no longer read as an unscanned
//! surface. `jq` keeps its `toolchain.toml` row: `tools/setup-secret.sh`, `tools/graphify.sh`,
//! `tools/restore.sh` and `.github/workflows/security.yml` still pipe JSON through it. JSON object
//! keys come out sorted, where `jq` wrote them in the order the filter named them; the
//! `components` array keeps the inventory's own order, which is what osv-scanner reads.

use crate::paths::Paths;
use crate::time;
use crate::updates::{Runner as _, SystemRunner};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

/// `tools/audit -h`. The script printed `sed -n '2,20p' "$0"` — its own header comment, comment
/// markers and all, cut off mid-sentence in the middle of the second pass's rationale. This is
/// the whole header instead, which is the same deliberate improvement `tools/toolchain-check`'s
/// port recorded.
const HELP: &str = "\
tools/audit — the one command, and the only two scanners Sjel still runs itself.

  tools/audit        # no arguments, no flags

  gitleaks     secret scan over this repository's git history AND the private overlay's.
               GitHub secret scanning with push protection already covers the public repo;
               the overlay is a private repository, where that is a paid feature, so the
               overlay half is the one scan nothing else does.
  osv-scanner  dependency CVEs over every lockfile here, reading osv-scanner.toml's dated
               exceptions. Dependabot alerts watch the same lockfiles continuously; this is
               the answer before a push rather than after it.

               AND, in a second pass, the same CVEs against software installed OUTSIDE this
               checkout: every node in the global npm tree, and every `cargo install`ed crate
               with the transitive tree its own published Cargo.lock pins, read against
               osv-scanner-installed.toml — where an accepted finding carries its reach and the
               date the acceptance ends, because installed software often has no fix this
               machine can reach.

               AND, in a second pass, the same CVEs against software installed OUTSIDE this
               checkout: every node in the global npm tree, and every `cargo install`ed crate
               with the transitive tree its own published Cargo.lock pins. That pass exists
               because nothing else looks at them. osv-scanner's own `directory` plugin
               cannot (measured 2026-10-03 on 2.6.0: 0 Extract calls, 1 dir visited, even
               against a valid dpkg status file), and Dependabot reads lockfiles in this
               repository only — so the class `tools/updates` is solely responsible for
               moving was the class no scanner covered. The inventory comes from
               `tools/updates --json --offline --inventory`, which reads `cargo install
               --list` and `npm ls -g --json --all` for the report already; this adds no
               second reader of either.

NOT here: container images (.github/workflows/security.yml runs `grype registry:` against
every image a service.toml declares, weekly and on push, whether or not this Mac is awake --
report-only since Q77, with the Critical/High counts in that run's summary) and Sjel's own
source (.github/workflows/codeql.yml).

Exit 0 clean · 1 a finding · 2 a scanner is not installed. A missing binary is a setup
error, never a finding: it must not read as clean and must not read as a leak. A finding
outranks a missing scanner, so a run with both exits 1.";

/// Both scanners' verdicts, as the two flags the exit code is decided from.
#[derive(Default)]
struct Report {
    /// A scanner reported something.
    fail: bool,
    /// A scanner is not installed, so its surface was not scanned.
    setup: bool,
}

impl Report {
    /// 0 clean · 1 a finding · 2 a scanner is missing, and a finding outranks a missing scanner.
    fn verdict(&self) -> u8 {
        if self.fail {
            1
        } else if self.setup {
            2
        } else {
            0
        }
    }
}

/// One row of `tools/updates --inventory`, the only three fields either pass reads.
#[derive(Debug, Clone)]
struct Component {
    ecosystem: String,
    name: String,
    version: String,
}

pub fn run(argv: &[String]) -> ExitCode {
    // `case "${1:-}"`: only the first argument is looked at, and an empty one is no argument.
    match argv.first().map_or("", String::as_str) {
        "" => {}
        "-h" | "--help" => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        other => {
            eprintln!("audit: takes no arguments (got '{other}')");
            return ExitCode::from(2);
        }
    }

    let root = match Paths::from_env() {
        Ok(p) => p.root,
        Err(e) => {
            eprintln!("audit: {e}");
            return ExitCode::from(2);
        }
    };
    // Read the name `tools/lib/paths.sh` exports for the private overlay, which is the same
    // path as `SJEL_OVERLAY_ROOT` in the shipped script and the only one its audit half ever
    // read. `Paths` deliberately does not carry it: this is the one tool whose overlay is a
    // git history rather than a deployment.
    let overlay = std::env::var("SJEL_PERSONAL_ROOT").unwrap_or_default();

    let mut r = Report::default();
    let root_text = root.display().to_string();

    println!("Axon audit · {}", utc_seconds());
    gitleaks_section(&root_text, &overlay, &mut r);
    repo_osv_section(&root, &mut r);
    globals_osv_section(&root, &mut r);

    println!();
    if r.setup {
        println!("── a scanner is not installed; its surface was not scanned ──");
    }
    let code = r.verdict();
    if code == 1 {
        println!("── audit: finding(s) — see above ──");
    } else if code == 0 {
        println!("── audit: clean ──");
    }
    ExitCode::from(code)
}

/// `date -u +%Y-%m-%dT%H:%M:%SZ`, which is `time::now_iso` with the milliseconds dropped.
fn utc_seconds() -> String {
    let iso = time::now_iso();
    let secs = iso.split('.').next().unwrap_or(iso.as_str());
    format!("{secs}Z")
}

// ── gitleaks ──────────────────────────────────────────────────────────────────

fn gitleaks_section(root: &str, overlay: &str, r: &mut Report) {
    println!();
    println!("▸ gitleaks · secret scan (git history)");
    if !have("gitleaks") {
        println!("  ⚠ gitleaks not installed — install: brew install gitleaks");
        r.setup = true;
        return;
    }
    scan_git_history("Axon", root, r);
    scan_git_history("private overlay", overlay, r);
}

/// One repository's secret scan. The overlay's path is never printed: a label is enough to act
/// on, and this output reaches terminals, transcripts and CI logs that the overlay's coordinate
/// must not. Neither is gitleaks' own output — a finding names the secret it found, redacted or
/// not, and the remedy is to run it there by hand.
fn scan_git_history(label: &str, repo: &str, r: &mut Report) {
    if repo.is_empty() {
        println!("  · {label} — not configured");
        return;
    }
    if !Path::new(repo).exists() {
        println!("  ✗ {label} — not reachable");
        r.fail = true;
        return;
    }
    // Ask git rather than test for a `.git` directory: a linked worktree has a `.git` FILE.
    if !Command::new("git")
        .args(["-C", repo, "rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        println!("  ✗ {label} — not a Git repository");
        r.fail = true;
        return;
    }

    let code = status_code(
        Command::new("gitleaks")
            .args([
                "detect",
                "-s",
                repo,
                "--redact",
                "--no-banner",
                "--exit-code",
                "1",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status(),
    );
    match code {
        0 => println!("  ✓ {label} — clean"),
        1 => {
            println!("  ✗ {label} — leak(s) found (details withheld; run gitleaks there yourself for redacted triage)");
            r.fail = true;
        }
        _ => {
            println!("  ? {label} — gitleaks errored (exit {code}; details withheld)");
            r.fail = true;
        }
    }
}

// ── osv-scanner, this checkout ────────────────────────────────────────────────

/// The directories osv-scanner is told to skip. Generated, not code — `capabilities/*/target`
/// alone runs to thousands of files — and the same set `.gitignore` excludes. `to-integrate/`
/// joins them for the reason `CONTRIBUTING.md#scratch-is-not-documentation` gives, and
/// `bazel-*` stays listed after PRD Q44 retired Bazel, because the symlinks it left still point
/// at multi-gigabyte trees on existing checkouts.
const EXCLUDED: [&str; 6] = [
    "target",
    "bazel-bin",
    "bazel-out",
    "bazel-testlogs",
    "node_modules",
    "to-integrate",
];

fn repo_osv_section(root: &Path, r: &mut Report) {
    println!();
    println!("▸ osv-scanner · dependency CVEs");
    if !have("osv-scanner") {
        println!("  ⚠ osv-scanner not installed — install: brew install osv-scanner");
        r.setup = true;
        return;
    }

    let root_text = root.display().to_string();
    let config = root.join("osv-scanner.toml").display().to_string();
    let mut cmd = Command::new("osv-scanner");
    cmd.args(["scan", "source", "-r", "--config", &config]);
    for d in EXCLUDED {
        cmd.args(["--experimental-exclude", d]);
    }
    cmd.arg(&root_text);
    // Inherited, not captured: the table of what it found is the finding, and a scanner that
    // reported one must not have its report swallowed. `osv-scanner.toml` enforces its own
    // `ignoreUntil` dates and names a lapsed entry under "unused ignores", so this carries no
    // clock of its own.
    let code = status_code(cmd.status());
    match code {
        0 => println!("  ✓ clean"),
        1 => {
            println!("  ✗ vulnerabilities found above");
            r.fail = true;
        }
        _ => {
            println!("  ? osv-scanner errored (exit {code})");
            r.fail = true;
        }
    }
}

// ── osv-scanner, software installed outside this checkout ─────────────────────

/// The second surface, and the one nothing else covers.
///
/// Every branch that cannot scan sets `setup`, never `fail`: an inventory that could not be read
/// is not evidence that the software on this machine is clean.
fn globals_osv_section(root: &Path, r: &mut Report) {
    println!();
    println!("▸ osv-scanner · globally installed (cargo install, npm -g)");
    if !have("osv-scanner") {
        println!("  ⚠ osv-scanner not installed — this surface was not scanned");
        r.setup = true;
        return;
    }
    let updates = root.join("tools/updates");
    if !is_executable(&updates) {
        println!("  ⚠ tools/updates not reachable — this surface was not scanned");
        r.setup = true;
        return;
    }

    // `--offline` asks for the INSTALLED versions, which is this pass's whole question. What is
    // newer than them is `sjel update`'s question, and asking a registry here would make the
    // audit depend on three of them. `--inventory`, not the default payload: `rows` is the
    // actionable view and for npm it is the top level only — 13 packages against 1825 nodes in
    // this machine's tree, measured 2026-10-03, and a CVE does not stop at the top level.
    let inventory = Command::new(&updates)
        .args(["--json", "--offline", "--inventory"])
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();

    let dir = std::env::temp_dir().join(format!("sjel-globals.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    if std::fs::create_dir_all(&dir).is_err() {
        println!("  ⚠ no temporary directory for the SBOM — this surface was not scanned");
        r.setup = true;
        return;
    }
    // Named `globals.cdx.json`: osv-scanner identifies a lockfile by its name, and `-L` is told
    // which file is the SBOM by where it sits in the argument list.
    let bom_path = dir.join("globals.cdx.json");
    let locks_dir = dir.join("locks");

    let components = parse_inventory(&inventory);
    let wrote_bom =
        components.is_some() && write_sbom(&bom_path, components.as_deref().unwrap_or(&[]));
    let locks = match &components {
        Some(c) => copy_lockfiles(&locks_dir, c),
        None => 0,
    };

    match &components {
        None => {
            println!("  ⚠ tools/updates returned no inventory — this surface was not scanned");
            r.setup = true;
        }
        Some(c) if scanned_rows(c).is_empty() => {
            // A real state, not a failure: nothing was installed by either manager. Reported as
            // such rather than as "clean", because an empty SBOM proves nothing about a machine.
            println!("  · nothing installed by cargo or npm — nothing to scan");
        }
        Some(c) if !wrote_bom => {
            println!("  ⚠ the SBOM could not be written — this surface was not scanned");
            r.setup = true;
        }
        Some(c) => {
            println!(
                "  · {} installed package(s) and {locks} crate lockfile(s)",
                scanned_rows(c).len()
            );
            // `-L`, not the deprecated `--sbom`: 2.6.0 accepts a CycloneDX file under either name
            // and prefers `-L`. `-r` over the lockfile directory walks only the copies made
            // above, and each is named in the finding's SOURCE column, so a transitive hit says
            // which installed crate carries it.
            //
            // `--verbosity error` deliberately silences the "unused ignores" listing, which this
            // pass would otherwise print directly under a repository pass that had just used
            // those entries — reading as an invitation to delete ones that are load-bearing.
            //
            // The cost is live as of 2026-10-04 and is stated in `osv-scanner-installed.toml`'s
            // own header rather than hidden here: that file's entries DO cover installed
            // packages, so an entry that stops applying — a crate rebuilt at a newer version, a
            // tree npm re-resolved — is not announced. The header carries the by-hand command
            // that lists which entries still apply.
            //
            // That file exists because the two passes have opposite policies. The repository
            // scan keeps `osv-scanner.toml`, whose rule is that a known vulnerability stays
            // blocking; installed software frequently has no fix this machine can reach, so
            // accepting one there is a decision with a date and a reason. CI reads the
            // repository config, so that rule is not weakened by anything here.
            let code = status_code(
                Command::new("osv-scanner")
                    .args(["scan", "source", "-r", "-L"])
                    .arg(&bom_path)
                    .arg("--config")
                    .arg(root.join("osv-scanner-installed.toml"))
                    .args(["--verbosity", "error"])
                    .arg(&locks_dir)
                    .status(),
            );
            match code {
                0 => println!("  ✓ clean"),
                1 => {
                    println!("  ✗ vulnerabilities found above");
                    // The one command that can move an npm finding whose OWNER is current: npm
                    // re-resolves a tree when the package is reinstalled, and `sjel update`'s
                    // report names the owners whose trees are behind. Named here rather than left
                    // as "run tools/audit", which was this pass's advice for a class no command
                    // reached (2026-10-04).
                    println!("  · an npm finding here may be movable: `sjel update apply --only npm --re-resolve <package>`");
                    r.fail = true;
                }
                _ => {
                    println!("  ? osv-scanner errored (exit {code})");
                    r.fail = true;
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The `--inventory` payload, or `None` when there is no inventory array in it at all — which is
/// a setup error, where a present-but-empty array is a machine with nothing installed.
fn parse_inventory(text: &str) -> Option<Vec<Component>> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let rows = value.get("inventory")?.as_array()?;
    Some(
        rows.iter()
            .map(|row| Component {
                ecosystem: field(row, "ecosystem"),
                name: field(row, "name"),
                version: field(row, "version"),
            })
            .collect(),
    )
}

fn field(row: &serde_json::Value, key: &str) -> String {
    row.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned()
}

/// The rows the two ecosystems this pass scans contribute, in the inventory's own order.
fn scanned_rows(components: &[Component]) -> Vec<&Component> {
    components
        .iter()
        .filter(|c| c.ecosystem == "npm" || c.ecosystem == "crates.io")
        .filter(|c| !c.name.is_empty() && !c.version.is_empty())
        .collect()
}

/// A package URL, which is how osv-scanner resolves a BOM component. An npm scope's `@` is
/// percent-encoded, as the spec requires; osv-scanner also accepts the raw form, and this is the
/// conformant one.
fn purl(c: &Component) -> String {
    if c.ecosystem == "crates.io" {
        format!("pkg:cargo/{}@{}", c.name, c.version)
    } else {
        format!("pkg:npm/{}@{}", c.name.replace('@', "%40"), c.version)
    }
}

/// CycloneDX 1.5, the shape osv-scanner reads. Only the two scanned ecosystems reach it — a brew
/// formula in here would be labelled as something it is not.
fn sbom(components: &[Component]) -> serde_json::Value {
    let rows: Vec<serde_json::Value> = scanned_rows(components)
        .iter()
        .map(|c| {
            serde_json::json!({
                "type": "library",
                "name": c.name,
                "version": c.version,
                "purl": purl(c),
            })
        })
        .collect();
    serde_json::json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "components": rows,
    })
}

fn write_sbom(path: &Path, components: &[Component]) -> bool {
    std::fs::write(path, sbom(components).to_string()).is_ok()
}

/// An installed crate's transitive tree is the `Cargo.lock` published inside it, because `cargo
/// install --locked` — the command `tools/updates` plans — resolves exactly that file. Read
/// rather than re-resolved, so what is scanned is what is installed. Copied into a directory per
/// crate because osv-scanner identifies a lockfile by its NAME, and each copy is its own half of
/// the scan.
fn copy_lockfiles(locks_dir: &Path, components: &[Component]) -> usize {
    let Some(cargo_home) = cargo_home() else {
        return 0;
    };
    let src = cargo_home.join("registry").join("src");
    for c in components.iter().filter(|c| c.ecosystem == "crates.io") {
        let crate_dir = format!("{}-{}", c.name, c.version);
        let Ok(registries) = std::fs::read_dir(&src) else {
            continue;
        };
        for registry in registries.flatten() {
            let lock = registry.path().join(&crate_dir).join("Cargo.lock");
            if !lock.is_file() {
                continue;
            }
            let dest = locks_dir.join(&crate_dir);
            if std::fs::create_dir_all(&dest).is_ok() {
                let _ = std::fs::copy(&lock, dest.join("Cargo.lock"));
            }
        }
    }
    // Counted from the tree, not from the copies: a crate version present in two registry source
    // directories writes the same one destination, and the script counted files with `find`.
    match std::fs::read_dir(locks_dir) {
        Ok(entries) => entries
            .flatten()
            .filter(|e| e.path().join("Cargo.lock").is_file())
            .count(),
        Err(_) => 0,
    }
}

/// `${CARGO_HOME:-$HOME/.cargo}`.
fn cargo_home() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CARGO_HOME").filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(|home| PathBuf::from(home).join(".cargo"))
}

// ── small shared pieces ───────────────────────────────────────────────────────

/// `command -v`, from the one implementation of "the first executable of that name on PATH" the
/// crate has (`tools/updates`' runner) rather than a second copy of it.
fn have(bin: &str) -> bool {
    SystemRunner.have(bin).is_some()
}

/// `[ -x <path> ]` for a path rather than a name on PATH.
fn is_executable(path: &Path) -> bool {
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// A child's exit code, or 127 the way a shell reports a command it could not run.
fn status_code(status: std::io::Result<std::process::ExitStatus>) -> i32 {
    status.ok().and_then(|s| s.code()).unwrap_or(127)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(ecosystem: &str, name: &str, version: &str) -> Component {
        Component {
            ecosystem: ecosystem.to_owned(),
            name: name.to_owned(),
            version: version.to_owned(),
        }
    }

    fn purls(json: &serde_json::Value) -> Vec<String> {
        json["components"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["purl"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn a_utc_stamp_has_whole_seconds() {
        let stamp = utc_seconds();
        assert_eq!(stamp.len(), 20, "{stamp}");
        assert!(stamp.ends_with('Z'), "{stamp}");
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[10..11], "T");
        assert_eq!(&stamp[19..], "Z");
    }

    #[test]
    fn a_scoped_npm_name_is_percent_encoded_and_a_crate_is_pkg_cargo() {
        assert_eq!(
            purl(&component("npm", "@sinclair/typebox", "0.34.52")),
            "pkg:npm/%40sinclair/typebox@0.34.52"
        );
        assert_eq!(
            purl(&component("npm", "ajv", "6.12.6")),
            "pkg:npm/ajv@6.12.6"
        );
        assert_eq!(
            purl(&component("crates.io", "macmon", "0.8.2")),
            "pkg:cargo/macmon@0.8.2"
        );
    }

    #[test]
    fn only_npm_and_crates_rows_reach_the_sbom() {
        // The brew row is the one the script's own comment singles out: it must be left out
        // rather than mislabelled. The nameless and versionless rows are excluded too.
        let json = sbom(&[
            component("crates.io", "macmon", "0.8.2"),
            component("npm", "@sinclair/typebox", "0.34.52"),
            component("brew", "jq", "1.7.1"),
            component("npm", "nameless", ""),
            component("npm", "", "9.9.9"),
        ]);
        assert_eq!(
            purls(&json),
            vec![
                "pkg:cargo/macmon@0.8.2".to_owned(),
                "pkg:npm/%40sinclair/typebox@0.34.52".to_owned()
            ]
        );
        assert_eq!(json["bomFormat"], "CycloneDX");
        assert_eq!(json["specVersion"], "1.5");
        assert_eq!(json["version"], 1);
    }

    #[test]
    fn an_inventory_is_an_inventory_only_when_it_carries_the_array() {
        assert!(parse_inventory("").is_none());
        assert!(parse_inventory("not json").is_none());
        assert!(parse_inventory("{}").is_none());
        assert!(parse_inventory(r#"{"inventory":"nope"}"#).is_none());
        assert_eq!(parse_inventory(r#"{"inventory":[]}"#).unwrap().len(), 0);
        let one = parse_inventory(
            r#"{"inventory":[{"ecosystem":"npm","name":"ajv","version":"6.12.6"}]}"#,
        )
        .unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].name, "ajv");
        // A row missing a field is a row with an empty one, never a dropped row.
        let sparse = parse_inventory(r#"{"inventory":[{"name":"ajv"}]}"#).unwrap();
        assert_eq!(sparse[0].version, "");
        assert_eq!(sparse[0].ecosystem, "");
        assert!(scanned_rows(&sparse).is_empty());
    }

    #[test]
    fn a_finding_outranks_a_missing_scanner() {
        assert_eq!(Report::default().verdict(), 0);
        assert_eq!(
            Report {
                fail: false,
                setup: true
            }
            .verdict(),
            2
        );
        assert_eq!(
            Report {
                fail: true,
                setup: false
            }
            .verdict(),
            1
        );
        assert_eq!(
            Report {
                fail: true,
                setup: true
            }
            .verdict(),
            1
        );
    }
}
