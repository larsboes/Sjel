//! `toolchain-check` — is every host tool Sjel needs actually installed?
//!
//! Reads toolchain.toml (the manifest of host binaries Sjel's own scripts assume) and, for each
//! entry that applies to THIS machine, checks it is on PATH at a new-enough version. The single
//! parser of that manifest; surfaced by tools/install.sh (bootstrap) and tools/doctor (ongoing).
//! Ported from the bash script of the same name on 2026-10-02. Its text and JSON output, exit
//! codes and scope rules are unchanged, and tools/toolchain-scope.test.sh asserts them.
//!
//! SCOPE. An entry's `needed_by` says where it applies (toolchain.toml's header). Absent means
//! core. `workflow:<name>` applies only when that workflow is asked about. `capability-field:<f>`
//! applies only when an ENABLED capability's manifest declares `f`, and `capability:<id>` only
//! when that capability is enabled. Both read machine.toml's list, so the requirement set follows
//! the deployment instead of a hand-kept role list.
//!
//! An out-of-scope entry reports `n/a` naming the scope that would pull it in, and never counts
//! toward the exit code.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use serde::{Deserialize, Serialize};

use crate::paths::Paths;

const HELP: &str = "\
tools/toolchain-check — is every host tool Sjel needs actually installed?

  tools/toolchain-check                    # check this machine, human report
  tools/toolchain-check --workflow backup  # ...plus what that workflow needs, before running it
  tools/toolchain-check --os macos         # check as if on <os> (bootstrap, no machine.toml yet)
  tools/toolchain-check --runtime docker   # resolve the container-runtime entry to <r>
  tools/toolchain-check --json             # machine-readable; exits 0 (verdict is data)
  tools/toolchain-check -h                 # this help

OS resolves from --os, else the build target. Container runtime resolves from --runtime, else
machine.toml, else unknown (runtime entries skipped with a note).

Exit 0 = every in-scope required tool present and new enough. 1 = an in-scope required or runtime
tool missing or outdated. 2 = usage error. --json is the exception: a miss is data in the payload,
never a non-zero exit, because its one caller is doctor, which wants the feed regardless.";

/// One `[section]` of toolchain.toml. Fields the checker does not read (`upstream`, comments)
/// are ignored rather than rejected: the manifest documents more than this tool consumes.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Entry {
    required: Option<String>,
    os: Option<String>,
    bin: Option<String>,
    why: Option<String>,
    install_macos: Option<String>,
    install_linux: Option<String>,
    min_version: Option<String>,
    version_cmd: Option<String>,
    needed_by: Vec<String>,
}

#[derive(Debug, Default)]
struct Options {
    os: String,
    runtime: String,
    workflow: String,
    json: bool,
}

/// What this machine's enabled set satisfies, resolved once per run.
#[derive(Debug, Default)]
struct Deployment {
    capabilities: Vec<String>,
    /// `capability-field:` names some enabled capability declares, in discovery order. The
    /// order is what the report's header line prints, so it is kept rather than sorted.
    active_fields: Vec<String>,
}

#[derive(Serialize)]
struct JsonEntry<'a> {
    tool: &'a str,
    bin: &'a str,
    class: &'a str,
    present: bool,
    status: &'a str,
    version: &'a str,
    note: &'a str,
    install: &'a str,
}

#[derive(Default, Serialize)]
struct Totals {
    count: u32,
    ok: u32,
    missing_required: u32,
    missing_optional: u32,
    outdated: u32,
    skipped: u32,
}

pub fn run(args: &[String]) -> ExitCode {
    let mut opts = match parse_args(args) {
        Ok(Some(o)) => o,
        Ok(None) => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("toolchain-check: {e}");
            return ExitCode::from(2);
        }
    };
    match check(&mut opts) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("toolchain-check: {e}");
            ExitCode::from(2)
        }
    }
}

