//! Explicit operator maintenance for the Calendar capability.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args == ["help"] || args == ["--help"] {
        println!(
            "usage: sjel calendar dedupe audit FROM TO PLAN.json\n       sjel calendar dedupe apply PLAN.json --apply\nAudits at most 90 days; only a reviewed apply command can delete exact copies."
        );
        return ExitCode::SUCCESS;
    }
    let result = match args.split_first() {
        Some((command, rest)) if command == "dedupe" => {
            calendar::google_sync::dedupe::command(rest)
        }
        _ => Err("unknown calendar command; run sjel calendar help".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("calendar: {error}");
            ExitCode::FAILURE
        }
    }
}
