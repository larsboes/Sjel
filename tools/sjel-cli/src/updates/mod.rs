//! `tools/updates` — every piece of software installed outside this checkout: who owns moving
//! it, what has gone stale, and what nothing moves at all.
//!
//! Ported from `tools/updates.ts` on 2026-10-04. The launcher keeps its path and execs this
//! binary under the script's own name, so `sjel update`, `tools/audit` and the dashboard do not
//! change. The report half and the apply half moved together: `apply` re-reads the report after
//! its steps, so splitting them would have left two readers of the same rows.
//!
//! ## The rule it inherits: one binary, one owner
//!
//! `apply` does NOT reimplement a single brew or uv step: it execs `tools/host-patch.sh`, which
//! is the one owner of those. What it owns directly is exactly the set nothing else owns —
//! `cargo install`ed crates and `npm -g` packages — plus delegation to the manual verbs.
//!
//! ## Output contract
//!
//! `--json` emits `{ generatedAt, offline, lastApply, surfaces, rows }` (plus `inventory` when
//! asked). That payload is the stable surface: the CLI table is derived from it, not the other
//! way round. `row.status` is one of current | stale | unknown | n/a and `row.owner` is one of
//! scheduled | manual | unowned | self. Those two vocabularies are the whole contract.
//!
//! ## Exit codes
//!
//! report: 0 = nothing stale · 1 = something is stale · 2 = usage error.
//! apply:  0 = every step succeeded · 1 = nothing to do or aborted · 2 = a step failed.

mod parse;
mod report;

use crate::time;
use parse::ApplyReceipt;
use report::{PlanOpts, Status};
use std::io::{IsTerminal, Write};
use std::process::{Command, ExitCode};

pub struct RunResult {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Injected rather than called directly, so the tests can drive every gatherer and the apply plan
/// against planted output without a brew, a cargo registry or a network.
pub trait Runner {
    fn run(&self, argv: &[&str]) -> RunResult;
    fn have(&self, bin: &str) -> Option<String>;
}

pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, argv: &[&str]) -> RunResult {
        match Command::new(argv[0]).args(&argv[1..]).output() {
            Ok(o) => RunResult {
                code: o.status.code().unwrap_or(1),
                stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            },
            Err(e) => RunResult {
                code: 1,
                stdout: String::new(),
                stderr: e.to_string(),
            },
        }
    }

    /// `Bun.which`: the first executable of that name on PATH.
    fn have(&self, bin: &str) -> Option<String> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|d| d.join(bin))
            .find(|c| {
                use std::os::unix::fs::PermissionsExt;
                c.metadata()
                    .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
            })
            .map(|p| p.display().to_string())
    }
}

static SYSTEM_RUNNER: SystemRunner = SystemRunner;

pub struct Ctx<'a> {
    runner: &'a dyn Runner,
    pub root: String,
    pub overlay: String,
    pub offline: bool,
}

impl Ctx<'_> {
    /// The injected runner, so a gatherer writes `ctx.run([...])` and not `ctx.runner.run([...])`.
    pub fn run(&self, argv: &[&str]) -> RunResult {
        self.runner.run(argv)
    }
    pub fn have(&self, bin: &str) -> Option<String> {
        self.runner.have(bin)
    }
}

/// The context from the environment `tools/lib/paths.sh` exports through the launcher. The
/// `SJEL_UPDATES_*` names win, so a caller that wants a different overlay for this one tool can
/// set them without moving every other tool's resolution.
fn make_ctx(offline: bool) -> Result<Ctx<'static>, String> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let root = var("SJEL_UPDATES_ROOT").or_else(|| var("SJEL_ROOT"));
    let overlay = var("SJEL_UPDATES_OVERLAY").or_else(|| var("SJEL_OVERLAY_ROOT"));
    match (root, overlay) {
        (Some(root), Some(overlay)) => Ok(Ctx {
            runner: &SYSTEM_RUNNER,
            root,
            overlay,
            offline,
        }),
        _ => Err(
            "run this through the launcher (tools/updates) — SJEL_UPDATES_ROOT/SJEL_UPDATES_OVERLAY unset"
                .to_owned(),
        ),
    }
}

// ── cli ───────────────────────────────────────────────────────────────────────

