//! Persistence: the launchd or systemd unit that brings a capability back after a reboot.
//!
//! `autostart = "true"` gets a watchdog unit (tools/watchdog.sh keeps it up); `schedule = "6h"`
//! gets a periodic unit that runs `service-runner.sh start` once per tick. Declaring both is a
//! contradiction refused by name, not a winner picked. A container that autostarts needs no
//! unit: the runtime restarts it natively (`--restart unless-stopped`).
//!
//! Units render from tools/templates/*.tmpl. `persistence-status` re-renders and compares byte
//! for byte, so a unit edited by hand or rendered from an older declaration reads `stale`, and
//! an installed unit rendered by the bash version reads `installed` only if this renders the
//! same bytes. Rendering keeps sed's semantics for that reason: one substitution per line per
//! placeholder, and `&` in a value stands for the placeholder itself.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::registry::Fail;
use crate::runner::{fail, get_array, launcher, read_table, status_of, which, Svc, R};
use crate::schedule;

const PREFIX: &str = "com.sjel";
const LEGACY_PREFIX: &str = "com.axon";

#[derive(PartialEq, Eq, Clone, Copy)]
enum Mode {
    Watchdog,
    Scheduled,
}

/// `Ok(None)` declares neither. `Err` declares both, with the reason printed when `loud`.
fn mode(s: &Svc, loud: bool) -> Result<Option<Mode>, ()> {
    let auto = s.autostart == "true";
    if auto && !s.schedule.is_empty() {
        if loud {
            eprintln!(
                "service-runner.sh: '{}' declares autostart AND schedule = \"{}\".",
                s.cap, s.schedule
            );
            eprintln!("  A watchdog keeps the process up continuously, so an interval would never have anything to start.");
            eprintln!(
                "  autostart is for a service, schedule is for a periodic job — declare one ({}).",
                s.manifest.display()
            );
        }
        return Err(());
    }
    Ok(if auto {
        Some(Mode::Watchdog)
    } else if !s.schedule.is_empty() {
        Some(Mode::Scheduled)
    } else {
        None
    })
}

/// Whether a unit is owed at all: `Ok(why)` or `Err(why not)`.
fn applicable(s: &Svc) -> Result<String, String> {
    match mode(s, true) {
        Err(()) => Err("contradictory manifest: autostart and schedule cannot both be declared".to_owned()),
        Ok(Some(Mode::Scheduled)) => schedule::seconds(&s.schedule).map(|n| format!("schedule declared — every {n}s")),
        Ok(Some(Mode::Watchdog)) if s.kind == "container" => {
            Err(format!("{} restarts it natively (--restart unless-stopped) — no watchdog needed", s.platform.runtime))
        }
        Ok(Some(Mode::Watchdog)) => Ok("autostart declared".to_owned()),
        Ok(None) => Err("on-demand (neither autostart nor schedule in the manifest) — a watchdog would defeat that".to_owned()),
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn systemd_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("systemd/user")
}

fn unit_path(s: &Svc) -> Result<PathBuf, String> {
    match s.platform.os.as_str() {
        "macos" => Ok(home().join(format!("Library/LaunchAgents/{PREFIX}.{}.plist", s.cap))),
        "linux" => Ok(
            systemd_dir().join(if mode(s, false) == Ok(Some(Mode::Scheduled)) {
                format!("axon-{}.timer", s.cap)
            } else {
                format!("axon-{}.service", s.cap)
            }),
        ),
        "windows" => Err("no windows persistence backend yet".to_owned()),
        os => Err(format!("unknown os '{os}' (machine.toml)")),
    }
}

/// The oneshot a Linux timer activates. Every other combination is one file.
fn companion_path(s: &Svc) -> Option<PathBuf> {
    (s.platform.os == "linux" && mode(s, false) == Ok(Some(Mode::Scheduled)))
        .then(|| systemd_dir().join(format!("axon-{}.service", s.cap)))
}

/// The PATH directories a bare supervisor environment needs: where the runtime, the command,
/// its build tool and each requirement's build tool live. A login shell has them; launchd and
/// systemd --user do not, which is how the launchd-PATH defect stayed invisible for two weeks.
fn path_dirs(s: &mut Svc) -> R<String> {
    let dir = |p: &Path| {
        p.parent()
            .map(|d| d.display().to_string())
            .unwrap_or_default()
    };
    if s.kind == "container" {
        let rt = s.resolve_runtime()?;
        return Ok(dir(&rt));
    }
    let cmd_bin = if s.command[0].starts_with('/') {
        Some(PathBuf::from(&s.command[0]))
    } else {
        which(&s.command[0])
    };
    let mut out = cmd_bin.map(|b| dir(&b)).unwrap_or_default();
    if let Some(b) = s.build.first().and_then(|b| which(b)) {
        out = format!("{out}:{}", dir(&b));
    }
    for dep in get_array(&s.table, "requires") {
        let mf = s
            .paths
            .root
            .join("capabilities")
            .join(&dep)
            .join("service.toml");
        let Ok(t) = read_table(&mf) else { continue };
        let Some(word) = get_array(&t, "build").into_iter().next() else {
            continue;
        };
        if let Some(b) = which(&word) {
            let d = dir(&b);
            if !format!(":{out}:").contains(&format!(":{d}:")) {
                out = format!("{out}:{d}");
            }
        }
    }
    Ok(out)
}

/// Machine-local env from `[capability.<cap>] env = ["K=V", ...]`, rendered for the unit:
/// XML-escaped `<key>`/`<string>` pairs on macOS, `Environment="K=V"` lines on Linux.
///
/// The parsing is `Svc::cap_env`, shared with the on-demand start so the two cannot disagree
/// about what this machine declared.
fn env_block(s: &Svc) -> R<String> {
    let esc = |v: &str| {
        v.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let mut out = String::new();
    for (k, v) in s.cap_env()? {
        match s.platform.os.as_str() {
            "macos" => out.push_str(&format!(
                "    <key>{}</key>\n    <string>{}</string>\n",
                esc(&k),
                esc(&v)
            )),
            "linux" => out.push_str(&format!("Environment=\"{k}={v}\"\n")),
            _ => {}
        }
    }
    Ok(out.strip_suffix('\n').unwrap_or(&out).to_owned())
}

/// sed's `s|placeholder|value|` for one line: the first occurrence only, with `&` in the value
/// standing for the match and `\x` for a literal `x`.
fn sed_sub(line: &str, placeholder: &str, value: &str) -> String {
    let Some(at) = line.find(placeholder) else {
        return line.to_owned();
    };
    let mut repl = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '&' => repl.push_str(placeholder),
            '\\' => repl.extend(chars.next()),
            c => repl.push(c),
        }
    }
    format!("{}{repl}{}", &line[..at], &line[at + placeholder.len()..])
}

