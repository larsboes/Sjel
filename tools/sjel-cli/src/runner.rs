//! `tools/service-runner.sh` — the shared service-manifest interpreter.
//!
//! Ported from bash on 2026-10-02. The script path stays as the launcher: the launchd and
//! systemd units this installs, tools/watchdog.sh, backup.sh, container-refresh.sh, doctor and
//! sjel-status all invoke it by that path.
//!
//! Capabilities declare WHAT they need; this is the only place that knows HOW to satisfy it on
//! this machine. `kind = "container"` (the default) goes to the container runtime,
//! `kind = "process"` execs a host process, and `kind = "data"` runs nothing: it is a file whose
//! manifest exists so tools/backup.sh has one owner to read a contract from. Every lifecycle
//! verb refuses it by name, and the whole-machine fan-out skips it.
//!
//! External programs stay external where the shell called them: the container runtime,
//! launchctl, systemctl, curl (health), pgrep and kill (process trees), nohup (detaching), and
//! this launcher itself for the per-capability fan-out, so a fanned-out verb runs exactly as a
//! hand-typed one does.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, SystemTime};

use crate::paths::{duplicate_message, Manifest, Paths};
use crate::persist;
use crate::registry::{self, Fail};
use crate::runargs;

const USAGE: &str = "\
usage: service-runner.sh <start|stop|idle-stop|restart|resume|recreate|status> <capability>
       service-runner.sh <install-persistence|remove-persistence|persistence-status> <capability>
       service-runner.sh persistence                     # persistence state for the whole enabled set
       service-runner.sh recreate <capability>          # rebuild the container from the current declaration
       service-runner.sh stop <capability> [--no-hold]   # --no-hold: do not keep it down
       service-runner.sh up [--all]     # start the autostart set (--all: everything enabled)
       service-runner.sh down           # stop everything enabled, dependents first; no hold
       service-runner.sh status         # one line per enabled service";

/// The hold expires: it sits in boot-cleared /tmp, so a crashed holder can never leave a
/// password manager permanently un-restartable.
const MAINT_MAX_AGE: u64 = 1800;

pub type R<T> = Result<T, Fail>;

pub fn fail<T>(code: u8, msg: impl Into<String>) -> R<T> {
    Err(Fail {
        code,
        msg: msg.into(),
    })
}

fn usage() -> Fail {
    Fail {
        code: 1,
        msg: USAGE.to_owned(),
    }
}

pub fn run(args: &[String]) -> ExitCode {
    match main(args) {
        Ok(code) => ExitCode::from(code),
        Err(f) => {
            if !f.msg.is_empty() {
                eprintln!("{}", f.msg);
            }
            ExitCode::from(f.code)
        }
    }
}

fn main(args: &[String]) -> R<u8> {
    let arg = |i: usize| args.get(i).map_or("", String::as_str);
    let (cmd, cap, flag) = (arg(0), arg(1), arg(2));
    if !flag.is_empty() && flag != "--no-hold" {
        eprintln!("service-runner.sh: unknown flag '{flag}'");
        return Err(usage());
    }
    if cmd.is_empty() {
        return Err(usage());
    }
    let paths = Paths::from_env().map_err(|m| Fail {
        code: 1,
        msg: format!("service-runner.sh: {m}"),
    })?;
    let platform = Platform::load(&paths)?;

    // No capability means "this whole machine". `down` passes --no-hold, so `down` then `up` is
    // not a dead end: holding there made every following `up` answer "held for maintenance".
    match cmd {
        "up" => {
            if !cap.is_empty() && cap != "--all" {
                return Err(usage());
            }
            return Ok(fan_out(&paths, "start", cap == "--all", None));
        }
        "down" => {
            if !cap.is_empty() {
                return Err(usage());
            }
            return Ok(fan_out(&paths, "stop", false, Some("--no-hold")));
        }
        "status" if cap.is_empty() => return Ok(fan_out(&paths, "status", false, None)),
        // "Which capabilities will not come back after a reboot" is a question about the
        // machine, not about one capability (#9).
        "persistence" => {
            if !cap.is_empty() {
                return Err(usage());
            }
            return Ok(fan_out(&paths, "persistence-status", false, None));
        }
        _ => {}
    }
    if cap.is_empty() {
        return Err(usage());
    }
    let mut svc = Svc::load(&paths, platform, cap, flag == "--no-hold")?;
    svc.dispatch(cmd)
}

/// `os` and `container_runtime` from machine.toml, which tools/lib/platform.sh exported to every
/// child; they are exported here too, because started processes and the units read them.
pub struct Platform {
    pub os: String,
    pub runtime: String,
    pub machine: PathBuf,
    pub machine_table: toml::Table,
}

impl Platform {
    fn load(paths: &Paths) -> R<Self> {
        if std::env::var("SJEL_PERSONAL_ROOT")
            .unwrap_or_default()
            .is_empty()
        {
            return fail(1, "platform.sh: source tools/lib/paths.sh first");
        }
        let machine = paths.machine_toml.clone().unwrap_or_default();
        if !machine.is_file() {
            let shown = if machine.as_os_str().is_empty() {
                "<unresolved>".to_owned()
            } else {
                machine.display().to_string()
            };
            return fail(
                1,
                format!("platform.sh: missing {shown} — copy schemas/machine.toml.example into the overlay, or name this machine in axon.local.toml"),
            );
        }
        let machine_table = read_table(&machine)?;
        let os = get_str(&machine_table, "os");
        let runtime = get_str(&machine_table, "container_runtime");
        // Edition 2021, single-threaded here: the exports platform.sh made, for every child.
        std::env::set_var("SJEL_OS", &os);
        std::env::set_var("SJEL_CONTAINER_RUNTIME", &runtime);
        if os.is_empty() || runtime.is_empty() {
            return fail(
                1,
                format!(
                    "platform.sh: {} missing 'os' or 'container_runtime'",
                    machine.display()
                ),
            );
        }
        Ok(Self {
            os,
            runtime,
            machine,
            machine_table,
        })
    }