const HELP: &str = "sjel update — every piece of software installed outside this checkout.

  sjel update                   report: who owns moving each class, and what is stale
  sjel update --json            the same, machine-readable (surfaces + rows)
  sjel update --offline         receipts and installed versions only; no registry is asked
  sjel update --json --inventory
                                add an 'inventory' array: every installed npm node and cargo
                                crate, not just the actionable rows. This is what tools/audit
                                scans, and it is why the flag exists — a CVE does not stop at
                                the top level, and 'rows' is the top level only
  sjel update apply [--only <class>...] [--yes]
                                move what this tool owns, and delegate the rest
  sjel update apply --prune     also REMOVE leftovers nothing requires: packages the registry
                                has deprecated and duplicates whose parents bundle their own
                                copy. The list is printed first; a package a parent pins is
                                never included. This is the only destructive mode
  sjel update apply --only cargo --re-resolve <crate,...>
                                reinstall the named crates WITHOUT --locked, for a crate whose
                                published lockfile pins a dependency the audit has flagged
  sjel update apply --only npm --re-resolve <package,...>
                                reinstall the named global npm packages, which is what makes npm
                                resolve their dependency ranges again. For an owner whose own
                                version is current and whose tree is behind: `sjel update`
                                reports those owners as current rows naming what is behind them,
                                and tools/audit's second pass reports the CVE-bearing ones
  sjel update -h

Classes for --only:
  brew uv rustup containers graphify interceptor checkout cargo npm vendor

Report exits 1 when something is stale, 0 when nothing is. Apply exits 1 when there was
nothing to do (or you declined), 2 when a step failed.";

#[derive(Debug, Default)]
pub struct Options {
    pub verb: String,
    pub json: bool,
    pub offline: bool,
    pub yes: bool,
    pub only: Vec<String>,
    pub inventory: bool,
    pub prune: bool,
    pub re_resolve: Vec<String>,
}

pub fn parse_args(argv: &[String]) -> Result<Options, String> {
    // A leading flag means the default verb, not a verb named '--offline': `sjel update
    // --offline` is a report, and `apply` is the only verb that reads as a verb.
    let rest: Vec<&String> = argv.iter().filter(|a| !a.is_empty()).collect();
    let has_verb = rest.first().is_some_and(|a| !a.starts_with('-'));
    let verb = if has_verb {
        rest[0].clone()
    } else {
        "report".to_owned()
    };
    let tail = if has_verb { &rest[1..] } else { &rest[..] };
    let mut out = Options {
        verb,
        ..Default::default()
    };
    let mut i = 0;
    while i < tail.len() {
        let a = tail[i].as_str();
        match a {
            "--json" => out.json = true,
            "--offline" => out.offline = true,
            "--inventory" => out.inventory = true,
            "--yes" | "-y" => out.yes = true,
            "--prune" => out.prune = true,
            "--re-resolve" => {
                i += 1;
                let next = tail.get(i).ok_or("--re-resolve needs a crate name")?;
                out.re_resolve
                    .extend(next.split(',').filter(|s| !s.is_empty()).map(str::to_owned));
            }
            "--only" => {
                i += 1;
                let next = tail.get(i).ok_or("--only needs a class id")?;
                out.only
                    .extend(next.split(',').filter(|s| !s.is_empty()).map(str::to_owned));
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
        i += 1;
    }
    Ok(out)
}

pub fn run(argv: &[String]) -> ExitCode {
    if argv.iter().any(|a| a == "-h" || a == "--help")
        || argv.first().map(String::as_str) == Some("help")
    {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }

    let opts = match parse_args(argv) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("sjel update: {e}");
            eprintln!("run 'sjel update -h'");
            return ExitCode::from(2);
        }
    };
    if opts.verb != "report" && opts.verb != "apply" {
        eprintln!("sjel update: unknown verb '{}' (report|apply)", opts.verb);
        return ExitCode::from(2);
    }
    for id in &opts.only {
        if report::surface(id).is_none() {
            eprintln!("sjel update: unknown class '{id}'");
            return ExitCode::from(2);
        }
    }
    // A flag that silently does nothing is a lie about what the caller asked for.
    if opts.inventory && !opts.json {
        eprintln!(
            "sjel update: --inventory only means something with --json, where it is the audit's input"
        );
        return ExitCode::from(2);
    }
    for (flag, on) in [
        ("--prune", opts.prune),
        ("--re-resolve", !opts.re_resolve.is_empty()),
    ] {
        if on && opts.verb != "apply" {
            eprintln!("sjel update: {flag} only means something with 'apply'");
            return ExitCode::from(2);
        }
    }

    let ctx = match make_ctx(opts.offline) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("sjel update: {e}");
            return ExitCode::from(2);
        }
    };

    let report = report::build_report(&ctx);

    if opts.verb == "report" {
        if opts.json {
            let inventory = opts.inventory.then(|| report::build_inventory(&ctx));
            println!(
                "{}",
                report::render_json(
                    &report.rows,
                    &report.generated_at,
                    opts.offline,
                    report.last_apply.as_ref(),
                    inventory.as_deref(),
                )
            );
        } else {
            println!(
                "{}",
                report::render_table(&report.rows, opts.offline, report.last_apply.as_ref())
            );
        }
        return if report.rows.iter().any(|r| r.status == Status::Stale) {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        };
    }

    apply(&opts, &ctx, &report)
}

