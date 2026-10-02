//! sjel-cli — Sjel's operator tooling as one binary.
//!
//! Each subcommand replaces one interpreted `tools/` script. The script stays at its path as a
//! launcher (`tools/lib/sjel-cli.sh`), so every caller, test and document that names the script
//! keeps working while the logic moves here. Decided 2026-10-02: one crate rather than one crate
//! per tool, so repository, overlay and manifest resolution are written once (`paths`), and
//! CONTRIBUTING.md#cargo-and-bun-are-the-build-path already orders operator tooling Rust first.
//!
//! The launcher sources `tools/lib/paths.sh` before it execs this binary. This binary reads the
//! `SJEL_*` variables that file exports and never re-derives the overlay order itself, the same
//! contract `tools/storage` keeps.
//!
//! Exit codes are each subcommand's own, kept identical to the script it replaced. `2` is a
//! usage error everywhere.

use std::process::ExitCode;

mod paths;
mod toolchain;

const USAGE: &str = "\
usage: sjel-cli <command> [args...]

commands:
  toolchain-check   is every host tool Sjel needs installed? (tools/toolchain-check)

Run through the launcher at tools/<command>, which resolves SJEL_ROOT and the overlay.";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    let rest: Vec<String> = args.collect();
    match command.as_deref() {
        Some("toolchain-check") => toolchain::run(&rest),
        Some("-h" | "--help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("sjel-cli: unknown command '{other}'\n\n{USAGE}");
            ExitCode::from(2)
        }
        None => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
