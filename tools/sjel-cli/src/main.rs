//! sjel-cli — Sjel's public command interface and operator tooling, as one binary.
//!
//! `sjel` (the repository-root launcher that ~/.local/bin/sjel links to) execs this binary with
//! its arguments unchanged, so the top level here IS `sjel`. A ported `tools/` script execs it
//! with its own name as the first argument (`toolchain-check`). Those names are not listed in
//! `sjel help`: the script path stays the documented interface. Decided 2026-10-02: one crate
//! rather than one per tool, so repository, overlay and manifest resolution are written once,
//! and CONTRIBUTING.md#cargo-and-bun-are-the-build-path orders operator tooling Rust first.
//!
//! The launchers export `SJEL_ROOT`. A ported script's launcher also sources
//! `tools/lib/paths.sh` first, and this binary reads the `SJEL_*` variables that file exports
//! rather than re-deriving the overlay order, the same contract `tools/storage` keeps.

use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};

mod capability;
mod help;
mod paths;
mod persist;
mod registry;
mod runargs;
mod runner;
mod schedule;
mod search;
mod toolchain;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, rest) = match args.split_first() {
        Some((c, r)) => (c.as_str(), r),
        None => ("help", &[][..]),
    };
    // Ported tools/ scripts, run by their launchers under the script's own name.
    match command {
        "toolchain-check" => return toolchain::run(rest),
        "capability.sh" => return registry::run(rest),
        "service-runner.sh" => return runner::run(rest),
        _ => {}
    }
    let root = match std::env::var("SJEL_ROOT") {
        Ok(r) if !r.is_empty() => PathBuf::from(r),
        _ => {
            eprintln!(
                "sjel: SJEL_ROOT is unset — run sjel through its launcher at the repository root"
            );
            return ExitCode::from(2);
        }
    };
    sjel(&root, command, rest)
}

fn sjel(root: &Path, command: &str, rest: &[String]) -> ExitCode {
    let tool = |rel: &str| root.join(rel);
    match command {
        "help" => help(rest.first().map_or("", String::as_str)),
        "-h" | "--help" => {
            println!("{}", help::USAGE);
            ExitCode::SUCCESS
        }
        "search" => search::run(root, rest),
        "doctor" => exec(Command::new(tool("tools/doctor")).args(rest)),
        // stdio MCP server (ISA ISC-40), and its registration with each harness. The server is
        // still tools/sjel-mcp.ts; the registration half is Rust.
        "mcp" => match rest.first().map(String::as_str) {
            Some("register" | "unregister") => {
                exec(Command::new(tool("tools/sjel-mcp/sjel-mcp")).args(rest))
            }
            _ => exec(
                Command::new("bun")
                    .arg("run")
                    .arg(tool("tools/sjel-mcp.ts"))
                    .args(rest),
            ),
        },
        // This device's user-level Claude Code settings (ISA ISC-45).
        "claude" => {
            exec(Command::new(tool("tools/claude-code-config/claude-code-config")).args(rest))
        }
        "context" => exec(Command::new(tool("tools/axon-context")).args(rest)),
        // Each launcher owns its overlay resolution and lazy build, so there is one
        // implementation of both.
        "storage" => exec(Command::new(tool("tools/storage/storage")).args(rest)),
        "update" => exec(Command::new(tool("tools/updates")).args(rest)),
        // The job names are CI's own: tools/ci-local reads the step list out of
        // .github/workflows/ci.yml, so neither verb can drift from what the push will run.
        "gates" => exec(
            Command::new(tool("tools/ci-local"))
                .args(["run", "repo-gates"])
                .args(rest),
        ),
        "test" => exec(
            Command::new(tool("tools/ci-local"))
                .args(["run", "bun-tests"])
                .args(rest),
        ),
        "cargo" => exec(Command::new(tool("tools/cargo-hermetic")).args(rest)),
        "capability" => capability::command(root, rest),
        "agent" => match rest.first().map(String::as_str) {
            Some("enroll") => {
                exec(Command::new(tool("tools/capability-auth/capability-auth")).arg("enroll"))
            }
            _ => fail("usage: sjel agent enroll"),
        },
        "pack" => pack(root, rest),
        other => {
            eprintln!("sjel: unknown command '{other}'");
            eprintln!("Run 'sjel help'.");
            ExitCode::from(1)
        }
    }
}

fn help(topic: &str) -> ExitCode {
    let text = match topic {
        "" => help::USAGE,
        "capability" => help::CAPABILITY,
        "claude" => help::CLAUDE,
        "mcp" => help::MCP,
        "pack" => help::PACK,
        "context" => "Usage: sjel context with [capability] | on [unit-or-path]",
        "storage" => help::STORAGE,
        "update" => help::UPDATE,
        "gates" | "test" => help::GATES,
        "cargo" => help::CARGO,
        "doctor" => "Usage: sjel doctor",
        "search" => "Usage: sjel search <words...>",
        other => return fail(&format!("sjel: no help for '{other}'")),
    };
    println!("{text}");
    ExitCode::SUCCESS
}

/// Harness adapters. claude, opencode and pi run under bun; codex has its own launcher.
fn pack(root: &Path, args: &[String]) -> ExitCode {
    let command = args.first().map_or("", String::as_str);
    let harness = args.get(1).map_or("", String::as_str);
    let rest = args.get(2..).unwrap_or(&[]);
    let bun = |script: &str| {
        let mut c = Command::new("bun");
        c.arg("run").arg(root.join("tools").join(script));
        c
    };
    match command {
        "list" => match harness {
            "" | "opencode" => exec(bun("packs-opencode.ts").arg("list")),
            "claude" => exec(bun("packs-claude.ts").arg("status")),
            "codex" => exec(Command::new(root.join("tools/packs-codex")).args(["status", "--all"])),
            "pi" => exec(bun("packs-pi.ts").arg("list")),
            _ => fail(&format!("sjel: unknown harness '{harness}'")),
        },
        "status" | "deploy" | "sync" | "remove" | "use" => match harness {
            "" => fail(&format!("usage: sjel pack {command} <harness> ...")),
            "claude" => exec(bun("packs-claude.ts").arg(command).args(rest)),
            "codex" => exec(
                Command::new(root.join("tools/packs-codex"))
                    .arg(command)
                    .args(rest),
            ),
            "opencode" => exec(bun("packs-opencode.ts").arg(command).args(rest)),
            "pi" => exec(bun("packs-pi.ts").arg(command).args(rest)),
            _ => fail(&format!("sjel: unknown harness '{harness}'")),
        },
        _ => fail("usage: sjel pack {list|status|deploy|sync|remove|use} ..."),
    }
}

/// Replace this process with `cmd`, as the bash launcher's `exec` did. Returns only on failure.
fn exec(cmd: &mut Command) -> ExitCode {
    let err = cmd.exec();
    eprintln!(
        "sjel: cannot run {}: {err}",
        cmd.get_program().to_string_lossy()
    );
    ExitCode::from(127)
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("{msg}");
    ExitCode::from(1)
}

/// A child's exit status as this process's. A signal-terminated child reads as 1.
pub fn exit_with(status: std::io::Result<ExitStatus>) -> ExitCode {
    match status {
        Ok(s) => ExitCode::from(s.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(1)),
        Err(e) => {
            eprintln!("sjel: {e}");
            ExitCode::from(127)
        }
    }
}
