//! Rust diagnostics (F6, D8): one line per diagnostic instead of the rendered output.
//!
//! A rendered warning is six or seven lines — the source line, a caret under the span, the note
//! text, the help text, and the "for further information about this error" trailer. The same
//! information is in `cargo`'s `--message-format=json`, so this reads that and prints
//! `path:line:col level[code] message`, with a child note indented under it when the compiler
//! has one (AGT-19). Measured smaller than the rendered output on a fixture, which is the only
//! place that number means anything (AGT-20).
//!
//! `cargo deny` is not here: it speaks its own format rather than the compiler's JSON, and D8
//! makes it F6's business "once a `deny.toml` exists". There is none in this repository, so
//! asking for it says so instead of returning output no one can read as diagnostics.

use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use sjel_agent::tools::{run_captured, Ran};
use sjel_agent::{Extension, Tool};

/// The name `agent.toml` and `--ext` use.
pub const NAME: &str = "rust";

/// The subcommands that answer in the compiler's JSON.
const COMMANDS: [&str; 3] = ["check", "clippy", "test"];
/// A workspace `test` build is minutes, not seconds, on this machine.
const DEFAULT_TIMEOUT: u64 = 900;
const MAX_TIMEOUT: u64 = 1800;

pub struct Rust {
    stop: Arc<AtomicBool>,
}

impl Rust {
    pub fn new(stop: &Arc<AtomicBool>) -> Self {
        Self {
            stop: Arc::clone(stop),
        }
    }
}

impl Extension for Rust {
    fn name(&self) -> &'static str {
        NAME
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(Cargo(Arc::clone(&self.stop)))]
    }
}

struct Cargo(Arc<AtomicBool>);

impl Tool for Cargo {
    fn name(&self) -> &'static str {
        "cargo"
    }
    fn description(&self) -> &'static str {
        "Run cargo in this project and return one line per diagnostic \
         (`path:line:col level[code] message`) instead of the rendered compiler output. Prefer \
         this over running cargo through bash: the same information in a fraction of the text."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "required": ["command"], "properties": {
            "command": { "type": "string", "enum": COMMANDS, "description": "The cargo subcommand. `deny` is not here: it has its own format, and this repository has no deny.toml." },
            "manifest_path": { "type": "string", "description": "Path to the Cargo.toml to build. Default: the working directory's." },
            "package": { "type": "string", "description": "Limit the build to one workspace package." },
            "all_targets": { "type": "boolean", "description": "Also check tests, examples and benches." },
            "timeout": { "type": "integer", "description": "Seconds. Default 900, maximum 1800." }
        }})
    }
    fn run(&self, args: &Value) -> Result<String, String> {
        let command = str_arg(args, "command")?;
        if command == "deny" {
            return Err(
                "`deny` is not offered: cargo-deny's output is not the compiler's JSON, and D8 \
                 makes it this tool's business only once a deny.toml exists — there is none in \
                 this repository"
                    .to_owned(),
            );
        }
        if !COMMANDS.contains(&command) {
            return Err(format!("`{command}` is not one of {}", COMMANDS.join(", ")));
        }
        let timeout = args
            .get("timeout")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT)
            .clamp(1, MAX_TIMEOUT);

        let mut cargo = Command::new("cargo");
        cargo.arg(command).arg("--message-format=json");
        if let Some(path) = args
            .get("manifest_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            cargo.arg("--manifest-path").arg(path);
        }
        if let Some(package) = args
            .get("package")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|package| !package.is_empty())
        {
            cargo.arg("--package").arg(package);
        }
        if args
            .get("all_targets")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            cargo.arg("--all-targets");
        }
        let ran = run_captured(cargo, Duration::from_secs(timeout), &self.0)?;
        Ok(report(&ran))
    }
    // Not read-only: cargo writes to its target directory. So a turn whose only call is this one
    // runs it on its own, which is what a build wants anyway.
}

