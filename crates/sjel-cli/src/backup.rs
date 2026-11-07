//! `sjel backup policy`: the backup timer's interval per target, the same store and checks as
//! `POST /api/sjel-status/backup/policy` (`crates/sjel-status/src/status/backup.rs`).

use std::path::Path;
use std::process::ExitCode;

use backup::BackupStore;

const USAGE: &str = "usage: sjel backup policy [<target> <hours>|off]";

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    match run_inner(root, args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sjel backup: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner(root: &Path, args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let store = BackupStore::open(&sjel_config::database_path())?;
    match args.as_slice() {
        ["policy"] => {}
        ["policy", target, interval] => {
            let hours = match *interval {
                "off" => None,
                value => Some(value.parse::<i64>().map_err(|_| USAGE)?),
            };
            let declared = backup::targets::declared(root)?;
            if !declared.iter().any(|t| t.id == *target) {
                return Err(format!("'{target}' is not a declared backup target").into());
            }
            store.refresh_targets(&declared)?;
            store.set_policy(target, hours)?;
        }
        _ => return Err(USAGE.into()),
    }
    let active = store.active_policies()?;
    if active.is_empty() {
        println!("No backup timer is on.");
    }
    for (target, hours) in active {
        println!("{target}: every {hours}h");
    }
    Ok(())
}