    /// `[capability.<cap>]` in machine.toml: per-machine overrides (port, ports, env,
    /// provided_by).
    pub fn cap_section(&self, cap: &str) -> Option<&toml::Table> {
        self.machine_table
            .get("capability")?
            .as_table()?
            .get(cap)?
            .as_table()
    }
}

/// Every enabled row, in the registry's dependency-first order, run as `<launcher> <op> <name>`.
/// `stop` walks it backwards so nothing stops while something that requires it is still up.
/// One capability refusing must not hide the state of the rest, so the walk completes and the
/// status is 1 if any failed.
fn fan_out(paths: &Paths, op: &str, all: bool, extra: Option<&str>) -> u8 {
    let rows = registry::runner_rows(paths).unwrap_or_else(|f| {
        if !f.msg.is_empty() {
            eprintln!("{}", f.msg);
        }
        Vec::new()
    });
    let mut names: Vec<String> = rows
        .iter()
        // kind=data is a file, not a process: a whole-machine verb reporting a failure for a
        // row that can never have one would make its exit status stop meaning anything.
        .filter(|s| s.kind != "data")
        .filter(|s| op != "start" || all || s.field("autostart") == "true")
        .map(|s| s.name.clone())
        .collect();
    if names.is_empty() {
        println!("nothing to do (no matching services in the registry)");
        return 0;
    }
    if op == "stop" {
        names.reverse();
    }
    let launcher = launcher(paths);
    let mut rc = 0;
    for name in names {
        let ok = Command::new(&launcher)
            .args([op, &name])
            .args(extra)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            rc = 1;
        }
    }
    rc
}

pub fn launcher(paths: &Paths) -> PathBuf {
    paths.root.join("tools/service-runner.sh")
}

/// One capability, resolved: its manifest and everything read from it.
pub struct Svc<'a> {
    pub paths: &'a Paths,
    pub platform: Platform,
    pub cap: String,
    pub manifest: PathBuf,
    pub table: toml::Table,
    /// The root relative manifest paths resolve against: the overlay for an overlay capability.
    pub cap_root: PathBuf,
    pub name: String,
    pub kind: String,
    pub autostart: String,
    pub schedule: String,
    pub full_disk_access: String,
    no_hold: bool,
    lock: PathBuf,
    pid_file: PathBuf,
    log: PathBuf,
    err: PathBuf,
    // kind = "process"
    pub command: Vec<String>,
    pub build: Vec<String>,
    panel_build: Vec<String>,
    build_output: String,
    sign: String,
    workdir: String,
    port: String,
    health_path: String,
    // kind = "container", filled by `prepare`
    runtime_bin: String,
    pub runtime_path: Option<PathBuf>,
    image: String,
    tag: String,
    container_args: Vec<String>,
    prepared: bool,
}

impl<'a> Svc<'a> {
    fn load(paths: &'a Paths, platform: Platform, cap: &str, no_hold: bool) -> R<Self> {
        // paths.sh owns the resolution order and the declared-twice refusal; this stays the
        // only place that turns a name into a manifest.
        let manifest = match paths.manifest(cap) {
            Manifest::Found(p) => p,
            Manifest::Duplicate(c, o) => return fail(2, duplicate_message(cap, &c, &o)),
            Manifest::None => {
                let overlay_caps = paths.overlay_caps_dir.clone().unwrap_or_default();
                return fail(
                    1,
                    format!(
                        "service-runner.sh: no service.toml for '{cap}' (looked in {}/{cap}/, {}/{cap}/ and {}/{cap}/)",
                        paths.caps_dir.display(),
                        overlay_caps.display(),
                        paths.root.display()
                    ),
                );
            }
        };
        // A capability this machine CONSUMES from another deployment has a manifest here but no
        // process here to act on (retired-tracker#169).
        if platform
            .cap_section(cap)
            .is_some_and(|t| !get_str(t, "provided_by").is_empty())
        {
            return fail(
                1,
                format!(
                    "service-runner.sh: '{cap}' is provided by another deployment — [capability.{cap}] provided_by in {}.\n  This machine may read its health; its lifecycle belongs to whoever owns its host.",
                    platform.machine.display()
                ),
            );
        }
        let cap_root = match &paths.overlay_caps_dir {
            Some(o) if manifest.starts_with(o) => paths.overlay_root.clone().unwrap_or_default(),
            _ => paths.root.clone(),
        };
        let table = read_table(&manifest).unwrap_or_default();
        let g = |k: &str| get_str(&table, k);
        let kind = Some(g("kind"))
            .filter(|k| !k.is_empty())
            .unwrap_or_else(|| "container".to_owned());

        // The one local model runtime this machine has, read by libs/inference.
        if let Some(b) = platform
            .machine_table
            .get("inference")
            .and_then(toml::Value::as_table)
            .map(|t| get_str(t, "backend"))
            .filter(|b| !b.is_empty())
        {
            std::env::set_var("SJEL_INFERENCE_BACKEND", b);
        }
        // Which hosts this operator owns (PRD Q39): trusted by libs/inference like loopback.
        let peers = trusted_peers(paths);
        if !peers.is_empty() {
            std::env::set_var("SJEL_INFERENCE_TRUSTED_PEERS", peers);
        }

        Ok(Self {
            lock: PathBuf::from(format!("/tmp/axon-{cap}.maintenance")),
            pid_file: PathBuf::from(format!("/tmp/axon-{cap}.pid")),
            log: PathBuf::from(format!("/tmp/axon-{cap}.log")),
            err: PathBuf::from(format!("/tmp/axon-{cap}.err")),
            name: g("name"),
            autostart: g("autostart"),
            schedule: g("schedule"),
            full_disk_access: g("full_disk_access"),
            kind,
            paths,
            platform,
            cap: cap.to_owned(),
            manifest,
            table,
            cap_root,
            no_hold,
            command: Vec::new(),
            build: Vec::new(),
            panel_build: Vec::new(),
            build_output: String::new(),
            sign: String::new(),
            workdir: String::new(),
            port: String::new(),
            health_path: String::new(),
            runtime_bin: String::new(),
            runtime_path: None,
            image: String::new(),
            tag: String::new(),
            container_args: Vec::new(),
            prepared: false,
        })
    }

