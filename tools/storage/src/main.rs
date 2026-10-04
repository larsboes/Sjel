//! sjel-storage — what is filling this disk, and what is safe to reclaim.
//!
//! `sysmon report` answers "am I full" with one `df` line. This answers "what is filling
//! me", which `df` structurally cannot: the 46 GB that started this was 215 individually
//! unremarkable 230 MB cache blocks, invisible to every per-file view.
//!
//! Four verbs, two questions. `report` and `apply` are about the machine and read the
//! overlay's `config/storage-policy.toml`, so core Axon stays generic and "normal here"
//! stays machine-specific (CONTRIBUTING.md#public-core-and-private-overlays). `target` and
//! `prune` are about this checkout, read no policy, and work on a clone with no overlay.
//!
//! Rust, not TypeScript. This replaced `tools/storage.ts` on 2026-09-03 under the owner's
//! ruling that generalized tooling is built in Rust first
//! (Packs/harness/skills/sjel/references/on-dependencies-and-build.md, "add backend logic in
//! Rust"). The TypeScript version's argument for its own runtime was that the policy is
//! array-of-tables and `tools/lib/toml.sh` cannot parse it — true, and an argument against
//! bash, not against Rust. Every pure function it carried is here with its tests.
//!
//! Exit codes, which `tools/host-watch`, `tools/doctor` and any hook depend on:
//!
//! * `0` — measured, and nothing is over a threshold.
//! * `1` — measured, and something is: free space below `free_critical_gb`, or `target`'s
//!   debug/release ratio above R6's 3×, or a `prune` path that had to be refused.
//! * `2` — could not measure: a usage error, or no overlay policy to read.

mod measure;
mod policy;
mod prune;
mod target;

#[cfg(test)]
mod testutil;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use measure::{fmt_bytes, size_of};
use policy::{disk_state, expand_glob, is_applicable, reclaim_argv, StorageClass, GB};

const HELP: &str = "sjel-storage — what is filling this disk, and what is safe to reclaim.

  sjel-storage report [--json]     free/used/total, every policy class, what is flagged
  sjel-storage apply  [--json]     run each applicable class's reclaim command
  sjel-storage target [--json]     the Cargo target dir against PRD §9's R6 ratio
  sjel-storage prune [--incremental] [--target] [--node-modules] [--dry-run]
                                   repo-scoped reclaim, no policy involved
  sjel-storage -h                  this help

Policy: <overlay>/config/storage-policy.toml — see schemas/storage-policy.toml.example.

Exit: 0 = nothing over a threshold, 1 = something is, 2 = could not measure.
";

const EXIT_OK: u8 = 0;
const EXIT_OVER: u8 = 1;
const EXIT_CANNOT_MEASURE: u8 = 2;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let json = has("--json");

    // The verb is the first argument that is not a flag. `--apply` with no verb is the
    // contract `tools/storage --apply` had before this crate existed, kept so an overlay
    // hook written against the TypeScript tool keeps working.
    let verb = match args.iter().find(|a| !a.starts_with('-')) {
        Some(v) => v.as_str(),
        None if has("--apply") => "apply",
        None => "report",
    };

    let result = match verb {
        "report" => cmd_report(json, false),
        "apply" => cmd_report(json, true),
        "target" => cmd_target(json),
        "prune" => cmd_prune(
            has("--incremental"),
            has("--target"),
            has("--node-modules"),
            has("--dry-run"),
        ),
        other => Err(format!("unknown command '{other}'\n\n{HELP}")),
    };

    match result {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("storage: {message}");
            ExitCode::from(EXIT_CANNOT_MEASURE)
        }
    }
}

