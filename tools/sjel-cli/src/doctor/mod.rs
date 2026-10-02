//! `tools/doctor` — health checks for an already-set-up Sjel machine.
//!
//! Ported from tools/doctor.ts on 2026-10-02 with the user's approval; until then
//! CONTRIBUTING.md#cargo-and-bun-are-the-build-path held that doctor stays interpreted. The
//! section order, every message and the exit code are the TypeScript version's, compared
//! line by line on this Mac before the launcher switched.
//!
//! Each rule stays with the tool that owns it: the host toolchain with toolchain-check, boot
//! persistence with service-runner.sh, Pack deployment with tools/harnesses.ts (through the
//! tools/doctor-packs.ts sidecar), local inference roles with tools/model-check.ts. Doctor reports.
//!
//!   tools/doctor            full report, offline (no GitHub calls)
//!   tools/doctor --online   also probe declared systems and fetch origin/main
//!   tools/doctor --version  installed vs origin/main version identity only (read-only, exits 0)
//!
//! Exit 0 = all checks pass, 1 = one or more failed.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

mod checks;
mod overlay;
pub mod pure;

const HELP: &str = "tools/doctor — health checks for an already-set-up Axon machine.

  tools/doctor            full report, offline (no GitHub calls)
  tools/doctor --online   also probe declared systems and fetch origin/main
  tools/doctor --version  installed vs origin/main version identity only
                          (read-only, exits 0; add --online for a live fetch first)
  tools/doctor -h         this help
";

/// What every check reads and writes, as doctor.ts's CheckContext held it. Output is
/// buffered per section, so independent sections can run at once and still print in order.
#[derive(Clone)]
pub struct Ctx {
    pub root: PathBuf,
    pub overlay_path: String,
    /// machine.toml; an empty table until "Machine identity" reads it.
    pub machine: toml::Table,
    /// Its text, for the sections that iterate `[capability.*]` in file order.
    pub machine_src: String,
    /// `[[state_mount]]` entries: (tool, path).
    pub mounts: Vec<(String, String)>,
    /// systems.toml, in file order.
    pub systems: Vec<(String, toml::Value)>,
    pub online: bool,
    pub failed: u32,
    pub out: String,
}

impl Ctx {
    pub fn ok(&mut self, msg: impl AsRef<str>) {
        self.line(format!("  ✓ {}", msg.as_ref()));
    }
    pub fn bad(&mut self, msg: impl AsRef<str>) {
        self.line(format!("  ✗ {}", msg.as_ref()));
        self.failed += 1;
    }
    pub fn warn(&mut self, msg: impl AsRef<str>) {
        self.line(format!("  ⚠ {}", msg.as_ref()));
    }
    /// A line of output that is not a verdict.
    pub fn line(&mut self, s: impl AsRef<str>) {
        self.out.push_str(s.as_ref());
        self.out.push('\n');
    }