    pub fn tools_dir(&self) -> PathBuf {
        self.paths.root.join("tools")
    }

    fn dispatch(&mut self, cmd: &str) -> R<u8> {
        if self.kind == "data" {
            if cmd == "persistence-status" {
                println!(
                    "{}\tn/a\tkind=data — a file, not a process: nothing to supervise",
                    self.cap
                );
                return Ok(0);
            }
            return fail(
                1,
                format!(
                    "service-runner.sh: '{}' is kind=data — it declares a file and how it is backed up, not something to {cmd}.\n  The capabilities that read it are the processes; this manifest exists for tools/backup.sh.",
                    self.cap
                ),
            );
        }
        let process = self.kind == "process";
        if process {
            self.process_init()?;
        }
        match cmd {
            "start" if process => self.start_process(),
            "start" => self.start_service(),
            "stop" => {
                let hold = !self.no_hold;
                if process { self.stop_process(hold) } else { self.stop_service(hold) }
            }
            "restart" if process => {
                if self.stop_process(false)? != 0 {
                    return Ok(1);
                }
                self.maybe_build(true)?;
                self.start_process()
            }
            "restart" => {
                self.stop_service(false)?;
                self.start_service()
            }
            "idle-stop" if process => self.stop_process(false),
            "idle-stop" => self.stop_service(false),
            "resume" if process => {
                let _ = std::fs::remove_file(&self.lock);
                self.start_process()
            }
            "resume" => {
                let _ = std::fs::remove_file(&self.lock);
                self.start_service()
            }
            "status" if process => {
                self.status_process();
                Ok(0)
            }
            "status" => self.status_service(),
            "recreate" if process => fail(1, "service-runner.sh: 'recreate' is for container capabilities; use restart for a process"),
            "recreate" => self.recreate_service(),
            "install-persistence" => persist::install(self),
            "remove-persistence" => persist::remove(self),
            "persistence-status" => {
                persist::status(self);
                Ok(0)
            }
            _ => Err(usage()),
        }
    }

    // ---- maintenance hold -------------------------------------------------------------------
    //
    // `stop` takes a capability down AND keeps it down, so a tool can work on its data while
    // nothing has it open (tools/backup.sh's cold SQLite copy); `resume` lifts it. Hardcoded
    // /tmp, not $TMPDIR: the holder runs in a login shell and the watchdog under launchd, and
    // the two must agree on one path or the hold protects nothing.

    fn hold_active(&self, quiet: bool) -> bool {
        let Ok(meta) = std::fs::metadata(&self.lock) else {
            return false;
        };
        // A hold that cannot be dated is not one that has expired: keep it and say so.
        let Some(age) = meta
            .modified()
            .ok()
            .and_then(|m| m.elapsed().ok())
            .map(|d| d.as_secs())
        else {
            if !quiet {
                eprintln!(
                    "service-runner.sh: cannot read the age of '{}' — treating the hold as active",
                    self.lock.display()
                );
            }
            return true;
        };
        if age > MAINT_MAX_AGE {
            if !quiet {
                eprintln!(
                    "service-runner.sh: ignoring stale maintenance hold on '{}' ({age}s old, max {MAINT_MAX_AGE}s): {}",
                    self.cap,
                    self.lock.display()
                );
            }
            let _ = std::fs::remove_file(&self.lock);
            return false;
        }
        true
    }

    fn take_hold(&self) -> R<()> {
        std::fs::write(&self.lock, "").map_err(|e| Fail {
            code: 1,
            msg: format!(
                "service-runner.sh: cannot write {}: {e}",
                self.lock.display()
            ),
        })
    }

    // ---- kind = "process" -------------------------------------------------------------------

