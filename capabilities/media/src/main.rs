use std::path::{Path, PathBuf};

use media::ingest::{self, IngestOptions};
use media::store::{Ledger, Result};
use media::{audit, index, verify_mirror, volume_uuid};

fn usage() {
    eprintln!("media — exact-byte index, ingest gate and mirror verification\n\
        usage:\n  media volume-id --root PATH\n  media index --root PATH --uuid UUID [--db PATH]\n  media audit --root PATH --uuid UUID --sample N [--db PATH]\n  media ingest --staging PATH --library PATH --uuid UUID [--apply] [--prune] [--db PATH]\n  media classify (--digest SHA256 | --digests-file PATH) [--db PATH]\n  media verify-mirror --left-root PATH --left-uuid UUID --right-root PATH --right-uuid UUID [--db PATH]\n  media status [--db PATH]\n  media duplicates --uuid UUID [--legacy PREFIX] [--resolve-inside PREFIX --root PATH --metadata] [--list PATH] [--db PATH]\n  media supersede --root PATH --uuid UUID --list PATH --quarantine PATH --journal PATH [--apply]\n  media preview --structure FILE [--metadata]\n  media organize --structure FILE [--apply --journal PATH] [--settled-for SECONDS] [--only COLLECTION]\n\n\
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
fn mounted(root: &Path, expected: &str) -> Result<String> {
    let actual = volume_uuid(root)?;
    if expected.to_ascii_uppercase() != actual {
        return Err(format!(
            "{} has volume UUID {actual}, expected {expected}; refusing to use this mount",
            root.display()
        )
        .into());
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
    // Preview and organize never reach configuration/database resolution, even on invalid arguments.
    if verb == "preview" {
        return match run_preview(opts) {
            Ok(code) => Ok(code),
            Err(error) => {
                println!(
                    "{}",
                    serde_json::json!({"complete": false, "moves_authorized": false, "error": error.to_string()})
                );
                Ok(1)
            }
        };
    }
    if verb == "organize" {
        return match run_organize(opts) {
            Ok(code) => Ok(code),
            Err(error) => {
                println!(
                    "{}",
                    serde_json::json!({"complete": false, "applied": false, "moves_authorized": true, "error": error.to_string()})
                );
                Ok(1)
            }
        };
    }
    let (keys, flags): (&[&str], &[&str]) = match verb.as_str() {
        "volume-id" => (&["--root"], &[]),
        "index" => (&["--root", "--uuid", "--db"], &[]),
        "audit" => (&["--root", "--uuid", "--sample", "--db"], &[]),
        "ingest" => (
            &["--staging", "--library", "--uuid", "--db"],
            &["--apply", "--prune"],
        ),
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
        "duplicates" => (
            &[
                "--uuid",
                "--db",
                "--legacy",
                "--list",
                "--root",
                "--resolve-inside",
            ],
            &["--metadata"],
        ),
        "supersede" => (
            &[
                "--root",
                "--uuid",
                "--list",
                "--quarantine",
                "--journal",
                "--db",
            ],
            &["--apply"],
        ),
        _ => {
            usage();
            return Err(format!("unknown command {verb}").into());
        }
    };
    allowed(opts, keys, flags)?;
    if verb == "volume-id" {
        let root = rooted(opts, "--root")?;
        println!("{}", serde_json::json!({"uuid": volume_uuid(&root)?}));
        return Ok(0);
    }
    // Mount identity and required arguments are checked before open_pool can migrate
    // the shared database. A removed drive must not be registered as the host volume.
    let mounted_uuid = match verb.as_str() {
        "index" | "audit" => Some(mounted(
            &rooted(opts, "--root")?,
            &required(opts, "--uuid")?,
        )?),
        "ingest" => Some(mounted(
            &rooted(opts, "--library")?,
            &required(opts, "--uuid")?,
        )?),
        _ => None,
    };
    let db = option(opts, "--db")?
        .map(PathBuf::from)
        .unwrap_or_else(sjel_config::database_path);
    let ledger = Ledger::open(&db)?;
    match verb.as_str() {
        "index" => {
            let root = rooted(opts, "--root")?;
            let uuid = mounted_uuid
                .as_deref()
                .ok_or("index needs a mounted volume")?;
            let label = root
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("library");
            let counted = index(&ledger, &root, uuid, label)?;
            let report = audit(&ledger, &root, uuid, 0)?;
            println!(
                "{}",
                serde_json::json!({"uuid":uuid,"hashed":counted.hashed,"pruned":counted.pruned,"disk_files":report.disk_files,"indexed_locations":report.indexed_locations,"discrepancies":report.disagreements})
            );
            Ok(i32::from(!report.disagreements.is_empty()))
        }
        "audit" => {
            let root = rooted(opts, "--root")?;
            let uuid = mounted_uuid
                .as_deref()
                .ok_or("audit needs a mounted volume")?;
            let sample = required(opts, "--sample")?.parse()?;
            let report = audit(&ledger, &root, uuid, sample)?;
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
            let uuid = mounted_uuid
                .as_deref()
                .ok_or("ingest needs a mounted volume")?;
            let staging = rooted(opts, "--staging")?;
            let pending = ledger.pending_paths(&staging.canonicalize()?.to_string_lossy())?;
            let inventory = audit(&ledger, &library, uuid, 0)?;
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
                    uuid,
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
                    uuid,
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
        "duplicates" => {
            let uuid = required(opts, "--uuid")?;
            let legacy = option(opts, "--legacy")?;
            let list = option(opts, "--list")?.map(PathBuf::from);
            let root = option(opts, "--root")?.map(PathBuf::from);
            let inside = option(opts, "--resolve-inside")?;
            if inside.is_some() && root.is_none() {
                return Err("--resolve-inside needs --root so the files can be read".into());
            }
            let resolve = inside.as_deref().map(|inside| media::duplicates::Resolve {
                inside,
                metadata: opts.iter().any(|arg| arg == "--metadata"),
            });
            let report = media::duplicates::duplicates(
                &ledger,
                &uuid,
                root.as_deref().unwrap_or_else(|| Path::new("/")),
                legacy.as_deref(),
                resolve,
                list.as_deref(),
            )?;
            println!("{}", serde_json::to_string(&report)?);
            Ok(0)
        }
        "supersede" => {
            let root = rooted(opts, "--root")?;
            let uuid = mounted(&root, &required(opts, "--uuid")?)?;
            let list = rooted(opts, "--list")?;
            let quarantine = rooted(opts, "--quarantine")?;
            let journal = rooted(opts, "--journal")?;
            let report = media::supersede::supersede(
                &root,
                &list,
                &quarantine,
                &journal,
                opts.iter().any(|arg| arg == "--apply"),
            )?;
            // A declared removal is not an absence: the index row goes with the file, and the
            // journal is the record of why. An undeclared disappearance still shows up in audit.
            for relpath in &report.quarantined_paths {
                ledger.forget(&uuid, relpath)?;
            }
            let fail = !report.complete || report.refused > 0;
            println!("{}", serde_json::to_string(&report)?);
            Ok(i32::from(fail))
        }
        _ => unreachable!(),
    }
}
fn run_preview(opts: &[String]) -> Result<i32> {
    allowed(opts, &["--structure"], &["--metadata"])?;
    if opts
        .iter()
        .filter(|arg| arg.as_str() == "--metadata")
        .count()
        > 1
    {
        return Err("repeated --metadata".into());
    }
    let structure = rooted(opts, "--structure")?;
    let report = media::preview::preview(&structure, opts.iter().any(|arg| arg == "--metadata"))?;
    let code = i32::from(!report.complete);
    println!("{}", serde_json::to_string(&report)?);
    Ok(code)
}

fn run_organize(opts: &[String]) -> Result<i32> {
    allowed(
        opts,
        &["--structure", "--journal", "--settled-for", "--only"],
        &["--apply"],
    )?;
    for flag in ["--apply", "--journal", "--structure", "--settled-for"] {
        if opts.iter().filter(|arg| arg.as_str() == flag).count() > 1 {
            return Err(format!("repeated {flag}").into());
        }
    }
    let only: Vec<String> = only_values(opts, "--only")?;
    if only.iter().any(|name| name.is_empty() || name.contains('/')) {
        return Err("--only takes a collection name, not a path".into());
    }
    if only.len() != only.iter().collect::<std::collections::BTreeSet<_>>().len() {
        return Err("repeated --only collection".into());
    }
    let structure = rooted(opts, "--structure")?;
    let journal = option(opts, "--journal")?.map(PathBuf::from);
    let settled_for = match option(opts, "--settled-for")? {
        Some(text) => text
            .parse::<i64>()
            .map_err(|_| "--settled-for must be a whole number of seconds")?,
        None => 300,
    };
    if settled_for < 0 {
        return Err("--settled-for must not be negative".into());
    }
    let report = media::organize::organize(
        &structure,
        opts.iter().any(|arg| arg == "--apply"),
        settled_for,
        journal.as_deref(),
        &only,
    )?;
    let code = i32::from(!report.complete);
    println!("{}", serde_json::to_string(&report)?);
    Ok(code)
}

/// Every value of a repeatable option, in order. `option` refuses a repeat; `--only` is the one
/// option where repeating is the point.
fn only_values(args: &[String], key: &str) -> Result<Vec<String>> {
    let mut values = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == key {
            match args.get(i + 1) {
                Some(value) if !value.starts_with("--") => values.push(value.clone()),
                _ => return Err(format!("{key} needs a value").into()),
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    Ok(values)
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

#[cfg(test)]
mod preview_tests {
    use super::*;

    #[test]
    fn preview_rejects_unknown_repeated_and_execution_flags_before_database_resolution() {
        for options in [
            vec![],
            vec!["--structure"],
            vec!["--structure", "missing", "--db", "scratch.db"],
            vec!["--structure", "missing", "--apply"],
            vec!["--structure", "missing", "--prune"],
            vec!["--structure", "missing", "--unknown"],
            vec!["--structure", "missing", "--metadata", "--metadata"],
            vec!["--structure", "missing", "--structure", "another"],
            vec!["--structure", "missing", "--only"],
            vec!["--structure", "missing", "--only", "a", "--only", "a"],
            vec!["--structure", "missing", "--only", "Trips/2016/x"],
        ] {
            let mut args = vec!["preview".to_owned()];
            args.extend(options.into_iter().map(str::to_owned));
            assert_eq!(run(&args).unwrap(), 1);
        }
    }
}