/// This checkout's root.
///
/// `SJEL_ROOT` first, because `tools/lib/paths.sh` exports it and the launcher sources that
/// — which is the only answer that stays right inside a git worktree, where the shared
/// `CARGO_TARGET_DIR` belongs to a different checkout. Walking up from the cwd is the
/// fallback for `cargo run` and for a direct invocation.
fn repo_root() -> Result<PathBuf, String> {
    if let Ok(root) = sjel_config::env_var("SJEL_ROOT") {
        if !root.trim().is_empty() {
            return Ok(PathBuf::from(root));
        }
    }
    let mut dir = std::env::current_dir().map_err(|e| format!("no working directory: {e}"))?;
    loop {
        let manifest = dir.join("Cargo.toml");
        if std::fs::read_to_string(&manifest)
            .map(|t| t.contains("[workspace]"))
            .unwrap_or(false)
        {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err(
                "no Sjel checkout here — set SJEL_ROOT or run this through tools/storage/storage"
                    .to_string(),
            );
        }
    }
}

/// The overlay's policy file.
///
/// Resolved through `sjel_config::overlay_root`, exactly as `capabilities/host-net` resolves
/// its own policy — so `SJEL_PERSONAL_ROOT` is the one input, and the axon.local.toml →
/// axon.toml order stays owned by `tools/lib/paths.sh` and `libs/overlay/overlay.ts`. A
/// third copy of that order in Rust is what this avoids; the launcher sources `paths.sh`.
fn policy_path() -> Result<PathBuf, String> {
    let root = sjel_config::overlay_root().ok_or_else(|| {
        "no overlay — SJEL_PERSONAL_ROOT is unset. Run this through tools/storage/storage \
         or `sjel storage`, which source tools/lib/paths.sh."
            .to_string()
    })?;
    Ok(root.join("config").join("storage-policy.toml"))
}

struct Measured {
    class: StorageClass,
    paths: Vec<String>,
    bytes: u64,
}

