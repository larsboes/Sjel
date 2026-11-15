// tools/claude-code-config/src/main.rs — apply and check Sjel's user-level Claude Code
// settings (the managed layer this replaced retired 2026-10-02).
//
// What changed and why: the security floor used to be deployed as a root-owned managed policy
// at /Library/Application Support/ClaudeCode/managed-settings.json, together with two locks
// (allowManagedPermissionRulesOnly, allowManagedMcpServersOnly) and an MCP allowlist. That
// layer was agent-proof — a session could not edit a root-owned file — and it was also
// per-device, needed sudo for every change, and its MCP allowlist silently blocked this
// machine's own MCP server. The principal's ruling of 2026-10-02 moved
// the floor to the user level: one file, no root, nothing to grasp twice.
//
// The trade is recorded rather than hidden. A user-level file is writable by the agent
// sessions that run as that user, so the floor is no longer something a session *cannot*
// lift, only something it *should not*. `check` exists for that: it reports drift, and this
// tool is the only thing in the tree that would notice.
//
// Two managed-only keys could not come along, and both are named in
// tools/templates/claude-code/settings.base.json's README rather than silently dropped:
// disableSideloadFlags (so --mcp-config, --plugin-dir, --plugin-url and --agents are
// accepted again) and sandbox.enabledPlatforms (inert here; it only ever mattered on
// Windows).
//
// Usage: sjel claude [apply|check] [--force] [--dry-run]

mod atomic;
mod check;
mod merge;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::Value;

const EXIT_USAGE: u8 = 1;
const EXIT_UNPARSEABLE: u8 = 2;
const EXIT_DRIFT: u8 = 3;

const USAGE: &str = "\
tools/claude-code-config — apply and check Sjel's user-level Claude Code settings.

  tools/claude-code-config/claude-code-config            merge the baseline into the settings file
  tools/claude-code-config/claude-code-config apply      the same thing, spelled out
  tools/claude-code-config/claude-code-config check      report drift from the baseline
  tools/claude-code-config/claude-code-config --force    overwrite the values the baseline declares
  tools/claude-code-config/claude-code-config --dry-run  report what would change, write nothing
  tools/claude-code-config/claude-code-config -h         this help

apply merges with existing-wins: your own values always survive, so re-running is safe and it
only ever adds what is missing. --force is the opposite leaf rule, and exists to restore the
floor after something edited it.

Target: $CLAUDE_CONFIG_DIR/settings.json, else ~/.claude/settings.json.
Baseline: tools/templates/claude-code/settings.base.json, or $SJEL_CLAUDE_TEMPLATE, laid over
with this deployment's own <overlay>/config/claude-code/settings.fragment.json when one exists
($SJEL_CLAUDE_FRAGMENT overrides; the former managed-settings.fragment.json name is still
read, with a note).

Exit: 0 done or nothing to change, 1 usage or an unreadable baseline, 2 the target is not
valid JSON and was left alone, 3 check found drift.
";

#[derive(PartialEq, Eq, Debug)]
enum Verb {
    Apply,
    Check,
}