    pub fn enabled(&self) -> Vec<String> {
        self.machine
            .get("capabilities")
            .and_then(toml::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn machine_str(&self, key: &str) -> Option<String> {
        self.machine
            .get(key)
            .map(js_value)
            .filter(|s| !s.is_empty())
    }
}

pub fn run(args: &[String]) -> ExitCode {
    let has = |f: &str| args.iter().any(|a| a == f);
    if has("-h") || has("--help") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let root = match std::env::var("SJEL_ROOT") {
        Ok(r) if !r.is_empty() => PathBuf::from(r),
        _ => {
            eprintln!("doctor: SJEL_ROOT is unset — run tools/doctor, which sets it");
            return ExitCode::from(2);
        }
    };
    if has("--version") {
        checks::print_version(&root, has("--online"));
        return ExitCode::SUCCESS;
    }
    let mut ctx = Ctx {
        root: root.clone(),
        overlay_path: String::new(),
        machine: toml::Table::new(),
        machine_src: String::new(),
        mounts: Vec::new(),
        systems: Vec::new(),
        online: has("--online"),
        failed: 0,
        out: String::new(),
    };
    println!("Axon doctor · {}", root.display());
    checks::run_all(&mut ctx);
    println!();
    if ctx.failed == 0 {
        println!("doctor: all checks passed");
        ExitCode::SUCCESS
    } else {
        println!("doctor: {} check(s) failed", ctx.failed);
        ExitCode::from(1)
    }
}

// ---- process and file helpers -------------------------------------------------------------

pub struct Out {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Run to completion with both streams captured, as `Bun.spawnSync({stdout: "pipe", stderr:
/// "pipe"})` did. A program that cannot start reads as no exit code and empty output.
pub fn capture(c: &mut Command) -> Out {
    match c.stdin(Stdio::null()).output() {
        Ok(o) => Out {
            code: o.status.code(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        },
        Err(_) => Out {
            code: None,
            stdout: String::new(),
            stderr: String::new(),
        },
    }
}

pub fn cmd(program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Out {
    capture(Command::new(program).args(args))
}

/// `git -C <root> ...`, trimmed stdout or empty on failure, as doctor.ts's gitOut.
pub fn git(root: &Path, args: &[&str]) -> String {
    let o = capture(Command::new("git").arg("-C").arg(root).args(args));
    if o.success() {
        o.stdout.trim().to_owned()
    } else {
        String::new()
    }
}

pub fn read_toml(path: &Path) -> Result<toml::Table, String> {
    let body = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    body.parse::<toml::Table>().map_err(|e| e.to_string())
}

/// The top-level keys of a TOML file in file order. Bun.TOML kept insertion order and the
/// report iterates it; `toml` is built without `preserve_order` workspace-wide, so the order
/// comes from each value's span.
pub fn read_toml_ordered(path: &Path) -> Result<Vec<(String, toml::Value)>, String> {
    let body = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    ordered(&body)
}

pub fn ordered(body: &str) -> Result<Vec<(String, toml::Value)>, String> {
    let map: std::collections::BTreeMap<String, toml::Spanned<toml::Value>> =
        toml::from_str(body).map_err(|e| e.to_string())?;
    let mut v: Vec<(usize, String, toml::Value)> = map
        .into_iter()
        .map(|(k, s)| (s.span().start, k, s.into_inner()))
        .collect();
    v.sort_by_key(|(start, _, _)| *start);
    Ok(v.into_iter().map(|(_, k, v)| (k, v)).collect())
}

pub fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// `~` at the front becomes $HOME, as doctor.ts's expandHome (which did not require a slash).
pub fn expand_home(p: &str) -> String {
    match p.strip_prefix('~') {
        Some(rest) => format!("{}{rest}", home()),
        None => p.to_owned(),
    }
}

pub fn exists(p: impl AsRef<Path>) -> bool {
    p.as_ref().exists()
}

pub fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn mtime_secs(p: impl AsRef<Path>) -> Option<f64> {
    let m = std::fs::metadata(p).ok()?.modified().ok()?;
    m.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs_f64())
}

// ---- JavaScript number and value formatting -----------------------------------------------
//
// Messages interpolate numbers the way JavaScript prints them, and a parity diff sees every
// difference: `${48}` is "48" not "48.0", `x.toFixed(1)` rounds an exact tie up where Rust's
// `{:.1}` rounds it to even (1.25 → "1.3" against "1.2").

/// `Number.prototype.toFixed(digits)` for finite, non-negative magnitudes this report prints.
pub fn to_fixed(x: f64, digits: usize) -> String {
    let exact = format!("{:.60}", x.abs());
    let (_, frac) = exact.split_once('.').unwrap_or((&exact, ""));
    let tail = frac.get(digits..).unwrap_or("");
    let is_tie = tail.starts_with('5') && tail[1..].bytes().all(|b| b == b'0');
    let scale = 10f64.powi(i32::try_from(digits).unwrap_or(0));
    let v = if is_tie {
        (x.abs() * scale).floor() / scale + 1.0 / scale
    } else {
        x.abs()
    };
    let s = format!("{v:.digits$}");
    if x < 0.0 && s.bytes().any(|b| (b'1'..=b'9').contains(&b)) {
        format!("-{s}")
    } else {
        s
    }
}

/// `Math.round`: halves round toward +∞.
pub fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// `String(n)` for a number.
pub fn js_num(x: f64) -> String {
    if x.is_nan() {
        "NaN".to_owned()
    } else if x.is_infinite() {
        if x > 0.0 {
            "Infinity".to_owned()
        } else {
            "-Infinity".to_owned()
        }
    } else if x.fract() == 0.0 && x.abs() < 1e21 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

/// `Number(v)` for a TOML value: numbers as themselves, strings parsed (blank is 0, junk NaN),
/// booleans 1 or 0, anything else NaN.
pub fn js_number(v: Option<&toml::Value>) -> f64 {
    match v {
        Some(toml::Value::Integer(i)) => *i as f64,
        Some(toml::Value::Float(f)) => *f,
        Some(toml::Value::Boolean(b)) => f64::from(u8::from(*b)),
        Some(toml::Value::String(s)) => js_number_str(s),
        _ => f64::NAN,
    }
}

pub fn js_number_str(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        0.0
    } else {
        t.parse::<f64>()
            .ok()
            .filter(|f| f.is_finite())
            .unwrap_or(f64::NAN)
    }
}

/// `${v}` for a TOML value: strings bare, numbers as JavaScript prints them.
pub fn js_value(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => js_num(*f),
        toml::Value::Boolean(b) => b.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_fixed_rounds_ties_up_like_javascript() {
        assert_eq!(to_fixed(1.25, 1), "1.3");
        assert_eq!(to_fixed(0.25, 1), "0.3");
        assert_eq!(to_fixed(6.0, 1), "6.0");
        assert_eq!(to_fixed(3.04, 1), "3.0");
        assert_eq!(to_fixed(4.0, 1), "4.0");
        assert_eq!(to_fixed(1.0 / 3.0, 1), "0.3");
        assert_eq!(to_fixed(2.35, 1), "2.4"); // 2.35 is 2.35000000000000008882 in binary
        assert_eq!(to_fixed(1.45, 1), "1.4"); // 1.45 is 1.44999999999999995559 in binary
    }

    #[test]
    fn numbers_print_as_javascript_prints_them() {
        assert_eq!(js_num(48.0), "48");
        assert_eq!(js_num(1.5), "1.5");
        assert_eq!(js_num(f64::NAN), "NaN");
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert!(js_number_str("soon").is_nan());
        assert_eq!(js_number_str(""), 0.0);
    }
}