/// `Ok(None)` asks for help. A flag that takes a value consumes the next argument whatever it
/// is, exactly as the bash parser did.
fn parse_args(args: &[String]) -> Result<Option<Options>, String> {
    let mut opts = Options::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let slot = match a.as_str() {
            "--os" => &mut opts.os,
            "--runtime" => &mut opts.runtime,
            "--workflow" => &mut opts.workflow,
            "--json" => {
                opts.json = true;
                continue;
            }
            "-h" | "--help" => return Ok(None),
            _ => return Err(format!("unknown arg '{a}'")),
        };
        *slot = it
            .next()
            .ok_or_else(|| format!("{a} needs a value"))?
            .clone();
    }
    Ok(Some(opts))
}

fn check(opts: &mut Options) -> Result<ExitCode, String> {
    let paths = Paths::from_env()?;
    let manifest = paths.root.join("toolchain.toml");
    let entries = read_manifest(&manifest)?;

    if opts.os.is_empty() {
        opts.os = match std::env::consts::OS {
            "macos" | "linux" => std::env::consts::OS.to_owned(),
            _ => String::new(),
        };
    }
    let machine = match paths.machine_toml() {
        Some(p) => Some(read_toml(p)?),
        None => None,
    };
    if opts.runtime.is_empty() {
        if let Some(rt) = machine
            .as_ref()
            .and_then(|m| m.get("container_runtime"))
            .and_then(toml::Value::as_str)
        {
            rt.clone_into(&mut opts.runtime);
        }
    }
    let deployment = resolve_deployment(&paths, machine.as_ref(), &entries);

    let mut text = String::new();
    let mut json_entries = Vec::new();
    let mut totals = Totals::default();
    let mut outdated_required = 0u32;

    if !opts.json {
        let _ = writeln!(text, "toolchain-check · {}", manifest.display());
        let _ = writeln!(
            text,
            "  os={}  runtime={}  workflow={}",
            or(&opts.os, "?"),
            or(&opts.runtime, "?"),
            or(&opts.workflow, "none")
        );
        if !deployment.active_fields.is_empty() {
            let _ = writeln!(
                text,
                "  enabled capabilities declare: {}",
                deployment.active_fields.join(" ")
            );
        }
        text.push('\n');
    }

    for (name, entry) in &entries {
        let required = entry.required.as_deref().unwrap_or("");
        let class = or(required, "yes");
        let bin = entry
            .bin
            .as_deref()
            .filter(|b| !b.is_empty())
            .unwrap_or(name);
        let install = install_hint(entry, &opts.os);

        // An entry pinned to another OS does not apply here.
        if entry
            .os
            .as_deref()
            .is_some_and(|o| !o.is_empty() && o != opts.os)
        {
            totals.skipped += 1;
            continue;
        }
        // Reported rather than dropped: an operator seeing `n/a (workflow:audit)` learns the
        // tool exists and what would require it, which a silent omission never teaches.
        if let Some(scope) = entry_scope(&entry.needed_by, &opts.workflow, &deployment) {
            if opts.json {
                json_entries.push(serde_json::json!(JsonEntry {
                    tool: name,
                    bin,
                    class,
                    present: false,
                    status: "n/a",
                    version: "",
                    note: &format!("needed by {scope}"),
                    install,
                }));
            } else {
                let _ = writeln!(text, "▸ {name:<16} · n/a here — needed by {scope}");
            }
            totals.skipped += 1;
            continue;
        }
        // Only the machine's chosen runtime is checked; the others are alternatives, not
        // missing tools. An unknown runtime cannot be judged.
        if required == "runtime" {
            if opts.runtime.is_empty() {
                if !opts.json {
                    let _ = writeln!(
                        text,
                        "▸ {name:<16} · runtime unknown (no --runtime / machine.toml) — not checked"
                    );
                }
                totals.skipped += 1;
                continue;
            }
            if *name != opts.runtime {
                totals.skipped += 1;
                continue;
            }
        }

        totals.count += 1;
        let present = on_path(bin);
        let mut version = String::new();
        let mut note = String::new();
        let status = if !present {
            // "yes" and a matched "runtime" are hard requirements; "optional" is a warning.
            if required == "optional" {
                "absent"
            } else {
                "missing"
            }
        } else {
            match (entry.min_version.as_deref(), entry.version_cmd.as_deref()) {
                (Some(min), Some(cmd)) if !min.is_empty() && !cmd.is_empty() => {
                    version = detect_version(cmd).unwrap_or_default();
                    if is_older(&version, min) {
                        note = format!("have {version}, need >= {min}");
                        "outdated"
                    } else {
                        "ok"
                    }
                }
                _ => "ok",
            }
        };

        match status {
            "ok" => totals.ok += 1,
            "outdated" => {
                totals.outdated += 1;
                if required != "optional" {
                    outdated_required += 1;
                }
            }
            "missing" => totals.missing_required += 1,
            _ => totals.missing_optional += 1,
        }

        if opts.json {
            json_entries.push(serde_json::json!(JsonEntry {
                tool: name,
                bin,
                class,
                present,
                status,
                version: &version,
                note: &note,
                install,
            }));
            continue;
        }
        let _ = match status {
            "ok" if version.is_empty() => writeln!(text, "▸ {name:<16} ✓ {bin}"),
            "ok" => writeln!(text, "▸ {name:<16} ✓ {bin} ({version})"),
            "outdated" => writeln!(
                text,
                "▸ {name:<16} ⚠ {bin} — {note}\n     install: {install}"
            ),
            "missing" => {
                writeln!(
                    text,
                    "▸ {name:<16} ✗ {bin} MISSING (required)\n     install: {install}"
                )
            }
            _ => writeln!(
                text,
                "▸ {name:<16} · {bin} absent (optional) — {}\n     install: {install}",
                entry.why.as_deref().unwrap_or("")
            ),
        };
    }

    if opts.json {
        let doc = serde_json::json!({
            "os": opts.os,
            "runtime": opts.runtime,
            "workflow": opts.workflow,
            "totals": totals,
            "entries": json_entries,
        });
        println!("{doc}");
        return Ok(ExitCode::SUCCESS);
    }

    let _ = writeln!(
        text,
        "\n── {} checked · {} ok · {} missing · {} outdated · {} optional absent · {} n/a ──",
        totals.count,
        totals.ok,
        totals.missing_required,
        totals.outdated,
        totals.missing_optional,
        totals.skipped
    );
    if opts.workflow.is_empty() {
        text.push_str(
            "   (scoped to this machine; add --workflow backup|restore|audit|build before running one)\n",
        );
    }
    print!("{text}");
    Ok(if totals.missing_required + outdated_required > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// toolchain.toml's sections in FILE order. The report reads top to bottom the way the manifest
/// does, and `toml` is built without `preserve_order` workspace-wide (tools/claude-code-config/
/// Cargo.toml says why), so the order comes from each section's span instead.
fn read_manifest(path: &Path) -> Result<Vec<(String, Entry)>, String> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let map: BTreeMap<String, toml::Spanned<Entry>> =
        toml::from_str(&body).map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    let mut sections: Vec<_> = map.into_iter().collect();
    sections.sort_by_key(|(_, e)| e.span().start);
    Ok(sections
        .into_iter()
        .map(|(n, e)| (n, e.into_inner()))
        .collect())
}

fn read_toml(path: &Path) -> Result<toml::Table, String> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    body.parse::<toml::Table>()
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))
}