fn cmd_report(json: bool, do_apply: bool) -> Result<u8, String> {
    let path = policy_path()?;
    let policy = policy::load(&path)?;
    let home = std::env::var("HOME").unwrap_or_default();

    let warn_gb = policy.thresholds.free_warn_gb.unwrap_or(0.0);
    let crit_gb = policy.thresholds.free_critical_gb.unwrap_or(0.0);
    let flag_gb = policy.thresholds.class_flag_gb.unwrap_or(f64::INFINITY);
    let flag_bytes = flag_gb * GB as f64;

    let mut measured: Vec<Measured> = policy
        .classes
        .iter()
        .map(|class| {
            let paths: Vec<String> = class
                .paths
                .iter()
                .flat_map(|p| expand_glob(p, &home))
                .collect();
            Measured {
                bytes: size_of(&paths),
                class: class.clone(),
                paths,
            }
        })
        .collect();
    measured.sort_by_key(|m| std::cmp::Reverse(m.bytes));

    let protected: Vec<(String, u64, String)> = policy
        .protected
        .iter()
        .map(|p| {
            (
                p.path.clone(),
                size_of(&expand_glob(&p.path, &home)),
                p.reason.clone(),
            )
        })
        .collect();

    let disk = measure::disk_usage();
    let state = disk_state(disk.free, warn_gb, crit_gb);
    let over_critical = (disk.free as f64) < crit_gb * GB as f64;

    if json {
        // Field names and nesting are the contract tools/host-watch already reads, kept
        // byte-compatible with what tools/storage.ts emitted. `expected_service` is the one
        // addition: the text report always printed it, and a JSON reader had no way to see
        // it. Additive, so an existing consumer is unaffected.
        let doc = serde_json::json!({
            "disk": disk,
            "state": state,
            "classes": measured.iter().map(|m| serde_json::json!({
                "name": m.class.name,
                "bytes": m.bytes,
                "applicable": is_applicable(&m.class),
                "flagged": (m.bytes as f64) > flag_bytes,
            })).collect::<Vec<_>>(),
            "protected": protected.iter().map(|(path, bytes, reason)| serde_json::json!({
                "path": path, "bytes": bytes, "reason": reason,
            })).collect::<Vec<_>>(),
            "expected_service": policy.expected_service,
        });
        println!("{}", serde_json::to_string_pretty(&doc).unwrap_or_default());
        return Ok(if over_critical { EXIT_OVER } else { EXIT_OK });
    }

    let pct = if disk.total == 0 {
        0
    } else {
        (disk.used as f64 / disk.total as f64 * 100.0).round() as u64
    };
    println!("Sjel storage · {}", disk.target);
    println!(
        "  {} used / {} ({}%) · {} free · {}\n",
        fmt_bytes(disk.used),
        fmt_bytes(disk.total),
        pct,
        fmt_bytes(disk.free),
        state
    );

    println!("Reclaimable by class");
    let mut reclaimable = 0u64;
    for m in &measured {
        if m.bytes == 0 {
            continue;
        }
        if is_applicable(&m.class) {
            reclaimable += m.bytes;
        }
        let marks: Vec<&str> = [
            (!is_applicable(&m.class)).then_some("report-only"),
            ((m.bytes as f64) > flag_bytes).then_some("OVER FLAG"),
            m.class.regrows.then_some("regrows"),
        ]
        .into_iter()
        .flatten()
        .collect();
        let suffix = if marks.is_empty() {
            String::new()
        } else {
            format!("  [{}]", marks.join(", "))
        };
        println!("  {:>9}  {}{}", fmt_bytes(m.bytes), m.class.name, suffix);
        println!(
            "             {} path{} · {}",
            m.paths.len(),
            if m.paths.len() == 1 { "" } else { "s" },
            m.class.reclaim.as_deref().unwrap_or("no reclaim command")
        );
    }
    println!(
        "\n  {} reclaimable without apply restrictions\n",
        fmt_bytes(reclaimable)
    );

    if !protected.is_empty() {
        println!("Protected — reported, never applied");
        for (path, bytes, reason) in &protected {
            println!("  {:>9}  {path}\n             {reason}", fmt_bytes(*bytes));
        }
        println!();
    }

    if !policy.expected_service.is_empty() {
        println!("Expected running (not findings)");
        for s in &policy.expected_service {
            let note = s
                .note
                .as_deref()
                .map(|n| format!(" — {n}"))
                .unwrap_or_default();
            println!("  {} ({}){}", s.name, s.kind, note);
        }
        println!();
    }

    if !do_apply {
        println!("Read-only. Re-run as `sjel storage apply` to execute the applicable reclaims.");
        return Ok(if over_critical { EXIT_OVER } else { EXIT_OK });
    }

    println!("Applying");
    for m in &measured {
        let Some(argv) = reclaim_argv(&m.class, &m.paths) else {
            continue;
        };
        if m.bytes == 0 {
            continue;
        }
        let out = Command::new(&argv[0])
            .args(&argv[1..])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output();
        let (code, err) = match out {
            Ok(o) => (
                o.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&o.stderr).trim().to_string(),
            ),
            Err(e) => (-1, e.to_string()),
        };
        let still_there: Vec<String> = m
            .paths
            .iter()
            .filter(|p| Path::new(p).exists())
            .cloned()
            .collect();
        let after = size_of(&still_there);
        let failed = if code == 0 {
            String::new()
        } else {
            format!(
                " (exit {code}: {})",
                err.chars().take(120).collect::<String>()
            )
        };
        println!(
            "  {}: freed {}{failed}",
            m.class.name,
            fmt_bytes(m.bytes.saturating_sub(after))
        );
    }

    let post = measure::disk_usage();
    println!(
        "\n  {} free (was {}, +{})",
        fmt_bytes(post.free),
        fmt_bytes(disk.free),
        fmt_bytes(post.free.saturating_sub(disk.free))
    );
    Ok(if (post.free as f64) < crit_gb * GB as f64 {
        EXIT_OVER
    } else {
        EXIT_OK
    })
}

