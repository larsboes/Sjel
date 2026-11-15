//! Backup inspection through the capability caller and local timer policy management.

use std::path::Path;
use std::process::ExitCode;

use backup::BackupStore;

const USAGE: &str =
    "usage: sjel backup status | runs [--limit 1..500] | policy [<target> <hours>|off]";

fn read_route(args: &[&str]) -> Result<Option<String>, &'static str> {
    match args {
        ["status"] => Ok(Some("/api/sjel-status/backups".into())),
        ["runs"] => Ok(Some("/api/sjel-status/backup/runs".into())),
        ["runs", "--limit", limit] => {
            let limit: u16 = limit.parse().map_err(|_| USAGE)?;
            if !(1..=500).contains(&limit) {
                return Err(USAGE);
            }
            Ok(Some(format!("/api/sjel-status/backup/runs?limit={limit}")))
        }
        ["policy"] | ["policy", _, _] => Ok(None),
        _ => Err(USAGE),
    }
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let words: Vec<_> = args.iter().map(String::as_str).collect();
    match read_route(&words) {
        Ok(Some(route)) => {
            return crate::capability::command(
                root,
                &[
                    "call".into(),
                    "sjel-status".into(),
                    "get".into(),
                    route,
                    "--connect-timeout".into(),
                    "2".into(),
                    "--max-time".into(),
                    "15".into(),
                ],
            );
        }
        Err(error) => {
            eprintln!("sjel backup: {error}");
            return ExitCode::FAILURE;
        }
        Ok(None) => {}
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_routes_use_the_existing_api_and_bound_the_limit() {
        assert_eq!(
            read_route(&["status"]).unwrap().unwrap(),
            "/api/sjel-status/backups"
        );
        assert_eq!(
            read_route(&["runs"]).unwrap().unwrap(),
            "/api/sjel-status/backup/runs"
        );
        for limit in ["1", "50", "500"] {
            assert_eq!(
                read_route(&["runs", "--limit", limit]).unwrap().unwrap(),
                format!("/api/sjel-status/backup/runs?limit={limit}")
            );
        }
        for args in [
            vec!["runs", "--limit", "0"],
            vec!["runs", "--limit", "501"],
            vec!["runs", "--limit", "-1"],
            vec!["runs", "--limit", "no"],
            vec!["runs", "--limit"],
            vec!["status", "extra"],
        ] {
            assert!(read_route(&args).is_err(), "{args:?}");
        }
        assert_eq!(read_route(&["policy"]).unwrap(), None);
        assert_eq!(read_route(&["policy", "target", "off"]).unwrap(), None);
    }
}