/// The enabled capabilities, and which `capability-field:` scopes they satisfy. Only fields some
/// entry asks about are looked up: reading every manifest key would cost capabilities × keys for
/// an answer nobody consumes.
fn resolve_deployment(
    paths: &Paths,
    machine: Option<&toml::Table>,
    entries: &[(String, Entry)],
) -> Deployment {
    let capabilities: Vec<String> = machine
        .and_then(|m| m.get("capabilities"))
        .and_then(toml::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();

    let wanted: Vec<&str> = entries
        .iter()
        .flat_map(|(_, e)| e.needed_by.iter())
        .filter_map(|s| s.strip_prefix("capability-field:"))
        .collect();

    let mut active_fields: Vec<String> = Vec::new();
    if !wanted.is_empty() {
        for cap in &capabilities {
            let Some(manifest) = paths.manifest_for(cap) else {
                continue;
            };
            // A manifest that does not parse declares nothing here; the service runner and
            // tools/check-service-tomls.sh are the gates that report it.
            let Ok(table) = read_toml(&manifest) else {
                continue;
            };
            for field in &wanted {
                // Scalar or array: backup_sqlite is a path, backup_paths a list, and "does this
                // capability declare it" is the same question for both.
                if table.contains_key(*field) && !active_fields.iter().any(|f| f == field) {
                    active_fields.push((*field).to_owned());
                }
            }
        }
    }
    Deployment {
        capabilities,
        active_fields,
    }
}

/// `None` when the entry is in scope for this run, otherwise the reason it is not. No
/// `needed_by` at all is core and always in scope.
fn entry_scope(needed_by: &[String], workflow: &str, d: &Deployment) -> Option<String> {
    let mut first: Option<String> = None;
    for s in needed_by {
        if first.is_none() {
            first = Some(s.clone());
        }
        let satisfied = if let Some(w) = s.strip_prefix("workflow:") {
            w == workflow
        } else if let Some(f) = s.strip_prefix("capability-field:") {
            d.active_fields.iter().any(|a| a == f)
        } else if let Some(c) = s.strip_prefix("capability:") {
            // A tool a capability execs directly. `capability-field` cannot express it: the
            // requirement is that the capability exists here at all. Added after macmon was
            // enabled without its binary and this check reported 0 missing throughout.
            d.capabilities.iter().any(|a| a == c)
        } else if s == "core" {
            true
        } else {
            // An unknown token must not silently widen or narrow the set. It stays unsatisfied
            // and is named, so a typo surfaces as a tool that stopped being checked.
            first = Some(format!("unknown scope '{s}'"));
            false
        };
        if satisfied {
            return None;
        }
    }
    first
}

/// The install hint for `os`, falling back to whichever the entry declares.
fn install_hint<'a>(e: &'a Entry, os: &str) -> &'a str {
    let pick = |h: &'a Option<String>| h.as_deref().filter(|s| !s.is_empty());
    let own = match os {
        "macos" => pick(&e.install_macos),
        "linux" => pick(&e.install_linux),
        _ => None,
    };
    own.or_else(|| pick(&e.install_linux))
        .or_else(|| pick(&e.install_macos))
        .unwrap_or("")
}