/// A template rendered line by line. `drop_empty_after_first` is the macOS schedule template's
/// `/^$/d`, which runs after the first substitution: a dropped launcher line leaves no blank.
/// `__EXTRA_ENV__` lines become the env block, or vanish when there is none.
fn render(
    template: &Path,
    subs: &[(&str, String)],
    drop_empty_after_first: bool,
    env: &str,
) -> R<String> {
    let body = std::fs::read_to_string(template).map_err(|e| Fail {
        code: 1,
        msg: format!("service-runner.sh: cannot read {}: {e}", template.display()),
    })?;
    let mut out = String::new();
    for line in body.lines() {
        let mut l = line.to_owned();
        let mut deleted = false;
        for (i, (ph, v)) in subs.iter().enumerate() {
            l = sed_sub(&l, ph, v);
            if i == 0 && drop_empty_after_first && l.is_empty() {
                deleted = true;
                break;
            }
        }
        if deleted {
            continue;
        }
        if l.contains("__EXTRA_ENV__") {
            if !env.is_empty() {
                out.push_str(env);
                out.push('\n');
            }
            continue;
        }
        out.push_str(&l);
        out.push('\n');
    }
    Ok(out)
}

fn render_unit(s: &mut Svc) -> R<String> {
    let m = match mode(s, true) {
        Ok(Some(m)) => m,
        _ => return fail(1, ""),
    };
    let runtime_dir = path_dirs(s)?;
    let tools = s.tools_dir();
    let watchdog = tools.join("watchdog.sh").display().to_string();
    let runner = launcher(s.paths).display().to_string();
    let env = env_block(s)?;
    let cap = s.cap.clone();
    let label = format!("{PREFIX}.{cap}");
    let (log_out, log_err) = if m == Mode::Scheduled {
        (
            format!("/tmp/axon-{cap}-schedule.log"),
            format!("/tmp/axon-{cap}-schedule.err"),
        )
    } else {
        (
            format!("/tmp/axon-{cap}-watchdog.log"),
            format!("/tmp/axon-{cap}-watchdog.err"),
        )
    };
    let secs = || {
        schedule::seconds(&s.schedule)
            .map(|n| n.to_string())
            .map_err(|e| Fail { code: 1, msg: e })
    };
    let t = |n: &str| tools.join("templates").join(n);
    match (s.platform.os.as_str(), m) {
        ("macos", Mode::Watchdog) => render(
            &t("launchd-watchdog.plist.tmpl"),
            &[
                ("__LABEL__", label),
                ("__WATCHDOG_PATH__", watchdog),
                (
                    "__PATH__",
                    format!("{runtime_dir}:/usr/bin:/bin:/usr/sbin:/sbin"),
                ),
                ("__CAPABILITY__", cap.clone()),
                ("__LOG_OUT__", log_out),
                ("__LOG_ERR__", log_err),
            ],
            false,
            &env,
        ),
        ("macos", Mode::Scheduled) => {
            let secs = secs()?;
            // full_disk_access: the run starts through the signed sjel-fda-launcher, which holds
            // the Full Disk Access grant, instead of as /bin/bash, which has none.
            let mut launcher_line = String::new();
            if s.full_disk_access == "true" {
                let personal = std::env::var("SJEL_PERSONAL_ROOT").unwrap_or_default();
                let fda = PathBuf::from(format!("{personal}/bin/sjel-fda-launcher"));
                if which(&fda.display().to_string()).is_none() {
                    return fail(
                        1,
                        format!(
                            "service-runner.sh: {cap} declares full_disk_access but {} is not installed.\n  Run tools/fda-launcher/install, grant it Full Disk Access, then install-persistence again.",
                            fda.display()
                        ),
                    );
                }
                launcher_line = format!("    <string>{}</string>", fda.display());
            }
            render(
                &t("launchd-schedule.plist.tmpl"),
                &[
                    ("__LAUNCHER__", launcher_line),
                    ("__LABEL__", label),
                    ("__RUNNER_PATH__", runner),
                    ("__INTERVAL_SECONDS__", secs),
                    (
                        "__PATH__",
                        format!("{runtime_dir}:/usr/bin:/bin:/usr/sbin:/sbin"),
                    ),
                    ("__CAPABILITY__", cap.clone()),
                    ("__LOG_OUT__", log_out),
                    ("__LOG_ERR__", log_err),
                ],
                true,
                &env,
            )
        }
        ("linux", Mode::Watchdog) => render(
            &t("systemd-watchdog.service.tmpl"),
            &[
                ("__WATCHDOG_PATH__", watchdog),
                (
                    "__PATH__",
                    format!("{runtime_dir}:/usr/local/bin:/usr/bin:/bin"),
                ),
                ("__CAPABILITY__", cap.clone()),
                ("__LOG_OUT__", log_out),
                ("__LOG_ERR__", log_err),
            ],
            false,
            &env,
        ),
        ("linux", Mode::Scheduled) => {
            let secs = secs()?;
            render(
                &t("systemd-schedule.timer.tmpl"),
                &[
                    ("__CAPABILITY__", cap.clone()),
                    ("__INTERVAL_SECONDS__", secs),
                ],
                false,
                &env,
            )
        }
        _ => fail(1, ""),
    }
}

