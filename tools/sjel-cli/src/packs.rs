//! The four per-harness Pack adapters — `tools/packs-claude.ts`, `tools/packs-codex.ts`,
//! `tools/packs-opencode.ts` and `tools/packs-pi.ts`, ported 2026-10-04.
//!
//! Each is a launcher-shaped CLI over machinery that already lived in Rust: the deployment
//! engine (`harnesses::engine` + `harnesses::mutate`) and pi's settings registry
//! (`harnesses::pi_settings`). What is here is only the verb surface — which verbs a harness
//! has, how its lines are shaped, and the exact messages it fails with. The four differ in
//! small, load-bearing ways (claude heads a multi-pack write with the Pack name, codex does not;
//! codex alone can `migrate-generated`; pi is a settings registry rather than a copy), so each
//! gets its own dispatcher over shared helpers rather than a flag table.
//!
//! `tools/packs.sh` stays a bash shim: its whole job is mapping the old `link`/`unlink` verbs
//! onto this one, and that translation is one `case` statement.

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::harnesses::engine::{self, DeployConfig, StatusRow};
use crate::harnesses::mutate;
use crate::harnesses::pi_settings;
use crate::harnesses::registry::{Harness, Registry};

const CLAUDE_HELP: &str = "\
tools/packs-claude — materialize Axon Packs into Claude Code.

  tools/packs-claude status [<pack>|--all]  show source/install/drift state
  tools/packs-claude deploy <pack>...       install one or more Packs
  tools/packs-claude adopt <pack>...        take ownership of identical copies already in place
  tools/packs-claude sync <pack>|--all      update already-deployed Packs
  tools/packs-claude remove <pack>...       remove one or more owned Packs
  tools/packs-claude use [<profile>]        activate a profile (or list them interactively)

Environment:
  CLAUDE_SKILLS_DIR       skill destination (default: $HOME/.claude/skills)
  CLAUDE_AGENTS_DIR       agents destination (default: $HOME/.claude/agents)
  SJEL_CLAUDE_STATE_FILE  ownership ledger override (mainly for tests)
";

const CODEX_HELP: &str = "\
tools/packs-codex — materialize Axon Packs.

  tools/packs-codex status [<pack>|--all]  show source/install/drift state
  tools/packs-codex deploy <pack>...       install one or more Packs
  tools/packs-codex sync <pack>|--all      update already-deployed Packs
  tools/packs-codex remove <pack>...       remove one or more owned Packs
  tools/packs-codex migrate-generated <pack> --accept-current
                                            adopt generated-artifact exclusions after review
  tools/packs-codex use [<profile>]        activate a profile (or pick interactively)

Environment:
  CODEX_SKILLS_DIR       destination (default: $HOME/.agents/skills)
  SJEL_CODEX_STATE_FILE ownership ledger override (mainly for tests)
";

const OPENCODE_USAGE: &str = "\
usage: tools/packs-opencode status [<pack>|--all]
       tools/packs-opencode deploy <pack>...
       tools/packs-opencode sync <pack>|--all
       tools/packs-opencode remove <pack>...
       tools/packs-opencode list";

const PI_USAGE: &str = "usage: tools/packs-pi list | status [pack ...] | deploy <pack ...> | \
                        sync <pack ...> | remove <pack ...> | use <profile>";

/// The ported entry point. `tool` is the launcher's own name, as `sjel_cli_exec` passes it.
pub fn run(tool: &str, args: &[String]) -> ExitCode {
    let root = match std::env::var("SJEL_ROOT").ok().filter(|r| !r.is_empty()) {
        Some(root) => PathBuf::from(root),
        None => {
            eprintln!("{tool}: SJEL_ROOT is unset — run it through its launcher");
            return ExitCode::from(2);
        }
    };
    let registry = Registry::new(&root);
    let result = match tool {
        "packs-claude" => claude(&registry, args),
        "packs-codex" => codex(&registry, args),
        "packs-opencode" => opencode(&registry, args),
        "packs-pi" => pi(&registry, args),
        other => Err(format!("unknown adapter '{other}'")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{tool}: {message}");
            ExitCode::from(1)
        }
    }
}

fn config(registry: &Registry, id: &str) -> Result<DeployConfig, String> {
    registry
        .by_id(id)
        .map(|harness: &Harness| harness.config.clone())
}

// ---- shared helpers ------------------------------------------------------------------------