/// `command -v`: an executable regular file on PATH, or the path itself when it names one.
fn on_path(bin: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let executable = |p: &Path| {
        p.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if bin.contains('/') {
        return executable(Path::new(bin));
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| executable(&dir.join(bin))))
}

/// The first dotted-number token of a version command's stdout ("git version 2.39.0" →
/// "2.39.0"). The command is a shell line from toolchain.toml, run through bash as the script
/// ran it through `eval`. Stderr is discarded, as it was.
fn detect_version(cmd: &str) -> Option<String> {
    let out = Command::new("bash")
        .args(["-c", cmd])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    first_dotted_number(&String::from_utf8_lossy(&out.stdout)).map(str::to_owned)
}

/// The leftmost match of `[0-9]+(\.[0-9]+)+`, without a regex dependency for one pattern.
fn first_dotted_number(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    let digits_from = |mut i: usize| {
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        i
    };
    let mut start = 0;
    while start < b.len() {
        if !b[start].is_ascii_digit() {
            start += 1;
            continue;
        }
        let mut end = digits_from(start);
        let mut groups = 0;
        while end + 1 < b.len() && b[end] == b'.' && b[end + 1].is_ascii_digit() {
            end = digits_from(end + 1);
            groups += 1;
        }
        if groups > 0 {
            return Some(&s[start..end]);
        }
        start = end;
    }
    None
}

/// True when `have` is strictly older than the floor `min`. A version that is not a plain dotted
/// number cannot be ordered, and inventing an order would be worse than admitting there is none
/// (tools/lib/version.sh), so it is never reported as outdated.
fn is_older(have: &str, min: &str) -> bool {
    match (numeric(have), numeric(min)) {
        (Some(h), Some(m)) => m > h,
        _ => false,
    }
}