fn apply(opts: &Options, ctx: &Ctx, report: &report::Report) -> ExitCode {
    let plan_opts = PlanOpts {
        prune: opts.prune,
        re_resolve: opts.re_resolve.clone(),
    };
    let steps = report::plan_apply(&report.rows, &opts.only, ctx, &plan_opts);
    if steps.is_empty() {
        println!("sjel update: nothing to do — everything this tool can move is current");
        return ExitCode::from(1);
    }
    println!("sjel update apply — {} step(s)", steps.len());
    // The removals are printed as their own block before anything runs. `--prune` deletes, and
    // this list plus the confirmation below is the whole of what stands between the flag and a
    // package somebody wanted.
    let removals: Vec<usize> = steps
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.argv.first().map(String::as_str) == Some("npm")
                && s.argv.get(1).map(String::as_str) == Some("uninstall")
        })
        .map(|(i, _)| i)
        .collect();
    if !removals.is_empty() {
        println!("  REMOVING {} package(s) nothing requires:", removals.len());
        for &i in &removals {
            let s = &steps[i];
            let label = s.label.strip_prefix("prune: ").unwrap_or(&s.label);
            println!(
                "    ✗ {label}{}",
                s.note
                    .as_ref()
                    .map(|n| format!(" — {n}"))
                    .unwrap_or_default()
            );
        }
    }
    for (i, s) in steps.iter().enumerate() {
        if removals.contains(&i) {
            continue;
        }
        println!(
            "  · {}{}{}",
            s.label,
            s.note
                .as_ref()
                .map(|n| format!(" — {n}"))
                .unwrap_or_default(),
            if s.slow {
                "  (slow — this one compiles or pulls)"
            } else {
                ""
            }
        );
    }
    // A crate or npm package named on the command line that nothing plans to move is said out
    // loud, so a typo does not read as "it re-resolved and it was fine". Both shapes count:
    // cargo's re-resolve keeps the verb's own label, npm's is the plan's only (re-resolve) step.
    for name in &opts.re_resolve {
        let planned = steps.iter().any(|s| {
            s.label == format!("cargo: {name}") || s.label == format!("npm: {name} (re-resolve)")
        });
        if !planned {
            println!("  · --re-resolve named {name}, which this plan does not move");
        }
    }
    // Deliberately unlike tools/update.sh, which pulls straight through when stdin is not a TTY.
    // This one installs software, so an unattended run has to say --yes out loud.
    if !opts.yes && !std::io::stdin().is_terminal() {
        eprintln!(
            "sjel update: refusing to install unattended — re-run with --yes, or --only <class> to scope it"
        );
        return ExitCode::from(1);
    }
    if !opts.yes {
        print!("proceed? [y/N] ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        let _ = std::io::stdin().read_line(&mut answer);
        let answer = answer.trim();
        if !(answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes")) {
            println!("aborted");
            return ExitCode::from(1);
        }
    }

    let scope = if opts.only.is_empty() {
        "all".to_owned()
    } else {
        opts.only.join(",")
    };
    let total = steps.len() as u64;
    report::write_apply_receipt(
        &ctx.overlay,
        &ApplyReceipt {
            at: Some(time::now_iso()),
            class: Some(scope.clone()),
            steps: Some(total),
            state: Some("running".to_owned()),
            ..Default::default()
        },
    );

    let mut failed = 0u64;
    for s in &steps {
        println!("\n▸ {}", s.label);
        let argv: Vec<&str> = s.argv.iter().map(String::as_str).collect();
        let res = SYSTEM_RUNNER.run(&argv);
        if !res.stdout.is_empty() {
            print!("{}", res.stdout);
        }
        if !res.stderr.is_empty() {
            eprint!("{}", res.stderr);
        }
        if res.code != 0 {
            failed += 1;
            eprintln!("  ✗ {} failed (exit {}) — continuing", s.label, res.code);
        }
    }

    // Re-read rather than assume: an upgrade that reported success and moved nothing is a thing
    // package managers do, and the second read is what makes this a report and not a claim.
    let after = report::build_report(ctx);
    let still_stale: Vec<&report::Row> = after
        .rows
        .iter()
        .filter(|r| r.status == Status::Stale)
        .collect();

    // Last, over the machine the steps above just changed. This is the seam that was missing:
    // `apply --only cargo` and `--only npm` install software that no scan had ever looked at.
    println!("\n▸ audit (tools/audit)");
    let audit = report::run_audit(ctx);

    report::write_apply_receipt(
        &ctx.overlay,
        &ApplyReceipt {
            at: Some(time::now_iso()),
            class: Some(scope),
            steps: Some(total),
            failed: Some(failed),
            still_stale: Some(still_stale.len() as u64),
            state: Some(if failed > 0 { "failed" } else { "done" }.to_owned()),
            audit: Some(audit.as_str().to_owned()),
        },
    );
    println!(
        "\n── applied {}/{total}; {} still stale ──",
        total - failed,
        still_stale.len()
    );
    println!("── audit: {}{} ──", audit.as_str(), audit.advice());
    for r in &still_stale {
        println!(
            "  still stale: {}{}{}",
            r.surface,
            if r.name.is_empty() {
                String::new()
            } else {
                format!(" {}", r.name)
            },
            r.installed
                .as_ref()
                .map(|i| format!(" {i}"))
                .unwrap_or_default()
        );
    }
    if failed > 0 {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_leading_flag_is_the_default_verb() {
        assert_eq!(parse_args(&args(&["--offline"])).unwrap().verb, "report");
        assert!(parse_args(&args(&["--offline"])).unwrap().offline);
        assert_eq!(parse_args(&[]).unwrap().verb, "report");
    }

    #[test]
    fn apply_with_flags_after_it_parses() {
        let o = parse_args(&args(&["apply", "--only", "cargo,npm", "--yes"])).unwrap();
        assert_eq!(o.verb, "apply");
        assert_eq!(o.only, vec!["cargo", "npm"]);
        assert!(o.yes);
    }

    #[test]
    fn only_consumes_exactly_one_argument() {
        assert_eq!(
            parse_args(&args(&["apply", "--only", "cargo"]))
                .unwrap()
                .only,
            vec!["cargo"]
        );
        assert!(parse_args(&args(&["apply", "--only"])).is_err());
    }

    #[test]
    fn an_unknown_flag_is_an_error_rather_than_a_silent_no_op() {
        assert!(parse_args(&args(&["report", "--jsoon"])).is_err());
    }

    #[test]
    fn the_flags_parse_and_are_refused_where_they_would_do_nothing() {
        assert!(parse_args(&args(&["apply", "--prune"])).unwrap().prune);
        assert_eq!(
            parse_args(&args(&["apply", "--prune"])).unwrap().re_resolve,
            Vec::<String>::new()
        );
        assert_eq!(
            parse_args(&args(&["apply", "--re-resolve", "a,b"]))
                .unwrap()
                .re_resolve,
            vec!["a", "b"]
        );
        // `run` reads the environment only after the flag checks, so these need no launcher.
        assert_eq!(run(&args(&["report", "--prune"])), ExitCode::from(2));
        assert_eq!(
            run(&args(&["report", "--re-resolve", "bottom"])),
            ExitCode::from(2)
        );
        assert_eq!(run(&args(&["--inventory"])), ExitCode::from(2));
    }
}