    fn process_init(&mut self) -> R<()> {
        let t = &self.table;
        self.command = get_array(t, "command");
        self.build = get_array(t, "build");
        self.panel_build = get_array(t, "panel_build");
        self.build_output = get_str(t, "build_output");
        self.sign = get_str(t, "sign");
        self.workdir = get_str(t, "workdir");
        self.port = get_str(t, "port");
        self.health_path = get_str(t, "health_path");
        if self.command.is_empty() {
            return fail(
                1,
                format!(
                    "service-runner.sh: '{}' is kind=process but declares no command = [...]",
                    self.cap
                ),
            );
        }
        // A manifest that needs a Sjel path from the overlay writes ${SJEL_ROOT}.
        let root = self.paths.root.display().to_string();
        let overlay = self
            .paths
            .overlay_root
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        for c in &mut self.command {
            *c = c
                .replace("${SJEL_ROOT}", &root)
                .replace("${SJEL_OVERLAY_ROOT}", &overlay);
        }
        // Relative with a slash resolves against the capability's root; a bare name is a PATH
        // lookup at exec time.
        if !self.command[0].starts_with('/') && self.command[0].contains('/') {
            self.command[0] = self.cap_root.join(&self.command[0]).display().to_string();
        }
        if let Some(p) = self
            .platform
            .cap_section(&self.cap)
            .map(|t| get_str(t, "port"))
            .filter(|p| !p.is_empty())
        {
            self.port = p;
        }
        let port = self.port.clone();
        for c in &mut self.command {
            *c = c.replace("${SJEL_PORT}", &port);
        }
        Ok(())
    }

    /// The machine's `[capability.<cap>] env` from machine.toml, as KEY=VALUE pairs.
    ///
    /// One reader for every path that needs it: the launchd/systemd unit (`persist::env_block`
    /// renders these) and the on-demand start below. It reached only the unit until 2026-10-04,
    /// so the SAME capability ran with a different environment depending on who started it —
    /// and a capability with `autostart = "false"` never gets a unit at all, so a machine-local
    /// PATH could not reach it by any route. Measured that day: ytalbum's panel and
    /// knowledge-graph both died at start under launchd's PATH (`ytalbum: missing required tool
    /// 'yt-dlp'`, `nohup: bun: No such file or directory`) while the identical command from a
    /// login shell succeeded.
    ///
    /// Process-kind only, deliberately: a container takes its environment from `--env-file`,
    /// and `docker run` inherits nothing, so an entry here would be silently inert for one.
    pub fn cap_env(&self) -> R<Vec<(String, String)>> {
        let entries = self
            .platform
            .cap_section(&self.cap)
            .map(|t| get_array(t, "env"))
            .unwrap_or_default();
        let mut out = Vec::with_capacity(entries.len());
        for line in entries {
            let Some((k, v)) = line.split_once('=') else {
                return fail(
                    1,
                    format!(
                        "service-runner.sh: [capability.{}] env entry '{line}' has no '=' — expected KEY=VALUE",
                        self.cap
                    ),
                );
            };
            out.push((k.to_owned(), v.to_owned()));
        }
        Ok(out)
    }

    fn health_url(&self) -> Option<String> {
        (!self.port.is_empty() && !self.health_path.is_empty())
            .then(|| format!("http://127.0.0.1:{}{}", self.port, self.health_path))
    }