/// The per-harness status listing: a Pack name, then one indented row per unit.
fn print_statuses(rows: &[StatusRow]) {
    let mut last = String::new();
    for row in rows {
        if row.pack != last {
            if !last.is_empty() {
                println!();
            }
            println!("{}", row.pack);
            last = row.pack.clone();
        }
        let detail = row
            .detail
            .as_deref()
            .map(|d| format!(" {d}"))
            .unwrap_or_default();
        println!("  {:24} [{}]{detail}", row.skill, row.status.as_str());
    }
}

/// Print a verb's lines: a Pack-headed, indented block for claude, bare lines for the rest.
fn emit(lines: &[String], header: Option<&str>) {
    match header {
        Some(pack) => {
            println!("{pack}");
            for line in lines {
                println!("  {line}");
            }
        }
        None => {
            for line in lines {
                println!("{line}");
            }
        }
    }
}

fn list(config: &DeployConfig, include_dedicated: bool) -> Result<(), String> {
    for pack in engine::available_packs(config, include_dedicated)? {
        println!("{pack}");
    }
    Ok(())
}

fn status(config: &DeployConfig, target: Option<&str>) -> Result<(), String> {
    let selected = target.filter(|t| *t != "--all");
    print_statuses(&engine::get_statuses(config, selected)?);
    Ok(())
}

fn deploy(config: &DeployConfig, packs: &[String], header: bool) -> Result<(), String> {
    for pack in packs {
        let lines = mutate::deploy_pack(config, pack, None)?;
        emit(&lines, if header { Some(pack) } else { None });
    }
    Ok(())
}

fn adopt(config: &DeployConfig, packs: &[String]) -> Result<(), String> {
    for pack in packs {
        let lines = mutate::adopt_pack(config, pack)?;
        emit(&lines, Some(pack));
    }
    Ok(())
}

fn sync(config: &DeployConfig, target: Option<&str>, header_on_all: bool) -> Result<(), String> {
    match target {
        Some("--all") => {
            let state = engine::read_state(config)?;
            for pack in state.packs.keys() {
                let lines = mutate::sync_pack(config, pack)?;
                emit(&lines, if header_on_all { Some(pack) } else { None });
            }
            Ok(())
        }
        Some(pack) => {
            let lines = mutate::sync_pack(config, pack)?;
            emit(&lines, None);
            Ok(())
        }
        None => Err("sync needs a pack or --all".to_string()),
    }
}

fn remove(config: &DeployConfig, packs: &[String], header: bool) -> Result<(), String> {
    for pack in packs {
        let lines = mutate::remove_pack(config, pack)?;
        emit(&lines, if header { Some(pack) } else { None });
    }
    Ok(())
}

/// `tools/packs-claude use`: a profile by name, or the list when none is given.
fn use_list(config: &DeployConfig, name: Option<&str>) -> Result<(), String> {
    if let Some(name) = name {
        return activate(config, name);
    }
    let profiles = mutate::read_profiles(config)?;
    if profiles.is_empty() {
        return Err("no profiles in profiles.toml".to_string());
    }
    println!("Profiles:");
    for profile in &profiles {
        println!("  {:12} {}", profile.name, profile.description);
    }
    Ok(())
}

fn activate(config: &DeployConfig, name: &str) -> Result<(), String> {
    let profiles = mutate::read_profiles(config)?;
    let profile = profiles
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| format!("no such profile: '{name}'"))?;
    for line in mutate::activate_profile(config, profile)? {
        println!("{line}");
    }
    Ok(())
}

/// `tools/packs-codex use` with no profile: the picker, which needs a terminal to answer it.
fn use_pick(config: &DeployConfig, name: Option<&str>) -> Result<(), String> {
    if let Some(name) = name {
        return activate(config, name);
    }
    let profiles = mutate::read_profiles(config)?;
    if profiles.is_empty() {
        println!("No profiles defined. Add them to profiles.toml.");
        return Ok(());
    }
    println!();
    for (index, profile) in profiles.iter().enumerate() {
        let active = if mutate::profile_active_packs(config, profile)?.is_empty() {
            ""
        } else {
            " [active]"
        };
        println!(
            "  {:3} {:20} {}{active}",
            index + 1,
            profile.name,
            profile.description
        );
    }
    print!("\nSelect profile (number or name): ");
    io::stdout().flush().map_err(|e| e.to_string())?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|e| e.to_string())?;
    let answer = answer.trim();
    let chosen = leading_int(answer)
        .filter(|number| *number >= 1 && *number <= profiles.len())
        .map(|number| &profiles[number - 1])
        .or_else(|| profiles.iter().find(|p| p.name == answer));
    match chosen {
        Some(profile) => {
            for line in mutate::activate_profile(config, profile)? {
                println!("{line}");
            }
            Ok(())
        }
        None => {
            println!("No profile matches '{answer}'");
            Ok(())
        }
    }
}