/// cargo's diagnostics, one line each, then anything cargo itself said on stderr.
fn report(ran: &Ran) -> String {
    let mut diagnostics: Vec<String> = Vec::new();
    let mut hidden: Vec<String> = Vec::new();
    for line in ran.stdout.lines() {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            // A line the reader cannot parse is not a diagnostic: `cargo` also emits
            // `compiler-artifact`, `build-script-executed` and `build-finished` here.
            continue;
        };
        if message["reason"] != "compiler-message" {
            continue;
        }
        match diagnostic(&message["message"]) {
            Some((line, children)) => {
                diagnostics.push(line);
                hidden.extend(children);
            }
            // A `compiler-message` with no text at all: nothing to show, and nothing lost that
            // the model could act on.
            None => continue,
        }
    }
    // The same crate built for several targets reports the same diagnostic once per target.
    let before = diagnostics.len();
    let mut seen = std::collections::BTreeSet::new();
    diagnostics.retain(|line| seen.insert(line.clone()));
    let repeats = before - diagnostics.len();

    let counted = match diagnostics.len() {
        0 => "no diagnostics".to_owned(),
        1 => "1 diagnostic".to_owned(),
        n => format!("{n} diagnostics"),
    };
    let mut out = format!("{} — {counted}", ran.summary());
    if repeats > 0 {
        out.push_str(&format!(" ({repeats} repeated line(s) dropped)"));
    }
    for line in diagnostics.iter().chain(hidden.iter()) {
        out.push('\n');
        out.push_str(line);
    }
    let stderr = ran.stderr.trim_end();
    if !stderr.is_empty() {
        // cargo's own failures (a manifest it cannot read, a missing toolchain) arrive here as
        // prose, and they are the whole answer when they do.
        out.push_str("\n--- stderr\n");
        out.push_str(stderr);
    }
    out
}