fn render_companion(s: &mut Svc) -> R<String> {
    let runtime_dir = path_dirs(s)?;
    let env = env_block(s)?;
    let cap = s.cap.clone();
    render(
        &s.tools_dir()
            .join("templates/systemd-schedule.service.tmpl"),
        &[
            ("__RUNNER_PATH__", launcher(s.paths).display().to_string()),
            (
                "__PATH__",
                format!("{runtime_dir}:/usr/local/bin:/usr/bin:/bin"),
            ),
            ("__CAPABILITY__", cap.clone()),
            ("__LOG_OUT__", format!("/tmp/axon-{cap}-schedule.log")),
            ("__LOG_ERR__", format!("/tmp/axon-{cap}-schedule.err")),
        ],
        false,
        &env,
    )
}

/// A render error's message goes to stderr, where the shell renderer wrote it, and the caller
/// reports the state.
fn rendered(r: R<String>) -> Option<String> {
    r.map_err(|f| {
        if !f.msg.is_empty() {
            eprintln!("{}", f.msg);
        }
    })
    .ok()
}

/// `(state, detail)`: misdeclared, n/a, unsupported, missing, stale or installed.
fn state(s: &mut Svc) -> (&'static str, String) {
    if mode(s, false).is_err() {
        return (
            "misdeclared",
            format!(
                "autostart and schedule cannot both be declared ({})",
                s.manifest.display()
            ),
        );
    }
    if let Err(why) = applicable(s) {
        return ("n/a", why);
    }
    let unit = match unit_path(s) {
        Ok(u) => u,
        Err(e) => return ("unsupported", e),
    };
    if !unit.is_file() {
        return ("missing", unit.display().to_string());
    }
    let Some(want) = rendered(render_unit(s)) else {
        return (
            "unsupported",
            format!("cannot render a unit for os {}", s.platform.os),
        );
    };
    if std::fs::read(&unit).ok().as_deref() != Some(want.as_bytes()) {
        return (
            "stale",
            format!(
                "{} no longer matches the declaration — re-run install-persistence",
                unit.display()
            ),
        );
    }
    if let Some(companion) = companion_path(s) {
        if !companion.is_file() {
            return (
                "missing",
                format!(
                    "{} — the timer is installed, the unit it activates is not",
                    companion.display()
                ),
            );
        }
        let Some(want) = rendered(render_companion(s)) else {
            return (
                "unsupported",
                format!(
                    "cannot render the oneshot companion for os {}",
                    s.platform.os
                ),
            );
        };
        if std::fs::read(&companion).ok().as_deref() != Some(want.as_bytes()) {
            return (
                "stale",
                format!(
                    "{} no longer matches the declaration — re-run install-persistence",
                    companion.display()
                ),
            );
        }
    }
    ("installed", unit.display().to_string())
}