    fn process_healthy(&self) -> bool {
        self.health_url().is_some_and(|u| {
            Command::new("curl")
                .args(["-sf", "-o", "/dev/null", "--max-time", "2", &u])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
    }

    fn running_pid(&self) -> Option<String> {
        let pid = std::fs::read_to_string(&self.pid_file)
            .ok()?
            .trim()
            .to_owned();
        (!pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()) && alive(&pid)).then_some(pid)
    }

    /// Build when the artifact is missing, or always when `force` (restart).
    fn maybe_build(&self, force: bool) -> R<()> {
        if self.build.is_empty() {
            return Ok(());
        }
        if !force {
            if !self.build_output.is_empty() {
                let out = if self.build_output.starts_with('/') {
                    PathBuf::from(&self.build_output)
                } else {
                    self.cap_root.join(&self.build_output)
                };
                if artifact_is_current(&self.build, &out, &self.cap_root) {
                    return Ok(());
                }
            } else if !self.command[0].starts_with('/') {
                // A bare name on PATH with nothing declared to look for.
                return Ok(());
            } else if artifact_is_current(&self.build, Path::new(&self.command[0]), &self.cap_root)
            {
                return Ok(());
            }
        }
        if !self.panel_build.is_empty() {
            println!(
                "building {} panel: {}",
                self.cap,
                self.panel_build.join(" ")
            );
            let ui = self.manifest.parent().unwrap_or(Path::new(".")).join("ui");
            let code = status_of(
                Command::new(&self.panel_build[0])
                    .args(&self.panel_build[1..])
                    .current_dir(ui),
            );
            if code != 0 {
                return fail(code, "");
            }
        }
        println!("building {}: {}", self.cap, self.build.join(" "));
        let dir = self.cap_root.join(if self.workdir.is_empty() {
            "."
        } else {
            &self.workdir
        });
        let code = status_of(
            Command::new(&self.build[0])
                .args(&self.build[1..])
                .current_dir(dir)
                .env("CARGO_TARGET_DIR", self.cap_root.join("target")),
        );
        if code != 0 {
            return fail(1, "");
        }
        if !self.sign.is_empty() {
            let bin = if self.command[0].starts_with('/') {
                PathBuf::from(&self.command[0])
            } else {
                self.cap_root.join(&self.command[0])
            };
            let code = status_of(
                Command::new(self.tools_dir().join("codesign-binary.sh"))
                    .arg(bin)
                    .arg(&self.sign),
            );
            if code != 0 {
                return fail(code, "");
            }
        }
        Ok(())
    }

    /// 0 as soon as the service answers; 1 after 30 s or when the process dies first.
    fn wait_healthy(&self) -> u8 {
        let Some(url) = self.health_url() else {
            return 0;
        };
        for _ in 0..60 {
            if self.process_healthy() {
                return 0;
            }
            if self.running_pid().is_none() {
                eprintln!(
                    "service-runner.sh: '{}' exited during startup — tail {}",
                    self.cap,
                    self.err.display()
                );
                return 1;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        eprintln!(
            "service-runner.sh: '{}' started but never answered {url} — tail {}",
            self.cap,
            self.err.display()
        );
        1
    }

    fn workdir_path(&self) -> PathBuf {
        self.cap_root.join(if self.workdir.is_empty() {
            "."
        } else {
            &self.workdir
        })
    }

    fn shell_port(&self) -> String {
        read_table(&self.paths.root.join("dashboard/service.toml"))
            .map(|t| get_str(&t, "port"))
            .unwrap_or_default()
    }

    fn start_process(&self) -> R<u8> {
        if self.hold_active(false) {
            println!(
                "service-runner.sh: '{}' is held for maintenance, not starting ({})",
                self.cap,
                self.lock.display()
            );
            return Ok(0);
        }
        if !self.schedule.is_empty() {
            return self.run_scheduled();
        }
        if self.running_pid().is_some() {
            return Ok(0); // already ours, already up
        }
        if self.process_healthy() {
            eprintln!(
                "service-runner.sh: '{}' already answers on port {} but is not managed here — stop it yourself first if you want this to own it",
                self.cap, self.port
            );
            return Ok(0);
        }
        self.maybe_build(false)?;
        let open = |p: &Path| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
        };
        let (Ok(out), Ok(err)) = (open(&self.log), open(&self.err)) else {
            return fail(
                1,
                format!(
                    "service-runner.sh: cannot open {} or {}",
                    self.log.display(),
                    self.err.display()
                ),
            );
        };
        // nohup, as the script ran it: it ignores SIGHUP and execs the command, so the pid
        // recorded is the service's own.
        let mut c = Command::new("nohup");
        c.args(&self.command).current_dir(self.workdir_path());
        // The machine's entries first, so the two keys the runner owns below keep the last
        // word — the precedence the unit path already has, where launchd sets the declared
        // environment and this process then overwrites them.
        for (k, v) in self.cap_env()? {
            c.env(k, v);
        }
        c.env("SJEL_SHELL_PORT", self.shell_port())
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        if !self.port.is_empty() {
            c.env("SJEL_PORT", &self.port);
        }
        let child = c.spawn().map_err(|e| Fail {
            code: 1,
            msg: format!(
                "service-runner.sh: cannot start '{}' in {}: {e}",
                self.cap,
                self.workdir_path().display()
            ),
        })?;
        std::fs::write(&self.pid_file, format!("{}\n", child.id())).map_err(|e| Fail {
            code: 1,
            msg: format!(
                "service-runner.sh: cannot write {}: {e}",
                self.pid_file.display()
            ),
        })?;
        Ok(self.wait_healthy())
    }

    /// A periodic job: start what it requires, run it to completion in the foreground, then
    /// stop whatever this run started. Unlike the script, the dependencies are stopped when the
    /// job fails too: `set -e` used to exit before reaching that line.
    fn run_scheduled(&self) -> R<u8> {
        self.maybe_build(false)?;
        let launcher = launcher(self.paths);
        let status_line = |dep: &str| {
            Command::new(&launcher)
                .args(["status", dep])
                .stderr(Stdio::null())
                .output()
                .map(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .next()
                        .unwrap_or("")
                        .to_owned()
                })
                .unwrap_or_default()
        };
        let mut started = Vec::new();
        for dep in get_array(&self.table, "requires") {
            let st = status_line(&dep);
            if st.contains("running") {
                continue;
            }
            if st.contains("held") {
                eprintln!("service-runner.sh: {} requires {dep}, which is HELD — not overriding an operator hold.", self.cap);
                eprintln!("  Release it with: tools/service-runner.sh resume {dep}");
                continue;
            }
            println!(
                "service-runner.sh: {} requires {dep} — starting it",
                self.cap
            );
            let _ = std::io::stdout().flush();
            let ok = Command::new(&launcher)
                .args(["start", &dep])
                .stdout(std::io::stderr())
                .status()
                .is_ok_and(|s| s.success());
            if !ok {
                eprintln!(
                    "service-runner.sh: could not start {dep}; {} may fail",
                    self.cap
                );
                continue;
            }
            started.push(dep.clone());
            let dep_health = read_table(
                &self
                    .paths
                    .root
                    .join("capabilities")
                    .join(&dep)
                    .join("service.toml"),
            )
            .map(|t| get_str(&t, "health_path"))
            .unwrap_or_default();
            let want = if dep_health.is_empty() {
                "running"
            } else {
                "healthy"
            };
            let mut waited = 0;
            while waited < 30 {
                if status_line(&dep).contains(want) {
                    break;
                }
                std::thread::sleep(Duration::from_secs(1));
                waited += 1;
            }
            if waited >= 30 {
                eprintln!("service-runner.sh: {dep} did not report {want} in {waited}s; running {} anyway", self.cap);
            }
        }

        let mut cmd = Command::new(&self.command[0]);
        cmd.args(&self.command[1..])
            .current_dir(self.workdir_path());
        for (k, v) in self.cap_env()? {
            cmd.env(k, v);
        }
        cmd.env("SJEL_SHELL_PORT", self.shell_port());
        let code = status_of(&mut cmd);
        for d in &started {
            println!(
                "service-runner.sh: {} started {d} for this run — stopping it",
                self.cap
            );
            let _ = std::io::stdout().flush();
            let ok = Command::new(&launcher)
                .args(["idle-stop", d])
                .stdout(std::io::stderr())
                .status()
                .is_ok_and(|s| s.success());
            if !ok {
                eprintln!("service-runner.sh: could not stop {d}");
            }
        }
        Ok(code)
    }

    fn stop_process(&self, hold: bool) -> R<u8> {
        if hold {
            self.take_hold()?;
        }
        if let Some(pid) = self.running_pid() {
            kill_tree(&pid);
            for _ in 0..20 {
                if !alive(&pid) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            let _ = Command::new("kill")
                .args(["-KILL", &pid])
                .stderr(Stdio::null())
                .status();
        }
        let _ = std::fs::remove_file(&self.pid_file);
        // A port still answering means something outside this pid file holds it; reporting
        // success there sent the next `start` into "already answers" forever.
        if !self.port.is_empty() {
            for _ in 0..12 {
                if !port_answers(&self.port) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            if port_answers(&self.port) {
                eprintln!("service-runner.sh: '{}' still answers on port {} after stop — refusing to report success.", self.cap, self.port);
                eprintln!("  Something outside this pid file holds it (a reparented child, or an unrelated process).");
                eprintln!("  Identify and stop it, then re-run: lsof -ti tcp:{}   (or: ss -ltnp 'sport = :{}')", self.port, self.port);
                return Ok(1);
            }
        }
        Ok(0)
    }

    fn status_process(&self) {
        let mut state = match self.running_pid() {
            Some(pid) => format!("running (pid {pid})"),
            None => "stopped".to_owned(),
        };
        let health = match self.health_url() {
            Some(_) if self.process_healthy() => "healthy",
            Some(_) => "no answer",
            None => "no health_path",
        };
        if self.hold_active(true) {
            state.push_str(", held");
        }
        println!("  {:<14} {:<9} {:<22} {health}", self.cap, "process", state);
    }

    // ---- kind = "container" -----------------------------------------------------------------

    /// The runtime binary, resolved lazily so a machine running only process capabilities never
    /// needs one installed. On macOS the docker CLI is OrbStack's, which a login shell has on
    /// PATH and a bare launchd environment does not: an unresolvable runtime says so by name.
    pub fn resolve_runtime(&mut self) -> R<PathBuf> {
        if let Some(p) = &self.runtime_path {
            return Ok(p.clone());
        }
        match self.platform.runtime.as_str() {
            "docker" | "podman" => self.runtime_bin.clone_from(&self.platform.runtime),
            other => {
                return fail(1, format!("service-runner.sh: unsupported container_runtime '{other}' (axon-overlay/config/machine.toml)"));
            }
        }
        let Some(p) = which(&self.runtime_bin) else {
            return fail(
                1,
                format!(
                    "service-runner.sh: container_runtime '{}' needs '{}' on PATH, not found (PATH={})",
                    self.platform.runtime,
                    self.runtime_bin,
                    std::env::var("PATH").unwrap_or_default()
                ),
            );
        };
        self.runtime_path = Some(p.clone());
        Ok(p)
    }

    fn prepare(&mut self) -> R<()> {
        if self.prepared {
            return Ok(());
        }
        self.resolve_runtime()?;
        let t = &self.table;
        self.image = get_str(t, "image");
        self.tag = get_str(t, "tag");
        let personal = std::env::var("SJEL_PERSONAL_ROOT").unwrap_or_default();
        let env_file = format!("{personal}/{}", get_str(t, "env_file"));
        let network = get_str(t, "network_mode");
        let mut ports = get_array(t, "ports");
        let volumes = get_array(t, "volumes");
        let caps = get_array(t, "cap_add");
        // A per-machine port override, the container counterpart of a process's `port`.
        if let Some(over) = self
            .platform
            .cap_section(&self.cap)
            .map(|s| get_array(s, "ports"))
            .filter(|v| !v.is_empty())
        {
            ports = over;
        }
        let mut args = vec!["--name".to_owned(), self.name.clone()];
        for c in caps {
            args.extend(["--cap-add".to_owned(), c]);
        }
        if network.is_empty() {
            for p in ports {
                args.extend(["-p".to_owned(), p]);
            }
        } else {
            args.extend(["--network".to_owned(), network]);
        }
        for v in volumes {
            let (host, dest) = v.split_once(':').unwrap_or((v.as_str(), v.as_str()));
            if host.starts_with('/') {
                if !Path::new(host).exists() {
                    return fail(1, format!("service-runner.sh: {} declares the system path {host}, which does not exist on this host", self.cap));
                }
                args.extend(["-v".to_owned(), format!("{host}:{dest}")]);
            } else {
                let resolved = format!("{personal}/{host}");
                std::fs::create_dir_all(&resolved).map_err(|e| Fail {
                    code: 1,
                    msg: format!("service-runner.sh: cannot create {resolved}: {e}"),
                })?;
                args.extend(["-v".to_owned(), format!("{resolved}:{dest}")]);
            }
        }
        args.extend(["--env-file".to_owned(), env_file]);
        self.container_args = args;
        self.prepared = true;
        Ok(())
    }

    fn runtime(&self) -> Command {
        Command::new(&self.runtime_bin)
    }

    /// Is the container listed: `ps` lists running ones, `ps -a` every existing one.
    fn listed(&self, all: bool) -> bool {
        let mut c = self.runtime();
        c.arg("ps");
        if all {
            c.arg("-a");
        }
        c.args(["--format", "{{.Names}}"]).stderr(Stdio::null());
        c.output().is_ok_and(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l == self.name)
        })
    }

    fn start_service(&mut self) -> R<u8> {
        self.prepare()?;
        if self.hold_active(false) {
            println!(
                "service-runner.sh: '{}' is held for maintenance, not starting ({})",
                self.cap,
                self.lock.display()
            );
            return Ok(0);
        }
        if self.listed(false) {
            return Ok(0);
        }
        if self.listed(true) {
            // An existing container was created from SOME declaration; starting it as-is is
            // correct, but a different one is reported rather than silently kept.
            let (report, _) = self.arg_drift();
            if !report.is_empty() {
                eprintln!("{report}");
            }
            return Ok(status_of(self.runtime().args(["start", &self.name])));
        }
        let image = format!("{}:{}", self.image, self.tag);
        Ok(status_of(
            self.runtime()
                .args(["run", "-d", "--restart", "unless-stopped"])
                .args(&self.container_args)
                .arg(image),
        ))
    }

    fn recreate_service(&mut self) -> R<u8> {
        self.prepare()?;
        println!("service-runner.sh: recreating '{}' — declared state mounts survive, undeclared in-container state does not", self.cap);
        let _ = std::io::stdout().flush();
        let quiet = |c: &mut Command| {
            c.stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        quiet(self.runtime().args(["stop", &self.name]));
        if !quiet(self.runtime().args(["rm", &self.name])) {
            quiet(self.runtime().args(["delete", &self.name]));
        }
        let _ = std::fs::remove_file(&self.lock);
        self.start_service()
    }

    fn stop_service(&mut self, hold: bool) -> R<u8> {
        self.prepare()?;
        if hold {
            self.take_hold()?;
        }
        let _ = self
            .runtime()
            .args(["stop", &self.name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        Ok(0)
    }

    /// The running container against the current declaration: `(report, ok)`. Unverifiable is
    /// never reported as equal.
    fn arg_drift(&self) -> (String, bool) {
        let out = self
            .runtime()
            .args(["inspect", &self.name, "--format", "{{json .}}"])
            .stderr(Stdio::null())
            .output();
        let Some(json) = out
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        else {
            return (format!("  the container runtime could not be asked about '{}' — drift unverified, NOT verified equal", self.name), false);
        };
        if json.is_empty() || json == "[]" || json == "null" {
            return (
                format!(
                    "  no container named '{}' exists — drift unverified, NOT verified equal",
                    self.name
                ),
                false,
            );
        }
        let declared = runargs::declared_runspec(&self.container_args);
        let running = runargs::runspec_from_docker(&json);
        let mut lines = Vec::new();
        let mut ok = true;
        for class in ["port", "mount", "cap", "network"] {
            if let Some(d) = runargs::runspec_diff(&declared, &running, class) {
                lines.push(format!("  {class}:\n{d}"));
                ok = false;
            }
        }
        if let Some(envfile) = declared.iter().find_map(|l| l.strip_prefix("envfile ")) {
            let (out, env_ok) =
                runargs::env_diff(Path::new(envfile), &runargs::env_from_docker(&json));
            ok &= env_ok;
            if !out.is_empty() {
                lines.push(format!("  env-file:\n{out}"));
            }
        }
        if !ok {
            lines.push(format!(
                "  the running container was created from a different declaration — 'service-runner.sh recreate {}' applies the current one",
                self.cap
            ));
        }
        (lines.join("\n"), ok)
    }

    fn status_service(&mut self) -> R<u8> {
        self.prepare()?;
        let mut state = if self.listed(false) {
            "running".to_owned()
        } else {
            "stopped".to_owned()
        };
        if self.hold_active(true) {
            state.push_str(", held");
        }
        let mut report = String::new();
        if state.starts_with("running") {
            let (r, ok) = self.arg_drift();
            if !ok {
                let classes: Vec<&str> = r
                    .lines()
                    .filter_map(|l| l.strip_prefix("  ").and_then(|l| l.strip_suffix(':')))
                    .filter(|c| {
                        !c.is_empty() && c.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
                    })
                    .collect();
                let shown = if classes.is_empty() {
                    "unverified".to_owned()
                } else {
                    classes.join(", ")
                };
                state.push_str(&format!(", drift: {shown}"));
            }
            report = r;
        }
        println!(
            "  {:<14} {:<9} {:<22} {}:{}",
            self.cap, "container", state, self.image, self.tag
        );
        if !report.is_empty() {
            eprintln!("{report}");
        }
        Ok(0)
    }
}

// ---- helpers ------------------------------------------------------------------------------

/// The exit status of a foreground child, its output inherited. A program that cannot be
/// started reads 127, as the shell reports one.
pub fn status_of(c: &mut Command) -> u8 {
    match c.status() {
        Ok(s) => s.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(1),
        Err(e) => {
            eprintln!(
                "service-runner.sh: {}: {e}",
                c.get_program().to_string_lossy()
            );
            127
        }
    }
}

fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// TERM a process and every descendant, children first, as found by `pgrep -P`.
fn kill_tree(pid: &str) {
    if let Ok(o) = Command::new("pgrep")
        .args(["-P", pid])
        .stderr(Stdio::null())
        .output()
    {
        for child in String::from_utf8_lossy(&o.stdout).split_whitespace() {
            kill_tree(child);
        }
    }
    let _ = Command::new("kill")
        .args(["-TERM", pid])
        .stderr(Stdio::null())
        .status();
}

fn port_answers(port: &str) -> bool {
    port.parse::<u16>().is_ok_and(|p| {
        std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], p)),
            Duration::from_secs(2),
        )
        .is_ok()
    })
}