/// One `compiler-message` as a line, plus its child notes indented under it.
fn diagnostic(message: &Value) -> Option<(String, Vec<String>)> {
    let level = message["level"].as_str()?;
    let text = message["message"].as_str()?;
    if text.is_empty() {
        return None;
    }
    let code = message["code"]["code"]
        .as_str()
        .map_or(String::new(), |code| format!("[{code}]"));
    let primary = message["spans"]
        .as_array()
        .and_then(|spans| spans.iter().find(|span| span["is_primary"] == true));
    let place = primary.map_or_else(
        || "-".to_owned(),
        |span| {
            format!(
                "{}:{}:{}",
                span["file_name"].as_str().unwrap_or("-"),
                span["line_start"].as_u64().unwrap_or(0),
                span["column_start"].as_u64().unwrap_or(0)
            )
        },
    );
    // The span's label is where rustc puts the part a model needs most: `expected `i32`, found
    // `&str`` is a label, not the message, and a tool that drops it makes the model read the
    // file to learn what the compiler already said.
    let label = primary
        .and_then(|span| span["label"].as_str())
        .filter(|label| !label.is_empty())
        .map_or(String::new(), |label| format!(" — {label}"));
    let line = format!("{place} {level}{code} {text}{label}");
    // A child that repeats the label is the same words twice: rustc says "prefix it with an
    // underscore" both as the help child and as the span's label.
    let children: Vec<String> = message["children"]
        .as_array()
        .map(|children| {
            children
                .iter()
                .filter_map(|child| {
                    let text = child["message"].as_str()?;
                    let level = child["level"].as_str()?;
                    if text.is_empty() || line.contains(text) {
                        return None;
                    }
                    Some(format!("  {level}: {text}"))
                })
                .collect()
        })
        .unwrap_or_default();
    Some((line, children))
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("`{key}` is required"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A crate with one error and one warning, in a directory of its own.
    ///
    /// Each fixture is its own package *name*, not just its own directory: every one of them
    /// builds into the same target dir, and two packages with one name overwrite each other's
    /// artifacts there — which shows up as a build that reports another fixture's diagnostics,
    /// or none at all.
    fn fixture(name: &str, source: &str) -> String {
        let dir =
            std::env::temp_dir().join(format!("sjel-agent-cargo-{}-{name}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"fixture-{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("src/lib.rs"), source).unwrap();
        dir.join("Cargo.toml").to_string_lossy().into_owned()
    }

    const WITH_ERROR_AND_WARNING: &str = "pub fn add(a: i32, b: i32) -> i32 {\n    let unused = 1;\n    a + b\n}\n\npub fn broken() -> i32 {\n    \"not a number\"\n}\n";
    const CLEAN: &str = "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";

    /// Ten warnings, to measure the ratio where it matters: the rendered form spends six lines
    /// per diagnostic and this spends one, so the saving grows with the count while the framing
    /// around it does not.
    fn many_warnings() -> String {
        let mut source = String::new();
        for n in 0..10 {
            source.push_str(&format!(
                "pub fn f{n}() -> i32 {{\n    let unused{n} = {n};\n    {n}\n}}\n\n"
            ));
        }
        source
    }

    fn tool() -> Cargo {
        Cargo(Arc::new(AtomicBool::new(false)))
    }

    fn check(manifest: &str) -> String {
        tool()
            .run(&json!({ "command": "check", "manifest_path": manifest }))
            .expect("the tool ran cargo")
    }

    /// Every diagnostic cargo put in the JSON, as its own reading of the same output.
    fn diagnostics_in_json(manifest: &str) -> Vec<String> {
        let mut cargo = Command::new("cargo");
        cargo
            .args(["check", "--message-format=json"])
            .arg("--manifest-path")
            .arg(manifest);
        let ran = run_captured(cargo, Duration::from_secs(300), &AtomicBool::new(false))
            .expect("cargo ran");
        ran.stdout
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|message| message["reason"] == "compiler-message")
            .filter_map(|message| message["message"]["message"].as_str().map(str::to_owned))
            .collect()
    }

    #[test]
    fn every_diagnostic_cargo_reported_is_a_line() {
        let manifest = fixture("both", WITH_ERROR_AND_WARNING);
        let out = check(&manifest);
        let reported = diagnostics_in_json(&manifest);
        assert!(
            reported.len() >= 2,
            "the fixture stopped producing two diagnostics: {reported:?}\n{out}"
        );
        for text in &reported {
            assert!(
                out.contains(text.as_str()),
                "cargo reported `{text}` and the tool result does not have it:\n{out}"
            );
        }
        assert!(out.contains("error[E0308]"), "{out}");
        assert!(
            out.contains("expected `i32`, found `&str`"),
            "the span label is the part that says what to change:\n{out}"
        );
        assert!(out.contains("warning[unused_variables]"), "{out}");
        // The place is in the shape the model is told to expect.
        assert!(out.contains("src/lib.rs:"), "{out}");
        assert!(out.contains("exit code 101"), "{out}");
    }

    #[test]
    fn the_result_is_smaller_than_the_rendered_output() {
        for (name, source) in [
            ("smaller", WITH_ERROR_AND_WARNING.to_owned()),
            ("warnings", many_warnings()),
        ] {
            let manifest = fixture(name, &source);
            let mut cargo = Command::new("cargo");
            cargo.args(["check"]).arg("--manifest-path").arg(&manifest);
            let rendered = run_captured(cargo, Duration::from_secs(300), &AtomicBool::new(false))
                .expect("cargo ran")
                .reported();
            let out = check(&manifest);
            // Printed for the record: the numbers in ISA.md are these, not an assumption.
            println!(
                "{name}: tool {} bytes, rendered {} bytes",
                out.len(),
                rendered.len()
            );
            assert!(
                out.len() < rendered.len(),
                "{name}: the tool result is {} bytes and the rendered output {} — measured, not assumed",
                out.len(),
                rendered.len()
            );
        }
    }

    #[test]
    fn a_clean_build_says_so() {
        let manifest = fixture("clean", CLEAN);
        let out = check(&manifest);
        assert!(out.contains("exit code 0"), "{out}");
        assert!(out.contains("no diagnostics"), "{out}");
    }

    #[test]
    fn deny_says_why_it_is_not_here() {
        let err = tool()
            .run(&json!({ "command": "deny" }))
            .expect_err("deny was accepted");
        assert!(err.contains("deny.toml"), "{err}");
        let err = tool()
            .run(&json!({ "command": "build" }))
            .expect_err("an unknown subcommand was accepted");
        assert!(err.contains("check, clippy, test"), "{err}");
    }

    #[test]
    fn a_child_note_is_kept_under_its_diagnostic() {
        // The notes and helps under a warning are what a model needs to act on it — here, both
        // the reason the lint is on and the way to keep the binding.
        let manifest = fixture("child", WITH_ERROR_AND_WARNING);
        let out = check(&manifest);
        assert!(
            out.contains("\n  note: `#[warn(unused_variables)]`"),
            "{out}"
        );
        // A child that repeats the span label is dropped, so the same words arrive once. This
        // lint's help is a child and its label is empty; E0308's other way round, which the
        // test above holds.
        assert_eq!(
            out.matches("prefix it with an underscore").count(),
            1,
            "the same words arrived twice:\n{out}"
        );
    }
}
