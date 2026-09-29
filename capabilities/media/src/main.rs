use std::path::{Path, PathBuf};

use media::ingest::{self, IngestOptions};
use media::store::{Ledger, Result};
use media::{audit, index, verify_mirror, volume_uuid};

fn usage() {
    eprintln!("media — exact-byte index, ingest gate and mirror verification\n\
        usage:\n  media index --root PATH [--db PATH]\n  media audit --root PATH --sample N [--db PATH]\n  media ingest --staging PATH --library PATH [--apply] [--prune] [--db PATH]\n  media classify (--digest SHA256 | --digests-file PATH) [--db PATH]\n  media verify-mirror --left-root PATH --left-uuid UUID --right-root PATH --right-uuid UUID [--db PATH]\n  media status [--db PATH]\n\n\
        ingest is a dry run unless --apply is set. --prune additionally removes staging/originals\n\
        only after every new import verifies; neither verb deletes library content.");
}

fn option(args: &[String], key: &str) -> Result<Option<String>> {
    let mut value = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == key {
            if value.is_some() {
                return Err(format!("repeated {key}").into());
            }
            value = Some(
                args.get(i + 1)
                    .filter(|s| !s.starts_with("--"))
                    .ok_or_else(|| format!("{key} needs a value"))?
                    .clone(),
            );
            i += 2;
        } else {
            i += 1;
        }
    }
    Ok(value)
}
fn required(args: &[String], key: &str) -> Result<String> {
    option(args, key)?.ok_or_else(|| format!("missing {key}").into())
}
fn allowed(args: &[String], keys: &[&str], flags: &[&str]) -> Result<()> {
    let mut i = 0;
    while i < args.len() {
        if keys.contains(&args[i].as_str()) {
            if i + 1 >= args.len() || args[i + 1].starts_with("--") {
                return Err(format!("{} needs a value", args[i]).into());
            }
            i += 2;
        } else if flags.contains(&args[i].as_str()) {
            i += 1;
        } else {
            return Err(format!("unknown option {}", args[i]).into());
        }
    }
    Ok(())
}
fn rooted(args: &[String], key: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(required(args, key)?))
}
fn mounted(root: &Path, expected: Option<&str>) -> Result<String> {
    let actual = volume_uuid(root)?;
    if expected.is_some_and(|id| id != actual) {
        return Err(format!("{} is not the registered volume ({actual})", root.display()).into());
    }
    Ok(actual)
}
fn run(args: &[String]) -> Result<i32> {
    let Some((verb, opts)) = args.split_first() else {
        usage();
        return Ok(1);
    };
    if matches!(verb.as_str(), "help" | "--help" | "-h") {
        usage();
        return Ok(0);
    }
    let (keys, flags): (&[&str], &[&str]) = match verb.as_str() {
        "index" => (&["--root", "--db"], &[]),
        "audit" => (&["--root", "--sample", "--db"], &[]),
        "ingest" => (&["--staging", "--library", "--db"], &["--apply", "--prune"]),
        "classify" => (&["--digest", "--digests-file", "--db"], &[]),
        "verify-mirror" => (
            &[
                "--left-root",
                "--right-root",
                "--left-uuid",
                "--right-uuid",
                "--db",
            ],
            &[],
        ),
        "status" => (&["--db"], &[]),
        _ => {
            usage();
            return Err(format!("unknown command {verb}").into());
        }
    };
    allowed(opts, keys, flags)?;
    let db = option(opts, "--db")?
        .map(PathBuf::from)
        .unwrap_or_else(sjel_config::database_path);
    let ledger = Ledger::open(&db)?;
    match verb.as_str() {
        "index" => {
            let root = rooted(opts, "--root")?;
            let uuid = mounted(&root, None)?;
            let label = root
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("library");
            let hashed = index(&ledger, &root, &uuid, label)?;
            let report = audit(&ledger, &root, &uuid, 0)?;
            println!(
                "{}",
                serde_json::json!({"uuid":uuid,"hashed":hashed,"disk_files":report.disk_files,"indexed_locations":report.indexed_locations,"discrepancies":report.disagreements})
            );
            Ok(i32::from(!report.disagreements.is_empty()))
        }
        "audit" => {
            let root = rooted(opts, "--root")?;
            let uuid = mounted(&root, None)?;
            let sample = required(opts, "--sample")?.parse()?;
            let report = audit(&ledger, &root, &uuid, sample)?;
            let fail = !report.disagreements.is_empty();
            println!("{}", serde_json::to_string(&report)?);
            Ok(i32::from(fail))
        }
        "ingest" => {
            let apply = opts.iter().any(|s| s == "--apply");
            if !apply && opts.iter().any(|s| s == "--prune") {
                return Err("--prune requires --apply".into());
            }
            let library = rooted(opts, "--library")?;
            let uuid = mounted(&library, None)?;
            let staging = rooted(opts, "--staging")?;
            let pending = ledger.pending_paths(&staging.canonicalize()?.to_string_lossy())?;
            let inventory = audit(&ledger, &library, &uuid, 0)?;
            let unexpected = inventory
                .disagreements
                .iter()
                .filter(|d| {
                    d.strip_prefix("not indexed: ")
                        .is_none_or(|rel| !pending.contains(rel))
                })
                .count();
            if unexpected > 0 {
                return Err(format!("library index differs from disk ({unexpected} paths); run media index and audit before ingest").into());
            }
            if apply {
                ledger.register(
                    &uuid,
                    library
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("library"),
                )?;
            }
            let report = ingest::ingest(
                &ledger,
                &IngestOptions {
                    staging: &staging,
                    library: &library,
                    uuid: &uuid,
                    apply,
                    prune: opts.iter().any(|s| s == "--prune"),
                    fail_before_verify: false,
                },
            )?;
            let fail = report.failed > 0 || report.refused > 0;
            println!("{}", serde_json::to_string(&report)?);
            Ok(i32::from(fail))
        }
        "classify" => {
            let digests = match (option(opts, "--digest")?, option(opts, "--digests-file")?) {
                (Some(one), None) => vec![one],
                (None, Some(path)) => std::fs::read_to_string(path)?
                    .lines()
                    .map(str::to_owned)
                    .collect(),
                _ => return Err("choose exactly one of --digest and --digests-file".into()),
            };
            for input in digests {
                if input.len() != 64 || !input.bytes().all(|c| c.is_ascii_hexdigit()) {
                    return Err("digest must be 64 hex characters".into());
                }
                let digest = input.to_ascii_lowercase();
                println!(
                    "{}",
                    serde_json::json!({"digest":digest,"present":ledger.has_digest(&digest)?})
                );
            }
            Ok(0)
        }
        "verify-mirror" => {
            let a = rooted(opts, "--left-root")?;
            let b = rooted(opts, "--right-root")?;
            let au = required(opts, "--left-uuid")?.to_ascii_uppercase();
            let bu = required(opts, "--right-uuid")?.to_ascii_uppercase();
            if au == bu {
                return Err("mirror volumes must have different UUIDs".into());
            }
            let report = verify_mirror(&ledger, (&a, &au), (&b, &bu))?;
            let fail = !report.discrepancies.is_empty();
            println!("{}", serde_json::to_string(&report)?);
            Ok(i32::from(fail))
        }
        "status" => {
            let (files, locations) = ledger.counts()?;
            println!(
                "{}",
                serde_json::json!({"files":files,"locations":locations})
            );
            Ok(0)
        }
        _ => unreachable!(),
    }
}
fn main() {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("media: {e}");
            std::process::exit(1);
        }
    }
}