/// `command -v` for an executable: the path itself when it names one, else the first match on
/// PATH. Unlike the shell, a builtin (`true`) resolves to its binary on PATH.
pub fn which(bin: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let exe = |p: &Path| {
        p.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if bin.contains('/') {
        return exe(Path::new(bin)).then(|| PathBuf::from(bin));
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(bin))
        .find(|p| exe(p))
}

pub fn read_table(path: &Path) -> R<toml::Table> {
    let body = std::fs::read_to_string(path).map_err(|e| Fail {
        code: 1,
        msg: format!("service-runner.sh: cannot read {}: {e}", path.display()),
    })?;
    body.parse::<toml::Table>().map_err(|e| Fail {
        code: 1,
        msg: format!("service-runner.sh: cannot parse {}: {e}", path.display()),
    })
}

pub fn get_str(t: &toml::Table, key: &str) -> String {
    t.get(key)
        .and_then(toml::Value::as_str)
        .unwrap_or("")
        .to_owned()
}

pub fn get_array(t: &toml::Table, key: &str) -> Vec<String> {
    t.get(key)
        .and_then(toml::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(toml::Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The sections of the overlay's config/systems.local.toml whose `owner = "self"`, in file
/// order, comma-joined: SJEL_INFERENCE_TRUSTED_PEERS. The rule of `trusted_peer_ids` in
/// tools/lib/external-ref.sh, which backup.sh and setup-secret.sh still source.
fn trusted_peers(paths: &Paths) -> String {
    let Some(file) = paths
        .overlay_root
        .as_ref()
        .map(|o| o.join("config/systems.local.toml"))
        .filter(|p| p.is_file())
    else {
        return String::new();
    };
    let body = std::fs::read_to_string(file).unwrap_or_default();
    let Ok(map) =
        toml::from_str::<std::collections::BTreeMap<String, toml::Spanned<toml::Value>>>(&body)
    else {
        return String::new();
    };
    let mut v: Vec<(usize, String)> = map
        .into_iter()
        .filter(|(_, t)| {
            t.get_ref()
                .as_table()
                .is_some_and(|t| get_str(t, "owner") == "self")
        })
        .map(|(n, t)| (t.span().start, n))
        .collect();
    v.sort();
    v.into_iter().map(|(_, n)| n).collect::<Vec<_>>().join(",")
}

/// The newest modification time among the Rust sources under `root`. Cargo owns the dependency
/// graph; this only decides whether to ask it, so a coarse "some source is newer" is enough — a
/// cargo build with nothing to do is cheap. Build output and vendored trees are skipped.
fn newest_rust_source(root: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if matches!(
                    name.as_ref(),
                    "target" | "node_modules" | ".git" | "graphify-out"
                ) {
                    continue;
                }
                stack.push(entry.path());
            } else if kind.is_file() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.ends_with(".rs") || name == "Cargo.toml" || name == "Cargo.lock" {
                    if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                        if newest.is_none_or(|n| modified > n) {
                            newest = Some(modified);
                        }
                    }
                }
            }
        }
    }
    newest
}