/// Is the supervisor running the unit: `yes`, `no`, or `unknown` when it cannot be asked.
fn loaded(s: &Svc) -> &'static str {
    match s.platform.os.as_str() {
        "macos" => {
            if which("launchctl").is_none() {
                return "unknown";
            }
            let Ok(o) = Command::new("launchctl")
                .arg("list")
                .stderr(Stdio::null())
                .output()
            else {
                return "unknown";
            };
            if !o.status.success() {
                return "unknown";
            }
            let label = format!("{PREFIX}.{}", s.cap);
            if String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l.ends_with(&label))
            {
                "yes"
            } else {
                "no"
            }
        }
        "linux" => {
            if which("systemctl").is_none() || !Path::new("/run/systemd/system").is_dir() {
                return "unknown";
            }
            let unit = if mode(s, false) == Ok(Some(Mode::Scheduled)) {
                format!("axon-{}.timer", s.cap)
            } else {
                format!("axon-{}.service", s.cap)
            };
            let active = Command::new("systemctl")
                .args(["--user", "is-active", "--quiet", &unit])
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|st| st.success());
            if active {
                "yes"
            } else {
                "no"
            }
        }
        _ => "unknown",
    }
}

pub fn status(s: &mut Svc) {
    let (st, detail) = state(s);
    let cap = &s.cap;
    if st != "installed" {
        println!("{cap}\t{st}\t{detail}");
        return;
    }
    match loaded(s) {
        "yes" => println!("{cap}\tinstalled\t{detail}"),
        "no" => println!("{cap}\tinstalled-not-loaded\tthe unit exists but the supervisor is not running it: {detail}"),
        _ => println!("{cap}\tinstalled\t{detail} (supervisor could not be asked — load state unverified)"),
    }
}