/// `parseInt(raw, 10)`: the leading digits, if any.
fn leading_int(raw: &str) -> Option<usize> {
    let digits: String = raw
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn help_or_default(args: &[String]) -> Option<&str> {
    args.first().map(String::as_str)
}

// ---- claude --------------------------------------------------------------------------------

fn claude(registry: &Registry, args: &[String]) -> Result<(), String> {
    let verb = help_or_default(args).unwrap_or("status");
    let rest = args.get(1..).unwrap_or(&[]);
    if matches!(verb, "-h" | "--help" | "help") {
        println!("{CLAUDE_HELP}");
        return Ok(());
    }
    let config = config(registry, "claude")?;
    match verb {
        "status" => status(&config, rest.first().map(String::as_str)),
        "list" => list(&config, false),
        "deploy" => {
            if rest.is_empty() {
                return Err("usage: tools/packs-claude deploy <pack>...".to_string());
            }
            deploy(&config, rest, true)
        }
        "adopt" => {
            if rest.is_empty() {
                return Err("usage: tools/packs-claude adopt <pack>...".to_string());
            }
            adopt(&config, rest)
        }
        "sync" => sync(&config, rest.first().map(String::as_str), true).map_err(|e| {
            if e == "sync needs a pack or --all" {
                "usage: tools/packs-claude sync <pack>|--all".to_string()
            } else {
                e
            }
        }),
        "remove" => {
            if rest.is_empty() {
                return Err("usage: tools/packs-claude remove <pack>...".to_string());
            }
            remove(&config, rest, true)
        }
        "use" => use_list(&config, rest.first().map(String::as_str)),
        other => Err(format!("unknown command '{other}'\n\n{CLAUDE_HELP}")),
    }
}

// ---- codex ---------------------------------------------------------------------------------

fn codex(registry: &Registry, args: &[String]) -> Result<(), String> {
    let verb = help_or_default(args).unwrap_or("status");
    let rest = args.get(1..).unwrap_or(&[]);
    if matches!(verb, "-h" | "--help" | "help") {
        println!("{CODEX_HELP}");
        return Ok(());
    }
    let config = config(registry, "codex")?;
    match verb {
        "status" => status(&config, rest.first().map(String::as_str)),
        "deploy" => {
            if rest.is_empty() {
                return Err("usage: tools/packs-codex deploy <pack>...".to_string());
            }
            deploy(&config, rest, false)
        }
        "sync" => sync(&config, rest.first().map(String::as_str), true).map_err(|e| {
            if e == "sync needs a pack or --all" {
                "usage: tools/packs-codex sync <pack>|--all".to_string()
            } else {
                e
            }
        }),
        "remove" => {
            if rest.is_empty() {
                return Err("usage: tools/packs-codex remove <pack>...".to_string());
            }
            remove(&config, rest, false)?;
            // ~/.agents/skills is also pi's global discovery root, so on a machine with pi
            // installed the destination has two readers and this removal costs pi those skills.
            let shared = engine::resolve(&home().join(".agents").join("skills"));
            if config.destination == shared && home().join(".pi/agent/settings.json").exists() {
                println!(
                    "note: pi is installed and discovers {} by default, so the removed unit(s) \
                     were loaded by pi until now. If pi should keep them, deploy them there before \
                     removing next time: tools/packs-pi deploy {}",
                    config.destination.display(),
                    rest.join(" ")
                );
            }
            Ok(())
        }
        "migrate-generated" => {
            let pack = rest.iter().find(|arg| !arg.starts_with("--"));
            let well_formed = pack.is_some_and(|pack| {
                rest.iter()
                    .all(|arg| arg == pack || arg == "--accept-current")
            });
            let Some(pack) = pack.filter(|_| well_formed) else {
                return Err(
                    "usage: tools/packs-codex migrate-generated <pack> --accept-current"
                        .to_string(),
                );
            };
            let accept = rest.iter().any(|arg| arg == "--accept-current");
            for line in mutate::migrate_generated_artifacts(&config, pack, accept)? {
                println!("{line}");
            }
            Ok(())
        }
        "profile" | "use" => use_pick(&config, rest.first().map(String::as_str)),
        other => Err(format!("unknown command '{other}'\n\n{CODEX_HELP}")),
    }
}

// ---- opencode ------------------------------------------------------------------------------

fn opencode(registry: &Registry, args: &[String]) -> Result<(), String> {
    let verb = help_or_default(args).unwrap_or("status");
    let rest = args.get(1..).unwrap_or(&[]);
    let config = config(registry, "opencode")?;
    // No `-h`: opencode's adapter never had one, and its usage is the error.
    match verb {
        "list" => list(&config, true),
        "status" => status(&config, rest.first().map(String::as_str)),
        "deploy" => {
            if rest.is_empty() {
                return Err(OPENCODE_USAGE.to_string());
            }
            deploy(&config, rest, false)
        }
        "sync" => {
            if rest.is_empty() {
                return Err(OPENCODE_USAGE.to_string());
            }
            sync(&config, rest.first().map(String::as_str), false)
        }
        "remove" => {
            if rest.is_empty() {
                return Err(OPENCODE_USAGE.to_string());
            }
            remove(&config, rest, false)
        }
        _ => Err(OPENCODE_USAGE.to_string()),
    }
}

// ---- pi ------------------------------------------------------------------------------------

fn pi(registry: &Registry, args: &[String]) -> Result<(), String> {
    let verb = help_or_default(args).unwrap_or("list");
    let rest = args.get(1..).unwrap_or(&[]);
    let config = config(registry, "pi")?;
    match verb {
        "list" => list(&config, true),
        "status" => {
            for line in pi_settings::status_lines(&config, rest)? {
                println!("{line}");
            }
            Ok(())
        }
        "deploy" | "sync" => {
            for line in pi_settings::deploy_packs(&config, rest)? {
                println!("{line}");
            }
            Ok(())
        }
        "remove" => {
            for line in pi_settings::remove_packs(&config, rest)? {
                println!("{line}");
            }
            Ok(())
        }
        "use" | "profile" => match rest.first() {
            Some(name) => {
                for line in pi_settings::activate_profile(&config, name)? {
                    println!("{line}");
                }
                Ok(())
            }
            None => Err("usage: tools/packs-pi use <profile>".to_string()),
        },
        _ => Err(PI_USAGE.to_string()),
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

/// The Packs every harness can see: the public roots plus the overlay's, dedicated ones
/// included. `sjel search` names them, and it used to shell `packs-opencode list` for this.
pub fn all_packs(root: &Path) -> Result<Vec<String>, String> {
    let registry = Registry::new(root);
    let config = config(&registry, "opencode")?;
    engine::available_packs(&config, true)
}

/// `sjel packs list` — the public Packs no dedicated deployer owns, one per line. It is the
/// read `tools/generate-marketplace.ts` needs and has no launcher of its own, so it is named
/// here rather than in the tools/ namespace: the marketplace is its only caller.
pub fn list_public(root: &Path, args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        None | Some("list") => {}
        Some("-h" | "--help") => {
            println!("usage: sjel packs list — public Packs, one per line");
            return ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("packs: unknown verb '{other}'");
            return ExitCode::from(1);
        }
    }
    let config = DeployConfig {
        axon_root: root.to_path_buf(),
        pack_roots: Some(vec![root.join("Packs")]),
        destination: PathBuf::new(),
        state_file: PathBuf::new(),
        adapter: "marketplace".to_string(),
        state_env_var: None,
        tree_convention: None,
        flat_file_convention: None,
        skip_manifest_skills: false,
        validate_adapter_files: None,
    };
    match engine::available_packs(&config, false) {
        Ok(packs) => {
            for pack in packs {
                println!("{pack}");
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("packs: {message}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_int_reads_digits_the_way_parse_int_did() {
        assert_eq!(leading_int("2"), Some(2));
        assert_eq!(leading_int("  12"), Some(12));
        assert_eq!(leading_int("2abc"), Some(2));
        assert_eq!(leading_int("home"), None);
        assert_eq!(leading_int("-1"), None);
    }
}