/// Whether the artifact a manifest declares is already built from the sources it has.
///
/// A missing artifact is never current. For a cargo build the newest source under `root` decides,
/// so a scheduled job picks up a change to a workspace crate it depends on instead of running a
/// binary built before it — measured 2026-10-03: `entities-server` was built 2026-09-30, before
/// the loopback-auth change in `libs/sjel-server`, and answered 401 for four runs. Any other build
/// tool keeps the old "exists means built" rule, because re-running an arbitrary build on every
/// tick is not something this can promise is cheap.
fn artifact_is_current(build: &[String], artifact: &Path, root: &Path) -> bool {
    let Ok(built) = std::fs::metadata(artifact).and_then(|m| m.modified()) else {
        return false;
    };
    if build.first().map(String::as_str) != Some("cargo") {
        return true;
    }
    newest_rust_source(root).is_none_or(|newest| newest <= built)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_at(path: &Path, when: SystemTime) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent dir");
        }
        std::fs::write(path, "x").expect("write");
        std::fs::File::options()
            .write(true)
            .open(path)
            .expect("open")
            .set_modified(when)
            .expect("set mtime");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sjel-runner-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn cargo_build() -> Vec<String> {
        vec!["cargo".to_owned(), "build".to_owned()]
    }

    #[test]
    fn a_cargo_artifact_older_than_its_sources_is_stale() {
        let dir = scratch("stale");
        let artifact = dir.join("target/release/tool");
        let source = dir.join("libs/x/src/lib.rs");
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let new = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000);
        write_at(&artifact, old);
        write_at(&source, new);
        assert!(!artifact_is_current(&cargo_build(), &artifact, &dir));
        write_at(&artifact, new);
        assert!(artifact_is_current(&cargo_build(), &artifact, &dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_output_under_target_is_not_a_source() {
        let dir = scratch("target");
        let artifact = dir.join("target/release/tool");
        let generated = dir.join("target/release/build/x/out.rs");
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let new = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000);
        write_at(&artifact, old);
        write_at(&generated, new);
        assert!(artifact_is_current(&cargo_build(), &artifact, &dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_non_cargo_build_keeps_exists_means_built() {
        let dir = scratch("noncargo");
        let artifact = dir.join("target/release/tool");
        let source = dir.join("src/lib.rs");
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let new = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000);
        write_at(&artifact, old);
        write_at(&source, new);
        let bun = vec!["bun".to_owned(), "run".to_owned()];
        assert!(artifact_is_current(&bun, &artifact, &dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_artifact_is_never_current() {
        let dir = scratch("missing");
        let artifact = dir.join("target/release/tool");
        assert!(!artifact_is_current(&cargo_build(), &artifact, &dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
