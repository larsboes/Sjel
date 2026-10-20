use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};

use operator_profile::{
    model::{Harness, ProfileInput},
    render::{self, SectionChange},
    store::{profile_json, OperatorProfileStore, PutOutcome},
};

fn main() {
    if let Err(error) = run() {
        eprintln!("operator-profile: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = args
        .next()
        .ok_or("usage: operator-profile show|put|validate|export (--dry-run | --apply --expected-revision N)")?;
    let store = OperatorProfileStore::open(&sjel_config::database_path())?;
    match command.as_str() {
        "show" => {
            let profile = store.get()?;
            let stored = profile.is_some();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "stored": stored,
                    "profile": profile_json(profile)?
                }))?
            );
        }
        "validate" => {
            let input = read_profile(args.next().as_deref())?;
            input.validate().map_err(io::Error::other)?;
            println!(
                "valid: {} fields, {} statements",
                input.fields.len(),
                input.statements.len()
            );
        }
        "put" => {
            let mut expected = None;
            let mut source = None;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--expected-revision" => {
                        expected = Some(args.next().ok_or("missing revision")?.parse::<u64>()?)
                    }
                    "--input" => source = args.next(),
                    _ => return Err(format!("unknown argument {arg:?}").into()),
                }
            }
            let expected =
                expected.ok_or("put requires --expected-revision (use 0 for a new profile)")?;
            let input = read_profile(source.as_deref())?;
            input.validate().map_err(io::Error::other)?;
            match store.put(&input, expected)? {
                PutOutcome::Stored(profile) => {
                    println!("stored profile revision {}", profile.revision)
                }
                PutOutcome::Stale { current_revision } => {
                    return Err(
                        format!("stale profile; current revision is {current_revision}").into(),
                    );
                }
            }
        }
        "export" => {
            let mut harness = None;
            let mut dry_run = false;
            let mut apply = false;
            let mut expected_revision = None;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--dry-run" => dry_run = true,
                    "--apply" => apply = true,
                    "--expected-revision" => {
                        expected_revision =
                            Some(args.next().ok_or("missing revision")?.parse::<u64>()?)
                    }
                    "--harness" => harness = Some(args.next().ok_or("missing harness id")?),
                    _ => return Err(format!("unknown argument {arg:?}").into()),
                }
            }
            if dry_run && apply {
                return Err("export accepts either --dry-run or --apply, not both".into());
            }
            if !dry_run && !apply {
                return Err(
                    "export requires --dry-run (or --apply --expected-revision N); no assistant file is written by default"
                        .into(),
                );
            }
            let harness = match harness.as_deref().ok_or("export requires --harness")? {
                "claude" => Harness::Claude,
                "codex" => Harness::Codex,
                "pi" => Harness::Pi,
                "opencode" => Harness::Opencode,
                other => {
                    return Err(format!(
                        "no verified user-instruction target for harness {other:?}"
                    )
                    .into());
                }
            };
            let profile = store.get()?.ok_or("no profile is stored yet")?;
            if apply {
                let expected = expected_revision.ok_or(
                    "export --apply requires --expected-revision <N> matching the reviewed profile revision",
                )?;
                if profile.revision != expected {
                    return Err(format!(
                        "stale profile revision: expected {expected}, current is {}",
                        profile.revision
                    )
                    .into());
                }
            }
            let generated = render::render(&profile, harness)?;
            let target = target_for(harness)?;
            let existing = match std::fs::read_to_string(&target) {
                Ok(text) => text,
                Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
                Err(_) => {
                    return Err(
                        format!("cannot read instruction target {}", target.display()).into(),
                    );
                }
            };
            let current = render::managed_section(&existing)?.map(str::to_owned);
            let change = if apply {
                render::apply_document(&target, &existing, &generated)?
            } else {
                render::preview_document(&existing, &generated)?.1
            };
            let action = match (apply, change) {
                (false, SectionChange::Create) => "would create managed section",
                (false, SectionChange::Replace) => "would replace managed section",
                (false, SectionChange::Unchanged) => "managed section is current",
                (true, SectionChange::Create) => "created managed section",
                (true, SectionChange::Replace) => "replaced managed section",
                (true, SectionChange::Unchanged) => "managed section already current (no write needed)",
            };
            let audit = render::audit(&profile, harness);
            println!(
                "harness: {}\ntarget: {}\nprofile revision: {}\nresult: {}",
                harness.as_str(),
                target.display(),
                profile.revision,
                action,
            );
            println!("included entry ids: {}", audit.included.join(", "));
            for (id, reason) in audit.omitted {
                println!("omitted entry {id}: {reason}");
            }
            println!("\nmanaged-section diff:");
            if let Some(current) = current {
                for line in current.lines() {
                    println!("-{line}");
                }
            } else {
                println!("(no existing managed section)");
            }
            for line in generated.lines() {
                println!("+{line}");
            }
        }
        _ => {
            return Err(
                format!("unknown command {command:?}; use show|put|validate|export").into(),
            );
        }
    }
    Ok(())
}

fn read_profile(source: Option<&str>) -> Result<ProfileInput, Box<dyn std::error::Error>> {
    let mut input = String::new();
    match source {
        Some(path) if path != "-" => std::fs::read_to_string(path)?.clone_into(&mut input),
        _ => {
            io::stdin().read_to_string(&mut input)?;
        }
    }
    Ok(serde_json::from_str(&input)?)
}

fn target_for(harness: Harness) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let home = std::env::var_os("SJEL_OPERATOR_HOME")
        .or_else(|| std::env::var_os("HOME"))
        .ok_or("HOME is not set")?;
    Ok(render::target_path(Path::new(&home), harness))
}
