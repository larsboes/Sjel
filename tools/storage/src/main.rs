//! sjel-storage — what is filling this disk, and what is safe to reclaim.
//!
//! `sysmon report` answers "am I full" with one `df` line. This answers "what is filling
//! me", which `df` structurally cannot: the 46 GB that started this was 215 individually
//! unremarkable 230 MB cache blocks, invisible to every per-file view.
//!
//! Five verbs, two questions. `report`, `discover` and `apply` are about the machine and
//! read the overlay's `config/storage-policy.toml`, so core Axon stays generic and "normal
//! here" stays machine-specific (CONTRIBUTING.md#public-core-and-private-overlays).
//! `target` and `prune` are about this checkout. `prune` asks the policy which paths are the
//! policy's to decide and defers to it there, but it works on a clone with no overlay, which
//! is when someone most needs the disk back.
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

mod checkout;
mod classes;
mod discover;
mod guard;
mod inuse;
mod measure;
mod policy;
mod reclaim;
mod service;
mod target;

#[cfg(test)]
mod testutil;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use classes::Measured;
use measure::{fmt_bytes, measured_state, size_of, MeasuredState};
use policy::{disk_state, expand_glob, is_applicable, ExpectedService, ProtectedEntry, GB};

const HELP: &str = "sjel-storage — what is filling this disk, and what is safe to reclaim.

  sjel-storage report [--json]     free/used/total, every policy class, what is flagged
  sjel-storage reclaim [--dry-run] [--force] [--scheduled] [--only <class>]
                                   [--repo [--incremental] [--target] [--node-modules]]
                                   the one verb that removes anything. Classes by default;
                                   `--repo` adds the checkout's build output
  sjel-storage target [--json]     the Cargo target dir against PRD §9's R6 ratio
  sjel-storage discover [--top N] [--depth D] [--json]
                                   the largest directories under $HOME that no policy
                                   path names — where the `unattributed` figure lives
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
    // A flag's value: `--top 20`. The verb is still the first non-flag argument, so a value
    // never has to be told apart from one.
    let value = |flag: &str| -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let number = |flag: &str, default: usize| -> Result<usize, String> {
        match value(flag) {
            None => Ok(default),
            Some(v) => v
                .parse::<usize>()
                .map_err(|_| format!("{flag} needs a whole number, got '{v}'")),
        }
    };

    // The verb is the first argument that is not a flag. `--apply` with no verb is the
    // contract `tools/storage --apply` had before this crate existed, kept so an overlay
    // hook written against the TypeScript tool keeps working.
    let verb = match args.iter().find(|a| !a.starts_with('-')) {
        Some(v) => v.as_str(),
        None if has("--apply") => "apply",
        None => "report",
    };

    let result = match verb {
        "report" => cmd_report(json),
        "reclaim" | "apply" | "prune" if json => Err(
            // Carried over from `apply --json`, which printed a report and deleted nothing: a
            // caller that meant to act read that as success.
            "reclaim does not take --json — it would either delete without showing the plan or \
             print a report without acting. `reclaim --dry-run` shows the plan, \
             `report --json` reads the report."
                .to_string(),
        ),
        "reclaim" | "apply" | "prune" => cmd_reclaim(
            reclaim::Args {
                dry_run: has("--dry-run"),
                force: has("--force"),
                scheduled: has("--scheduled"),
                only: value("--only"),
                // `prune` and `apply` are spellings of this verb, not separate commands: both
                // were reclaim, and keeping two guard implementations is what let one of them
                // offer to clean a directory the policy had already said no to.
                repo: verb == "prune" || has("--repo"),
                incremental: has("--incremental"),
                target: has("--target"),
                node_modules: has("--node-modules"),
            },
            verb != "reclaim",
        ),
        "target" => cmd_target(json),
        "discover" => number("--top", 20)
            .and_then(|top| number("--depth", 2).and_then(|depth| cmd_discover(json, top, depth))),
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

/// A protected path, measured against the disk rather than trusted.
///
/// `paths` is empty when nothing on the machine answers to the entry. That is a different
/// finding from a directory measuring 0 bytes, and the report has to say which one it is:
/// the 2026-10-11 audit found four of six entries stale, each printing a confident reason
/// beside a `0 MB` that actually meant "this is not here any more". A protection guarding
/// nothing is not harmless — it teaches the next reader that the file is out of date, which
/// is exactly the state it is supposed to prevent.
struct Protected {
    entry: ProtectedEntry,
    paths: Vec<String>,
    bytes: u64,
}

/// One declared service and what the machine answered about it.
struct CheckedService {
    service: ExpectedService,
    presence: service::Presence,
}

fn cmd_report(json: bool) -> Result<u8, String> {
    let path = policy_path()?;
    let policy = policy::load(&path)?;
    let home = std::env::var("HOME").unwrap_or_default();

    let warn_gb = policy.thresholds.free_warn_gb.unwrap_or(0.0);
    let crit_gb = policy.thresholds.free_critical_gb.unwrap_or(0.0);
    let flag_gb = policy.thresholds.class_flag_gb.unwrap_or(f64::INFINITY);
    let flag_bytes = flag_gb * GB as f64;

    // The same walk `reclaim` acts on, so a class cannot measure one size here and act on
    // another there.
    let mut measured = classes::measure(&policy, &home);
    measured.sort_by_key(|m| std::cmp::Reverse(m.bytes));

    let protected: Vec<Protected> = policy
        .protected
        .iter()
        .map(|entry| {
            let paths = expand_glob(&entry.path, &home);
            Protected {
                bytes: size_of(&paths).bytes,
                entry: entry.clone(),
                paths,
            }
        })
        .collect();

    // The policy declares these; the machine answers. Asking here rather than in the report
    // is deliberate: the answer is part of the report's contract, so `--json` carries it too.
    let services: Vec<CheckedService> = policy
        .expected_service
        .iter()
        .map(|s| CheckedService {
            presence: service::check(&s.kind, &s.name),
            service: s.clone(),
        })
        .collect();

    let disk = measure::disk_usage();
    let state = disk_state(disk.free, warn_gb, crit_gb);
    let over_critical = (disk.free as f64) < crit_gb * GB as f64;

    // What the policy does not name.
    //
    // The failure this exists to catch is a report reading "1.3 GB reclaimable" on a disk that
    // is 96% full. On 2026-10-11 the gap between these two numbers was roughly 375 GB of the
    // 415 GB then in use, and ~/.cargo-target held 149 GB of it with no class naming that path
    // — the largest thing on the machine, invisible to the tool written to find exactly it.
    //
    // Read `unattributed` as a ceiling on the gap, not a measurement of one thing: `used` is
    // the whole Data volume, so it includes the OS, /Applications, other users and every log
    // here. `named` can also over-count, because a class path nested inside a protected path
    // is summed in both, which makes `unattributed` smaller rather than larger.
    //
    // Deliberately not an exit-code trigger. Exit 1 is a contract `tools/host-watch` reads,
    // and "this policy is incomplete" is a different alarm from "this disk is full".
    // `discover`'s rule, and the offset for the caveat this file documents: a class that
    // sits entirely inside a protected path is one thing on disk, and `named` used to count
    // it twice. Exact for that shape; a class straddling the boundary still over-counts,
    // which is why `named` stays a ceiling rather than becoming a measurement.
    let named = discover::named_bytes(
        &measured
            .iter()
            .map(|m| (m.paths.clone(), m.bytes))
            .collect::<Vec<_>>(),
        &protected
            .iter()
            .map(|p| (p.paths.clone(), p.bytes))
            .collect::<Vec<_>>(),
    );
    let unattributed = disk.used.saturating_sub(named);

    if json {
        // Field names and nesting are the contract tools/host-watch already reads, kept
        // byte-compatible with what tools/storage.ts emitted. Everything since is additive,
        // so an existing consumer is unaffected: `expected_service` (the text report always
        // printed it and a JSON reader had no way to see it), `named_bytes`,
        // `unattributed_bytes`, `measured` per class, and `presence` on each service.
        let doc = serde_json::json!({
            "disk": disk,
            "state": state,
            "classes": measured.iter().map(|m| serde_json::json!({
                "name": m.class.name,
                "bytes": m.bytes,
                "applicable": is_applicable(&m.class),
                "flagged": (m.bytes as f64) > flag_bytes,
                // Additive. `measured` is false when `du` returned a partial or empty
                // answer, so a reader can tell a real size from a lower bound. `present`
                // separates "the path is gone" from "it is here and measured 0", the same
                // pair `protected` gained in 2026-10-11.
                "measured": m.complete,
                "present": !m.paths.is_empty(),
                // The size axis, additive. `cap_bytes` is the gate `reclaim` obeys, and
                // `over_cap` its answer — distinct from `flagged`, which is the class_flag_gb
                // alarm and has never been actionable.
                "cap_bytes": m.class.cap_gb.map(|gb| (gb * GB as f64) as u64),
                "over_cap": m.class.cap_gb.is_some() && m.class.over_cap(m.bytes),
                "older_than_days": m.class.older_than_days,
            })).collect::<Vec<_>>(),
            "protected": protected.iter().map(|p| serde_json::json!({
                "path": p.entry.path, "bytes": p.bytes, "reason": p.entry.reason,
                // Additive, like `expected_service` below it: a path that is gone and a path
                // measuring 0 bytes were one indistinguishable number before.
                "present": !p.paths.is_empty(),
            })).collect::<Vec<_>>(),
            // Also additive. `unattributed_bytes` is the space no class and no protection
            // accounts for — the only figure here that can name the reader's blind spot
            // rather than a policy author's decision.
            "named_bytes": named,
            "unattributed_bytes": unattributed,
            // Additive like the fields above. `presence` is the machine's answer to a claim
            // the report used to print unchecked; `running` is the same answer as a boolean,
            // and null (never false) when the probe could not run.
            "expected_service": services.iter().map(|c| serde_json::json!({
                "name": c.service.name,
                "kind": c.service.kind,
                "note": c.service.note,
                "presence": c.presence.key(),
                "running": c.presence.running(),
                "reason": c.presence.reason(),
            })).collect::<Vec<_>>(),
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
    let mut nested_reclaimable = 0u64;
    for m in &measured {
        if m.bytes == 0 {
            continue;
        }
        // A class whose paths all sit inside another applicable class's would be counted twice,
        // and the sum is the one number a reader acts on. This is the 2026-10-11 lesson one
        // level up: `--incremental --target` reported 210.9 GB for roughly 150 GB because the
        // subtotal was added on top of the directory containing it. `rust-incremental` sits
        // inside `rust-debug-cache` by design — one gate is a size, the other an age — so the
        // inner one is subtracted rather than dropped, and `nested_reclaimable` says so.
        let nested_here = is_applicable(&m.class)
            && measured.iter().any(|outer| {
                outer.class.name != m.class.name
                    && is_applicable(&outer.class)
                    && !outer.paths.is_empty()
                    && m.paths.iter().all(|p| {
                        outer
                            .paths
                            .iter()
                            .any(|q| Path::new(p).starts_with(Path::new(q)))
                    })
            });
        if is_applicable(&m.class) && !nested_here {
            reclaimable += m.bytes;
        } else if nested_here {
            nested_reclaimable += m.bytes;
        }
        let mut marks: Vec<String> = [
            (measured_state(!m.paths.is_empty(), m.complete) == MeasuredState::Unmeasured)
                .then_some("partially measured".to_string()),
            (!is_applicable(&m.class)).then_some("report-only".to_string()),
            ((m.bytes as f64) > flag_bytes).then_some("OVER FLAG".to_string()),
            m.class.regrows.then_some("regrows".to_string()),
            m.class
                .older_than_days
                .map(|d| format!("entries older than {d}d")),
        ]
        .into_iter()
        .flatten()
        .collect();
        // The size axis is read against the class, not the machine. A cap is the one mark here
        // that changes what `reclaim` does, so it says which side of it the class is on rather
        // than only that a cap exists.
        if let Some(cap) = m.class.cap_gb {
            let cap = if cap.fract() == 0.0 {
                format!("{cap:.0}")
            } else {
                format!("{cap}")
            };
            marks.push(format!(
                "cap {cap} GB, {}",
                if m.class.over_cap(m.bytes) {
                    "OVER"
                } else {
                    "under"
                }
            ));
        }
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
        "\n  {} reclaimable without apply restrictions",
        fmt_bytes(reclaimable)
    );
    // Printed rather than dropped, so the per-class lines above still add up to the total below
    // them: a reader who adds them must not think the tool cannot count.
    if nested_reclaimable > 0 {
        println!(
            "    {} of that sits inside another class here and is counted once",
            fmt_bytes(nested_reclaimable)
        );
    }
    println!(
        "  {} named by policy · {} unattributed\n",
        fmt_bytes(named),
        fmt_bytes(unattributed)
    );

    // A class whose paths have all vanished is skipped by the list above, which is right for
    // an empty directory and wrong for a path that is gone: it makes a dead entry quieter
    // than a live one. `omlx-kv-cache` and `bazel-cache` both named directories that no
    // longer existed, and nothing said so.
    let absent: Vec<&Measured> = measured
        .iter()
        .filter(|m| measured_state(!m.paths.is_empty(), m.complete) == MeasuredState::Absent)
        .collect();
    if !absent.is_empty() {
        println!("Declared but absent — the policy names these, and nothing is there");
        for m in &absent {
            println!("  {:<26} {}", m.class.name, m.class.paths.join(", "));
        }
        println!();
    }

    // The case the old report dropped entirely: the path exists, so it was not "absent",
    // and its size was skipped by the list above, so it vanished from both halves. `du`
    // exits non-zero and still prints a total for what it read when it meets a subdirectory
    // it cannot enter, which made that total a lower bound presented as a size.
    let unmeasured: Vec<&Measured> = measured
        .iter()
        .filter(|m| measured_state(!m.paths.is_empty(), m.complete) == MeasuredState::Unmeasured)
        .collect();
    if !unmeasured.is_empty() {
        println!("Unmeasured — the path is here and du could not read all of it");
        for m in &unmeasured {
            println!("  {:<26} {}", m.class.name, m.class.paths.join(", "));
            if !m.error.is_empty() {
                println!("  {:<26} {}", "", m.error);
            }
        }
        println!();
    }

    if !protected.is_empty() {
        println!("Protected — reported, never applied");
        for p in &protected {
            // `absent` rather than `0 MB`: the reason on the next line was written when the
            // path existed, and nothing had ever re-asked it.
            let size = if p.paths.is_empty() {
                "absent".to_string()
            } else {
                fmt_bytes(p.bytes)
            };
            println!(
                "  {size:>9}  {}\n             {}",
                p.entry.path, p.entry.reason
            );
        }
        println!();
    }

    if !services.is_empty() {
        // The header no longer claims "not findings" without asking. A declared service that
        // does not answer prints as ABSENT, which is the finding the old section carried in
        // its own prose notes.
        println!("Expected running — declared in the policy, checked here");
        for c in &services {
            let note = c
                .service
                .note
                .as_deref()
                .map(|n| format!(" — {n}"))
                .unwrap_or_default();
            let reason = c
                .presence
                .reason()
                .map(|r| format!(" ({r})"))
                .unwrap_or_default();
            println!(
                "  {:<12} {} ({}){reason}{note}",
                c.presence.label(),
                c.service.name,
                c.service.kind
            );
        }
        println!();
    }

    println!("Read-only. Run `sjel storage reclaim` to act on what this names.");
    Ok(if over_critical { EXIT_OVER } else { EXIT_OK })
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
             `sjel storage reclaim --repo --target` reclaims it, if the policy allows.",
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

/// The one verb that removes anything, and the aliases that reach it.
///
/// `alias` says the caller used `apply` or `prune`, which is what decides whether the checkout
/// half and the policy half are in scope — see `reclaim::Args`. It is passed rather than
/// inferred because "the user typed the old name" is not visible from the arguments.
fn cmd_reclaim(args: reclaim::Args, alias: bool) -> Result<u8, String> {
    let root = repo_root()?;
    let home = std::env::var("HOME").unwrap_or_default();
    let policy = reclaim::load_policy();

    if alias {
        // The old verbs each named their half, so a caller that typed one of them meant that
        // half and nothing else. Loosening that quietly would change what the command they
        // have run for months does.
        println!(
            "Sjel storage reclaim · {} (as `{}`)",
            root.display(),
            if args.repo { "prune" } else { "apply" }
        );
    } else {
        println!(
            "Sjel storage reclaim · {}{}",
            root.display(),
            if args.dry_run { " (dry run)" } else { "" }
        );
    }

    let outcome = reclaim::run(policy.as_ref(), &home, &root, &args)?;
    if outcome.lines.is_empty() {
        println!("  nothing to reclaim");
    }
    for line in outcome.lines {
        println!("{line}");
    }
    // The survivors' total, not the plan's. A refused candidate is still on disk, so printing
    // the plan's bytes here would be the one number a reader acts on being wrong by exactly the
    // amount that was refused.
    println!(
        "\n  {} {}",
        fmt_bytes(outcome.bytes),
        if args.dry_run {
            "would be freed"
        } else {
            "freed"
        }
    );
    // Said out loud rather than dropped silently: the per-directory lines above still list the
    // incremental dirs, so without this the total looks like it fails to add up.
    if outcome.nested_bytes > 0 {
        println!(
            "    {} of that sits inside a directory the same plan removes, and is counted once",
            fmt_bytes(outcome.nested_bytes)
        );
    }
    if outcome.skipped > 0 {
        println!("  {} skipped by its own policy", outcome.skipped);
    }
    if outcome.refused > 0 {
        println!(
            "  {} refused — nothing was removed for those",
            outcome.refused
        );
    }
    Ok(if outcome.refused > 0 {
        EXIT_OVER
    } else {
        EXIT_OK
    })
}

/// Where the space the policy does not name actually is.
///
/// The walk is one `du`; the policy paths are measured on their own because each has to be
/// subtracted from whichever directory contains it. `-x` keeps the walk on one filesystem,
/// and `-d` bounds what is reported rather than what is read — `du` walks the whole tree at
/// any depth, so `--depth` changes the answer's shape, not its cost.
fn cmd_discover(json: bool, top: usize, depth: usize) -> Result<u8, String> {
    if depth == 0 {
        return Err("--depth must be at least 1".to_string());
    }
    let home = std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| "no HOME in the environment".to_string())?;
    let policy = policy::load(&policy_path()?)?;

    // Every policy path measured on its own. `report` sums a class's paths together; here
    // they must be separate or the subtraction charges the wrong directory.
    let mut named: Vec<discover::Named> = Vec::new();
    let policy_paths = policy
        .classes
        .iter()
        .flat_map(|c| c.paths.iter())
        .chain(policy.protected.iter().map(|p| &p.path));
    for pattern in policy_paths {
        for expanded in expand_glob(pattern, &home) {
            let bytes = size_of(std::slice::from_ref(&expanded)).bytes;
            named.push(discover::Named {
                path: expanded,
                bytes,
            });
        }
    }

    let out = Command::new("du")
        .arg("-x")
        .arg("-k")
        .arg("-d")
        .arg(depth.to_string())
        .arg(&home)
        .output()
        .map_err(|e| format!("could not run du: {e}"))?;
    let scan = discover::scan_state(out.status.success(), &String::from_utf8_lossy(&out.stderr));
    let walk = discover::parse_du_depth(&String::from_utf8_lossy(&out.stdout));

    let mut rows = discover::rows(&walk, &named, &home, depth);
    rows.truncate(top);

    let root_bytes = walk
        .iter()
        .find(|d| d.path == home)
        .map(|d| d.bytes)
        .unwrap_or(0);
    let named_bytes = discover::named_inside(&home, &named);
    let unnamed_bytes = root_bytes.saturating_sub(named_bytes);

    if json {
        let doc = serde_json::json!({
            "root": home,
            "depth": depth,
            "top": top,
            "partial": !scan.complete,
            "error": scan.error,
            "root_bytes": root_bytes,
            "named_bytes": named_bytes,
            "unnamed_bytes": unnamed_bytes,
            "rows": rows,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?
        );
        return Ok(EXIT_OK);
    }

    println!("Sjel storage discover · {home} (depth {depth})");
    println!(
        "  {} under $HOME is named by no policy path ({} total, {} named)",
        fmt_bytes(unnamed_bytes),
        fmt_bytes(root_bytes),
        fmt_bytes(named_bytes)
    );
    if !scan.complete {
        // The same rule as a class measurement: a partial number is said to be partial
        // rather than presented as a size. `du` meets this on cloud-backed directories.
        println!("  PARTIAL — du could not read everything, so every size below is a lower bound");
        if !scan.error.is_empty() {
            println!("    {}", scan.error);
        }
    }
    println!();
    if rows.is_empty() {
        println!("  nothing unnamed at depth {depth}");
        return Ok(EXIT_OK);
    }
    for row in &rows {
        let named_part = row.bytes.saturating_sub(row.unnamed_bytes);
        let short = discover::shorten(&row.path, &home);
        if named_part > 0 {
            println!(
                "  {:>9}  {short}  ({} of it named)",
                fmt_bytes(row.unnamed_bytes),
                fmt_bytes(named_part)
            );
        } else {
            println!("  {:>9}  {short}", fmt_bytes(row.unnamed_bytes));
        }
    }
    println!("\n  What is reclaimable is `sjel storage report`. This ranks what no policy names.");
    Ok(EXIT_OK)
}