fn cmd_target(json: bool) -> Result<u8, String> {
    let root = repo_root()?;
    let report = target::measure(&root);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
        return Ok(if report.r6 == "over" {
            EXIT_OVER
        } else {
            EXIT_OK
        });
    }

    println!("Sjel storage · {} ({})", report.target_dir, report.source);
    println!("  {} total\n", fmt_bytes(report.bytes));

    for p in &report.profiles {
        if !p.present {
            println!("{:>9}  {} — never built", "—", p.name);
            continue;
        }
        println!(
            "  {:>9}  {} · {} unit{}",
            fmt_bytes(p.bytes),
            p.name,
            p.units,
            if p.units == 1 { "" } else { "s" }
        );
        for (label, bytes) in [
            ("deps", p.buckets.deps),
            ("incremental", p.buckets.incremental),
            ("build", p.buckets.build),
            ("fingerprint", p.buckets.fingerprint),
            ("binaries", p.buckets.binaries),
            ("other", p.buckets.other),
        ] {
            println!("             {:>9}  {label}", fmt_bytes(bytes));
        }
    }

    println!();
    match report.ratio {
        None => println!(
            "R6 (PRD §9, Q53): unknown — no release build to compare against. \
             Run `cargo build --release --workspace` first."
        ),
        Some(ratio) => {
            println!(
                "R6 (PRD §9, Q53): debug is {ratio:.1}× release, limit {:.0}× — {}",
                report.r6_max_ratio, report.r6
            );
            // Q53's control is "same crates, same machine, same moment". Say when the
            // crate sets differ, because then the ratio is comparing two different builds
            // and the reader needs to know before acting on it.
            let units = |name: &str| {
                report
                    .profiles
                    .iter()
                    .find(|p| p.name == name)
                    .map_or(0, |p| p.units)
            };
            let (d, r) = (units("debug"), units("release"));
            if d != r {
                println!(
                    "  Not a like-for-like control: debug holds {d} compilation units and \
                     release {r}. Build both profiles over the same crates before acting on \
                     the ratio."
                );
            }
        }
    }

    let t = &report.toolchain;
    if t.matches {
        match (&t.recorded, &t.current) {
            // Deliberately not phrased as "clean". Cargo rewrites .rustc_info.json on its
            // first run under a new compiler and leaves the previous generation's output in
            // deps/, so a match says cargo has run since the last roll and nothing more.
            (Some(hash), _) => println!(
                "Toolchain: cargo last recorded rustc {}, which is the one installed. \
                 A match does not mean the tree holds no output from an older rustc.",
                &hash[..hash.len().min(9)]
            ),
            _ => println!("Toolchain: no recorded rustc in .rustc_info.json — nothing to compare"),
        }
    } else {
        println!(
            "Toolchain: STALE — .rustc_info.json records {} and rustc is {}. \
             {} of deps and fingerprints was built by a compiler this machine no longer has; \
             `sjel storage prune --target` reclaims it.",
            t.recorded.as_deref().unwrap_or("?"),
            t.current.as_deref().unwrap_or("?"),
            fmt_bytes(t.stale_candidate_bytes),
        );
    }
    if !report.secondary.is_empty() {
        println!(
            "\n  {} in {} target dir{} cargo does not resolve to here:",
            fmt_bytes(report.secondary_bytes),
            report.secondary.len(),
            if report.secondary.len() == 1 { "" } else { "s" }
        );
        for s in &report.secondary {
            println!("  {:>9}  {}", fmt_bytes(s.bytes), s.path);
            println!("             cargo clean --manifest-path {}", s.manifest);
        }
    }

    Ok(if report.r6 == "over" {
        EXIT_OVER
    } else {
        EXIT_OK
    })
}

fn cmd_prune(
    incremental: bool,
    want_target: bool,
    node_modules: bool,
    dry_run: bool,
) -> Result<u8, String> {
    if !(incremental || want_target || node_modules) {
        return Err(format!(
            "prune needs at least one of --incremental, --target, --node-modules\n\n{HELP}"
        ));
    }
    let root = repo_root()?;
    let (dir, _) = target::target_dir(&root);
    let plan = prune::Plan::build(&root, &dir, incremental, want_target, node_modules);

    println!(
        "Sjel storage prune · {}{}",
        root.display(),
        if dry_run { " (dry run)" } else { "" }
    );
    let (lines, refused) = prune::run(&plan, &root, &dir, dry_run);
    if lines.is_empty() {
        println!("  nothing to prune");
    }
    for line in lines {
        println!("{line}");
    }
    println!(
        "\n  {} {}",
        fmt_bytes(plan.bytes()),
        if dry_run { "would be freed" } else { "freed" }
    );
    Ok(if refused > 0 { EXIT_OVER } else { EXIT_OK })
}