fn uid() -> String {
    Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn remove_legacy_launchd_unit(cap: &str) {
    let legacy = home().join(format!("Library/LaunchAgents/{LEGACY_PREFIX}.{cap}.plist"));
    if !legacy.is_file() {
        return;
    }
    let _ = Command::new("launchctl")
        .arg("unload")
        .arg(&legacy)
        .stderr(Stdio::null())
        .status();
    let _ = std::fs::remove_file(&legacy);
    println!("removed legacy unit {}", legacy.display());
}

fn write(path: &Path, body: &str) -> R<()> {
    std::fs::write(path, body).map_err(|e| Fail {
        code: 1,
        msg: format!("service-runner.sh: cannot write {}: {e}", path.display()),
    })
}

pub fn install(s: &mut Svc) -> R<u8> {
    if let Err(why) = applicable(s) {
        eprintln!(
            "service-runner.sh: '{}' — {why}. Not installing persistence.",
            s.cap
        );
        if mode(s, false).is_err() {
            return Ok(1);
        }
        // A natively restarting runtime owes nothing, so that is not a failure.
        if s.autostart == "true" {
            return Ok(0);
        }
        eprintln!(
            "  (in {}: autostart = \"true\" if it is meant to always run,",
            s.manifest.display()
        );
        eprintln!("   schedule = \"6h\" if it is meant to run periodically and exit)");
        return Ok(1);
    }
    let unit = match unit_path(s) {
        Ok(u) => u,
        Err(e) => {
            return fail(
                1,
                format!(
                    "service-runner.sh: {e}\nFor now: run '{}/watchdog.sh {}' manually, or add a scheduler entry here.",
                    s.tools_dir().display(),
                    s.cap
                ),
            );
        }
    };
    let os = s.platform.os.clone();
    if os == "linux" && (which("systemctl").is_none() || !Path::new("/run/systemd/system").is_dir())
    {
        return fail(
            1,
            format!(
                "service-runner.sh: systemd not available (no systemctl, or systemd isn't PID 1).\n  On WSL, add 'systemd=true' under [boot] in /etc/wsl.conf, then 'wsl --shutdown' and reopen.\n  Until then run '{}/watchdog.sh {}' manually (e.g. inside tmux/screen).",
                s.tools_dir().display(),
                s.cap
            ),
        );
    }
    if let Some(dir) = unit.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Fail {
            code: 1,
            msg: format!("service-runner.sh: cannot create {}: {e}", dir.display()),
        })?;
    }
    let body = render_unit(s)?;
    write(&unit, &body)?;
    if let Some(companion) = companion_path(s) {
        let body = render_companion(s)?;
        write(&companion, &body)?;
        println!("installed {}", companion.display());
    }
    let cap = s.cap.clone();
    match os.as_str() {
        "macos" => {
            let _ = Command::new("launchctl")
                .arg("unload")
                .arg(&unit)
                .stderr(Stdio::null())
                .status();
            remove_legacy_launchd_unit(&cap);
            let _ = Command::new("launchctl")
                .args(["enable", &format!("gui/{}/{PREFIX}.{cap}", uid())])
                .stderr(Stdio::null())
                .status();
            let code = status_of(Command::new("launchctl").arg("load").arg(&unit));
            if code != 0 {
                return fail(code, "");
            }
            println!("installed {}", unit.display());
        }
        "linux" => {
            let code = status_of(Command::new("systemctl").args(["--user", "daemon-reload"]));
            if code != 0 {
                return fail(code, "");
            }
            let name = unit
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let code =
                status_of(Command::new("systemctl").args(["--user", "enable", "--now", &name]));
            if code != 0 {
                return fail(code, "");
            }
            println!("installed {} (systemctl --user)", unit.display());
            let user = Command::new("id")
                .arg("-un")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                .unwrap_or_default();
            let lingers = Command::new("loginctl")
                .args(["show-user", &user])
                .stderr(Stdio::null())
                .output()
                .is_ok_and(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .any(|l| l.starts_with("Linger=yes"))
                });
            if !lingers {
                eprintln!(
                    "  note: run 'loginctl enable-linger {user}' so this survives logout / reboot."
                );
            }
        }
        _ => {}
    }
    Ok(0)
}

pub fn remove(s: &mut Svc) -> R<u8> {
    let unit = match unit_path(s) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("service-runner.sh: {e} — nothing to remove");
            return Ok(0);
        }
    };
    let companion = companion_path(s);
    if !unit.is_file() && !companion.as_ref().is_some_and(|c| c.is_file()) {
        println!(
            "service-runner.sh: no persistence installed for '{}' ({})",
            s.cap,
            unit.display()
        );
        return Ok(0);
    }
    match s.platform.os.as_str() {
        "macos" => {
            let _ = Command::new("launchctl")
                .arg("unload")
                .arg(&unit)
                .stderr(Stdio::null())
                .status();
            remove_legacy_launchd_unit(&s.cap);
        }
        "linux" => {
            let name = unit
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let _ = Command::new("systemctl")
                .args(["--user", "disable", "--now", &name])
                .stderr(Stdio::null())
                .status();
        }
        _ => {}
    }
    let _ = std::fs::remove_file(&unit);
    println!("removed {}", unit.display());
    if let Some(c) = companion.filter(|c| c.is_file()) {
        let _ = std::fs::remove_file(&c);
        println!("removed {}", c.display());
    }
    if s.platform.os == "linux" {
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .stderr(Stdio::null())
            .status();
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::sed_sub;

    #[test]
    fn substitution_keeps_seds_semantics() {
        assert_eq!(
            sed_sub("a __X__ b __X__", "__X__", "v"),
            "a v b __X__",
            "first occurrence only"
        );
        assert_eq!(
            sed_sub("p=__X__", "__X__", "a&b"),
            "p=a__X__b",
            "& is the match"
        );
        assert_eq!(
            sed_sub("p=__X__", "__X__", "a\\&b"),
            "p=a&b",
            "\\& is a literal &"
        );
        assert_eq!(sed_sub("none", "__X__", "v"), "none");
    }
}