#[derive(Debug)]
struct Opts {
    verb: Verb,
    dry_run: bool,
    force: bool,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut opts = Opts {
        verb: Verb::Apply,
        dry_run: false,
        force: false,
    };
    for arg in args {
        match arg.as_str() {
            "apply" => opts.verb = Verb::Apply,
            "check" => opts.verb = Verb::Check,
            "--dry-run" | "-n" => opts.dry_run = true,
            "--force" | "-f" => opts.force = true,
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    Ok(opts)
}

/// `~/` at the front of a configured path means the home directory, as it does everywhere
/// else in Sjel's config.
fn expand_home(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(raw)
}

fn template_path() -> Result<PathBuf, String> {
    if let Some(raw) = env::var_os("SJEL_CLAUDE_TEMPLATE") {
        return Ok(expand_home(&raw.to_string_lossy()));
    }
    let root = env::var_os("SJEL_ROOT").ok_or(
        "SJEL_ROOT is not set — run this through tools/claude-code-config/claude-code-config, which resolves it",
    )?;
    Ok(PathBuf::from(root).join("tools/templates/claude-code/settings.base.json"))
}

fn target_path() -> Result<PathBuf, String> {
    // CLAUDE_CONFIG_DIR is the harness's own environment variable; honouring it is the same
    // rule tools/packs.sh applies to CLAUDE_SKILLS_DIR.
    if let Some(dir) = env::var_os("CLAUDE_CONFIG_DIR") {
        return Ok(expand_home(&dir.to_string_lossy()).join("settings.json"));
    }
    let home = env::var_os("HOME")
        .ok_or("neither HOME nor CLAUDE_CONFIG_DIR is set, so there is no target file")?;
    Ok(PathBuf::from(home).join(".claude/settings.json"))
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&text).map_err(|error| error.to_string())
}

struct Baseline {
    value: Value,
    /// Where it came from, in the order it was laid down. Printed only when a fragment
    /// contributed: a private overlay file that quietly changes the floor should say so.
    sources: Vec<String>,
}

/// The deployment's own additions. These live in the overlay, never in the public template,
/// because they name protected paths and credential identifiers that are not this repository's
/// to publish. Retiring the managed layer had to keep this channel: the machine that runs this
/// has eight deny rules and fifteen sandbox and credential protections of its own, and losing
/// them silently was the one outcome worth the extra code.
fn fragment_path() -> Option<(PathBuf, bool)> {
    if let Some(raw) = env::var_os("SJEL_CLAUDE_FRAGMENT") {
        let path = expand_home(&raw.to_string_lossy());
        return path.exists().then_some((path, false));
    }
    let overlay = env::var_os("SJEL_OVERLAY_ROOT").or_else(|| env::var_os("SJEL_PERSONAL_ROOT"))?;
    let dir = PathBuf::from(overlay).join("config/claude-code");
    let preferred = dir.join("settings.fragment.json");
    if preferred.exists() {
        return Some((preferred, false));
    }
    // Read the old name rather than ignore rules because a file was not renamed. It is
    // reported as legacy so the rename is a choice, not something the tool does silently.
    let legacy = dir.join("managed-settings.fragment.json");
    legacy.exists().then_some((legacy, true))
}

fn assemble_baseline(base_path: &Path) -> Result<Baseline, String> {
    let base = read_json(base_path)
        .map_err(|error| format!("baseline unreadable at {}: {error}", base_path.display()))?;
    let mut sources = vec![base_path.display().to_string()];
    let mut value = merge::strip_documentation(&base);
    if let Some((path, legacy)) = fragment_path() {
        let fragment = read_json(&path)
            .map_err(|error| format!("fragment unreadable at {}: {error}", path.display()))?;
        value = merge::merge_over(&value, &merge::strip_documentation(&fragment));
        sources.push(if legacy {
            format!(
                "{} (legacy name — rename it to settings.fragment.json)",
                path.display()
            )
        } else {
            path.display().to_string()
        });
    } else if env::var_os("SJEL_OVERLAY_ROOT").is_none() {
        sources.push("no overlay resolved, so no deployment fragment was applied".to_string());
    }
    if let Some(overlay) =
        env::var_os("SJEL_OVERLAY_ROOT").or_else(|| env::var_os("SJEL_PERSONAL_ROOT"))
    {
        let home = env::var_os("HOME").map(PathBuf::from);
        deny_overlay_data(&mut value, &PathBuf::from(overlay), home.as_deref());
        sources.push("the overlay's data/ closed to direct reads and writes".to_string());
    }
    if let Some(target) = env::var_os("CARGO_TARGET_DIR").filter(|t| !t.is_empty()) {
        push_unique(
            &mut value,
            &["sandbox", "filesystem", "allowWrite"],
            PathBuf::from(target).display().to_string(),
        );
    }
    Ok(Baseline { value, sources })
}

/// Append `entry` to the string array at `pointer`, creating the path, once.
fn push_unique(value: &mut Value, pointer: &[&str], entry: String) {
    let mut node = &mut *value;
    for key in pointer {
        if !node.is_object() {
            *node = merge::empty_object();
        }
        node = node
            .as_object_mut()
            .expect("an object was just ensured")
            .entry(key.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
    }
    if let Some(list) = node.as_array_mut() {
        let entry = Value::String(entry);
        if !list.contains(&entry) {
            list.push(entry);
        }
    }
}

/// Close `<overlay>/data/` to the session's file tools and its sandboxed commands. Personal data
/// reaches an agent only through the Sjel gate, which pseudonymizes it and withholds `c3`
/// (`docs/plans/containment.md`, C1). The path is derived here, so no private path is published.
fn deny_overlay_data(value: &mut Value, overlay: &Path, home: Option<&Path>) {
    let data = overlay.join("data");
    let (glob, rule_path) = match home.and_then(|home| data.strip_prefix(home).ok()) {
        Some(relative) => {
            let tilde = format!("~/{}/**", relative.display());
            (tilde.clone(), tilde)
        }
        None => (
            format!("{}/**", data.display()),
            format!("/{}/**", data.display()),
        ),
    };
    let push = push_unique;
    push(value, &["sandbox", "filesystem", "denyRead"], glob.clone());
    push(value, &["sandbox", "filesystem", "denyWrite"], glob);
    // `Edit(path)` covers every file-editing tool; Claude Code ignores `Write(path)` rules.
    for tool in ["Read", "Edit"] {
        push(
            value,
            &["permissions", "deny"],
            format!("{tool}({rule_path})"),
        );
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let opts = match parse(&args) {
        Ok(opts) => opts,
        Err(error) => {
            eprintln!("claude-code-config: {error}");
            eprint!("{USAGE}");
            return ExitCode::from(EXIT_USAGE);
        }
    };

    let template = match template_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("claude-code-config: {error}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let baseline = match assemble_baseline(&template) {
        Ok(baseline) => baseline,
        Err(error) => {
            eprintln!("claude-code-config: {error}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let target = match target_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("claude-code-config: {error}");
            return ExitCode::from(EXIT_USAGE);
        }
    };

    let deployed = if target.exists() {
        match read_json(&target) {
            Ok(value) => value,
            Err(error) => {
                // Never clobber a file we cannot read: it may be a hand edit mid-flight, and
                // the rules it holds are the ones Claude Code is running on right now.
                eprintln!(
                    "claude-code-config: {} exists but is not valid JSON — refusing to overwrite it.",
                    target.display()
                );
                eprintln!("  {error}");
                return ExitCode::from(EXIT_UNPARSEABLE);
            }
        }
    } else {
        merge::empty_object()
    };

    if baseline.sources.len() > 1 {
        println!(
            "claude-code-config: baseline is {} laid over {}",
            baseline.sources[1..].join(", "),
            baseline.sources[0]
        );
    }

    match opts.verb {
        Verb::Check => report_drift(&target, &deployed, &baseline.value),
        Verb::Apply => apply(&opts, &target, deployed, &baseline.value),
    }
}

fn report_drift(target: &Path, deployed: &Value, baseline: &Value) -> ExitCode {
    let found = check::drift(deployed, baseline);
    if found.is_clean() {
        println!(
            "claude-code-config: {} matches the baseline ({}) — no drift.",
            target.display(),
            check::short_digest(baseline)
        );
        return ExitCode::SUCCESS;
    }
    println!(
        "claude-code-config: {} has drifted from the baseline.",
        target.display()
    );
    for path in &found.differing {
        println!("  changed  {path}");
    }
    for path in &found.missing {
        println!("  missing  {path}");
    }
    println!("  baseline {}", check::short_digest(baseline));
    println!("  deployed {}", check::short_digest(deployed));
    println!("  restore it with: tools/claude-code-config/claude-code-config --force");
    ExitCode::from(EXIT_DRIFT)
}

fn apply(opts: &Opts, target: &Path, mut deployed: Value, baseline: &Value) -> ExitCode {
    let mut touched: Vec<String> = Vec::new();
    if opts.force {
        merge::merge_force(&mut deployed, baseline, "", &mut touched);
    } else {
        merge::merge_defaults(&mut deployed, baseline, "", &mut touched);
    }
    let verb = if opts.force { "restored" } else { "added" };

    if touched.is_empty() {
        println!(
            "claude-code-config: {} already applies Sjel's baseline — no changes.",
            target.display()
        );
        return ExitCode::SUCCESS;
    }
    if opts.dry_run {
        println!(
            "claude-code-config: [dry-run] would have {verb} in {}:",
            target.display()
        );
        for path in &touched {
            println!("  + {path}");
        }
        return ExitCode::SUCCESS;
    }

    // Keep whatever mode the user's own file carries; this merges into a file Claude Code
    // created, and silently tightening or loosening it is not what "add the baseline" means.
    // A file we create ourselves starts at 0600.
    let mode = atomic::existing_mode(target).unwrap_or(0o600);
    let mut serialized = match serde_json::to_string_pretty(&deployed) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("claude-code-config: could not serialize the merged settings: {error}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    serialized.push('\n');

    if let Err(error) = atomic::write_atomic(target, &serialized, mode) {
        eprintln!(
            "claude-code-config: could not write {}: {error}",
            target.display()
        );
        return ExitCode::from(EXIT_USAGE);
    }

    println!("claude-code-config: updated {} — {verb}:", target.display());
    for path in &touched {
        println!("  + {path}");
    }

    // existing-wins means an edited value stays edited, so say when that happened rather than
    // let "updated" read as "the floor is back".
    let remaining = check::drift(&deployed, baseline);
    if !remaining.is_clean() {
        println!(
            "claude-code-config: {} key path(s) still differ from the baseline — `check` reports them, `--force` restores them.",
            remaining.differing.len() + remaining.missing.len()
        );
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn the_overlay_data_directory_is_closed_once_whatever_the_baseline_held() {
        let mut value = serde_json::json!({
            "permissions": {"deny": ["Read(~/.ssh/**)"]},
            "sandbox": {"filesystem": {"denyRead": ["~/.ssh/**"]}}
        });
        let home = Path::new("/Users/someone");
        deny_overlay_data(&mut value, Path::new("/Users/someone/overlay"), Some(home));
        deny_overlay_data(&mut value, Path::new("/Users/someone/overlay"), Some(home));
        assert_eq!(
            value["sandbox"]["filesystem"]["denyRead"],
            serde_json::json!(["~/.ssh/**", "~/overlay/data/**"])
        );
        assert_eq!(
            value["sandbox"]["filesystem"]["denyWrite"],
            serde_json::json!(["~/overlay/data/**"])
        );
        assert_eq!(
            value["permissions"]["deny"],
            serde_json::json!([
                "Read(~/.ssh/**)",
                "Read(~/overlay/data/**)",
                "Edit(~/overlay/data/**)"
            ])
        );
        let mut outside = serde_json::json!({});
        deny_overlay_data(&mut outside, Path::new("/srv/overlay"), Some(home));
        assert_eq!(
            outside["permissions"]["deny"][0],
            "Read(//srv/overlay/data/**)"
        );
        assert_eq!(
            outside["sandbox"]["filesystem"]["denyRead"][0],
            "/srv/overlay/data/**"
        );
    }

    #[test]
    fn apply_is_the_default_verb() {
        let opts = parse(&args(&[])).expect("parse");
        assert_eq!(opts.verb, Verb::Apply);
        assert!(!opts.dry_run && !opts.force);
    }

    #[test]
    fn check_and_the_flags_parse() {
        let opts = parse(&args(&["check"])).expect("parse");
        assert_eq!(opts.verb, Verb::Check);
        let opts = parse(&args(&["apply", "--force", "--dry-run"])).expect("parse");
        assert!(opts.force && opts.dry_run);
    }

    #[test]
    fn an_unknown_argument_is_refused_rather_than_ignored() {
        assert!(parse(&args(&["--managed"])).is_err());
        assert!(parse(&args(&["frobnicate"])).is_err());
    }
}
