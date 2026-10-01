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
        .ok_or("usage: operator-profile show|put|validate|export --dry-run")?;
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
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--dry-run" => dry_run = true,
                    "--harness" => harness = Some(args.next().ok_or("missing harness id")?),
                    "--apply" => {
                        return Err(
                            "file writes are not implemented; export is dry-run only".into()
                        );
                    }
                    _ => return Err(format!("unknown argument {arg:?}").into()),
                }
            }
            if !dry_run {
                return Err("export requires --dry-run; no assistant file is written".into());
            }
            let harness = match harness.as_deref().ok_or("export requires --harness")? {
                "claude" => Harness::Claude,
                other => {
                    return Err(format!(
                        "no verified user-instruction target for harness {other:?}"
                    )
                    .into());
                }
            };
            let profile = store.get()?.ok_or("no profile is stored yet")?;
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
            let (_, change) = render::preview_document(&existing, &generated)?;
            let action = match change {
                SectionChange::Create => "would create managed section",
                SectionChange::Replace => "would replace managed section",
                SectionChange::Unchanged => "managed section is current",
            };
            let audit = render::audit(&profile, harness);
            let current = render::managed_section(&existing)?;
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
    if harness != Harness::Claude {
        return Err(format!(
            "no verified user-instruction target for harness '{}'",
            harness.as_str()
        )
        .into());
    }
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(render::claude_target(Path::new(&home)))
}