/// A leading `v` stripped, then each dotted component as a number. Component-wise comparison of
/// these vectors is the order `sort -V` gives dotted numbers, including 1.2 < 1.2.0.
fn numeric(v: &str) -> Option<Vec<u64>> {
    let v = v.strip_prefix('v').unwrap_or(v);
    if v.is_empty() {
        return None;
    }
    v.split('.')
        .map(|p| {
            p.parse::<u64>()
                .ok()
                .filter(|_| p.bytes().all(|c| c.is_ascii_digit()))
        })
        .collect()
}

fn or<'a>(s: &'a str, fallback: &'a str) -> &'a str {
    if s.is_empty() {
        fallback
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_number_is_the_leftmost_full_match() {
        assert_eq!(first_dotted_number("git version 2.39.0"), Some("2.39.0"));
        assert_eq!(first_dotted_number("bun 1.3"), Some("1.3"));
        assert_eq!(first_dotted_number("abc12x3.4.5."), Some("3.4.5"));
        assert_eq!(first_dotted_number("v10 then 1.2"), Some("1.2"));
        assert_eq!(first_dotted_number("no version 12"), None);
        assert_eq!(first_dotted_number(""), None);
    }

    #[test]
    fn only_orderable_versions_can_be_outdated() {
        assert!(is_older("1.2.9", "1.3.0"));
        assert!(is_older("1.2", "1.2.0"));
        assert!(!is_older("1.10.0", "1.9.0"));
        assert!(!is_older("1.3.0", "v1.3.0"));
        assert!(!is_older("", "1.0"));
        assert!(!is_older("2026-rc1", "1.0"));
    }

    fn deployment(caps: &[&str], fields: &[&str]) -> Deployment {
        Deployment {
            capabilities: caps.iter().map(|s| (*s).to_owned()).collect(),
            active_fields: fields.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn scope(tokens: &[&str], workflow: &str, d: &Deployment) -> Option<String> {
        let v: Vec<String> = tokens.iter().map(|s| (*s).to_owned()).collect();
        entry_scope(&v, workflow, d)
    }

    #[test]
    fn scope_tokens_resolve_against_the_deployment() {
        let d = deployment(&["macmon"], &["backup_sqlite"]);
        assert_eq!(scope(&[], "", &d), None);
        assert_eq!(scope(&["core"], "", &d), None);
        assert_eq!(scope(&["workflow:backup"], "backup", &d), None);
        assert_eq!(
            scope(&["workflow:backup"], "", &d).as_deref(),
            Some("workflow:backup")
        );
        assert_eq!(scope(&["capability:macmon"], "", &d), None);
        assert_eq!(scope(&["capability-field:backup_sqlite"], "", &d), None);
        assert_eq!(
            scope(&["capability-field:backup_paths", "workflow:audit"], "", &d).as_deref(),
            Some("capability-field:backup_paths")
        );
    }

    #[test]
    fn an_unknown_token_is_named_and_never_satisfies() {
        let d = deployment(&[], &[]);
        assert_eq!(
            scope(&["workflo:backup"], "backup", &d).as_deref(),
            Some("unknown scope 'workflo:backup'")
        );
        assert_eq!(
            scope(&["workflo:backup", "workflow:backup"], "backup", &d),
            None
        );
    }

    #[test]
    fn a_value_flag_consumes_the_next_argument() {
        let args: Vec<String> = ["--os", "linux", "--json", "--workflow", "audit"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let o = parse_args(&args).unwrap().unwrap();
        assert_eq!(
            (o.os.as_str(), o.workflow.as_str(), o.json),
            ("linux", "audit", true)
        );
        assert!(parse_args(&["--os".to_owned()]).is_err());
        assert!(parse_args(&["--bogus".to_owned()]).is_err());
        assert!(parse_args(&["-h".to_owned()]).unwrap().is_none());
    }

    #[test]
    fn manifest_sections_keep_file_order() {
        let dir = std::env::temp_dir().join(format!("sjel-cli-order-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("toolchain.toml");
        std::fs::write(
            &p,
            "[zz]\nrequired = \"yes\"\n\n[aa]\nrequired = \"optional\"\n\n[mm]\n",
        )
        .unwrap();
        let names: Vec<String> = read_manifest(&p)
            .unwrap()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(names, ["zz", "aa", "mm"]);
    }
}
