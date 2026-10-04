//! `tools/capability.sh` — which capabilities are enabled on this machine, and the registry.
//!
//! Ported from bash on 2026-10-02; the script path stays as the launcher, so every caller
//! (service-runner.sh, sjel-status, dashboard/vite.config.ts, doctor, self) is unchanged.
//!
//! The `capabilities = [...]` line in <overlay>/config/machine.toml is the single source of
//! truth for the enabled set, and this is the one tool that writes it. `requires`
//! dependencies resolve transitively and cycle-safely on enable, and are guarded on disable so a
//! still-needed capability cannot be stranded. Nothing is started here: after enable, the next
//! command is printed, never run.
//!
//! `registry` is the one place manifest facts leave for non-TOML consumers, in dependency
//! order, so `up` starts a capability after whatever it requires and `down` walks it backwards.
//! `sjel capability` and `sjel search` call [`services`] in-process.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use serde::Deserialize;

use crate::paths::{duplicate_message, Manifest, Paths};

const HELP: &str = "\
tools/capability.sh — manage which capabilities are enabled on this machine.

The `capabilities = [...]` line in <overlay>/config/machine.toml is the single source of truth,
and this is the one tool that writes it (enable/disable). requires= dependencies are resolved
transitively and cycle-safely on enable, and guarded on disable so a still-needed capability
can't be stranded. No service is ever started here: after enable, the suggested
`tools/service-runner.sh start <name>` is printed for you to run.

  tools/capability.sh list             # every capability + enabled/disabled + requires
  tools/capability.sh enable <name>    # enable <name> and everything it requires
  tools/capability.sh disable <name>   # disable <name> (blocked if a dependent needs it)
  tools/capability.sh registry         # the enabled set as JSON, in dependency order
  tools/capability.sh -h               # this help";

/// The registry's scalar fields, in emission order. backup_* ride the same list: they are
/// manifest facts sjel-status needs and may not parse TOML for (CONTRIBUTING.md#one-manifest-
/// per-concern). `backup_sqlite_online` keeps `backup_sqlite` honest: that copy holds nothing
/// down, and a consumer reading only `backup_sqlite` would render "this stops the service".
const FIELDS: [&str; 17] = [
    "port",
    "health_path",
    "ready_path",
    "panel_port",
    "panel_path",
    "autostart",
    "schedule",
    "proxy_api_only",
    "idle_timeout",
    "routes_absent",
    "backup_target",
    "backup_sqlite",
    "backup_sqlite_online",
    "backup_advise_days",
    "backup_stale_days",
    "freshness_advise_hours",
    "freshness_stale_hours",
];

/// An exit code and the message that goes with it, as the script's `exit` carried them.
#[derive(Debug)]
pub struct Fail {
    pub code: u8,
    pub msg: String,
}

fn fail<T>(code: u8, msg: impl Into<String>) -> Result<T, Fail> {
    Err(Fail {
        code,
        msg: msg.into(),
    })
}

/// One registry entry.
#[derive(Debug, Clone)]
pub struct Service {
    pub name: String,
    pub kind: String,
    pub scope: String,
    /// The [`FIELDS`], in order, empty when absent.
    pub fields: Vec<(&'static str, String)>,
    pub endpoint: String,
    pub proxy_extra: Vec<String>,
    pub requires: Vec<String>,
}

impl Service {
    pub fn field(&self, key: &str) -> &str {
        self.fields
            .iter()
            .find(|(k, _)| *k == key)
            .map_or("", |(_, v)| v)
    }
}

struct Ctx<'a> {
    paths: &'a Paths,
    machine: PathBuf,
    /// Parsed manifests, by path. A manifest that does not parse reads as empty, as the line
    /// reader it replaced read whatever lines matched; tools/check-service-tomls.sh reports it.
    cache: RefCell<HashMap<PathBuf, toml::Table>>,
}

impl<'a> Ctx<'a> {
    fn new(paths: &'a Paths) -> Result<Self, Fail> {
        let machine = paths.machine_toml.clone().unwrap_or_default();
        if !machine.is_file() {
            return fail(
                1,
                format!(
                    "capability.sh: no machine.toml at {} — run tools/install.sh first.",
                    machine.display()
                ),
            );
        }
        Ok(Self {
            paths,
            machine,
            cache: RefCell::new(HashMap::new()),
        })
    }

    fn table(&self, path: &Path) -> toml::Table {
        if let Some(t) = self.cache.borrow().get(path) {
            return t.clone();
        }
        let t = std::fs::read_to_string(path)
            .ok()
            .and_then(|b| b.parse::<toml::Table>().ok())
            .unwrap_or_default();
        self.cache.borrow_mut().insert(path.to_owned(), t.clone());
        t
    }

    fn get(&self, path: &Path, key: &str) -> String {
        get_str(&self.table(path), key)
    }

    fn array(&self, path: &Path, key: &str) -> Vec<String> {
        get_array(&self.table(path), key)
    }

    /// machine.toml's enabled set, in its own order.
    fn enabled(&self) -> Vec<String> {
        self.array(&self.machine, "capabilities")
    }

    /// Direct `requires` of a capability. A name declared in both roots is reported and
    /// requires nothing, as the script's `_cap_requires` did.
    fn requires(&self, name: &str) -> Vec<String> {
        match self.paths.manifest(name) {
            Manifest::Found(p) => self.array(&p, "requires"),
            Manifest::None => Vec::new(),
            Manifest::Duplicate(c, o) => {
                eprintln!("{}", duplicate_message(name, &c, &o));
                Vec::new()
            }
        }
    }

    fn manifest_field(&self, name: &str, key: &str) -> String {
        self.paths
            .manifest_for(name)
            .map(|p| self.get(&p, key))
            .unwrap_or_default()
    }

    /// `name` and its transitive requires, dependencies first, appended to `resolved`.
    /// `visiting` holds every name entered, so a cycle stops instead of recursing.
    fn resolve(
        &self,
        name: &str,
        resolved: &mut Vec<String>,
        visiting: &mut Vec<String>,
    ) -> Result<(), Fail> {
        if resolved.iter().any(|r| r == name) || visiting.iter().any(|v| v == name) {
            return Ok(());
        }
        if self.paths.cap_dir(name).is_none() {
            return fail(
                1,
                format!("capability.sh: required capability '{name}' has no capabilities/{name}/ directory in Axon or the overlay"),
            );
        }
        visiting.push(name.to_owned());
        for dep in self.requires(name) {
            self.resolve(&dep, resolved, visiting)?;
        }
        if !resolved.iter().any(|r| r == name) {
            resolved.push(name.to_owned());
        }
        Ok(())
    }

    /// The enabled set and everything it requires, dependencies first.
    fn resolved_enabled(&self) -> Result<Vec<String>, Fail> {
        let (mut resolved, mut visiting) = (Vec::new(), Vec::new());
        for n in self.enabled() {
            self.resolve(&n, &mut resolved, &mut visiting)?;
        }
        Ok(resolved)
    }

    /// Every capability directory name across both roots, sorted and deduplicated.
    fn cap_dirs(&self) -> Vec<String> {
        let mut names: Vec<String> = [
            Some(&self.paths.caps_dir),
            self.paths.overlay_caps_dir.as_ref(),
        ]
        .into_iter()
        .flatten()
        .flat_map(|root| subdirs(root))
        .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Spine components: every `<root>/<name>/service.toml`. Discovered rather than listed, so
    /// the list cannot go stale (CONTRIBUTING.md#three-architectural-nouns).
    fn spine(&self) -> Vec<String> {
        subdirs(&self.paths.root)
            .into_iter()
            .filter(|n| self.paths.root.join(n).join("service.toml").is_file())
            .collect()
    }

    /// Capabilities this machine consumes from another host: `[capability.<name>]` sections of
    /// machine.toml with a `provided_by`, in file order.
    fn externals(&self) -> Result<Vec<(String, String)>, Fail> {
        #[derive(Deserialize)]
        struct Machine {
            #[serde(default)]
            capability: BTreeMap<String, toml::Spanned<toml::Table>>,
        }
        let body = std::fs::read_to_string(&self.machine).unwrap_or_default();
        let m: Machine = toml::from_str(&body).map_err(|e| Fail {
            code: 1,
            msg: format!(
                "capability.sh: cannot parse {}: {e}",
                self.machine.display()
            ),
        })?;
        let mut v: Vec<_> = m
            .capability
            .into_iter()
            .map(|(name, t)| (t.span().start, name, get_str(t.get_ref(), "provided_by")))
            .filter(|(_, _, p)| !p.is_empty())
            .collect();
        v.sort();
        Ok(v.into_iter().map(|(_, n, p)| (n, p)).collect())
    }

    /// The base URL of the host that provides `name`, from the overlay's
    /// config/systems.local.toml — the provided_by half of `capability_endpoint` in
    /// tools/lib/external-ref.sh. backup.sh, setup-secret.sh and materialize-inference-key still
    /// source that file, so the rule has two copies until they move: change both.
    fn endpoint(&self, name: &str, provider: &str) -> Result<String, Fail> {
        let systems = self
            .paths
            .overlay_root
            .clone()
            .unwrap_or_default()
            .join("config/systems.local.toml");
        let url = if systems.is_file() {
            let t = self.table(&systems);
            lookup(&t, provider)
                .map(|s| get_str(s, "url"))
                .unwrap_or_default()
        } else {
            String::new()
        };
        if url.is_empty() {
            return fail(
                1,
                format!(
                    "external-ref: {} declares [capability.{name}] provided_by = \"{provider}\",\n  but {} has no [{provider}] url = \"...\" to resolve it to.\n  Add that entry, or drop the provided_by line if this machine runs {name} itself.\ncapability.sh: cannot resolve the external provider for '{name}' — see above.",
                    self.machine.display(),
                    systems.display()
                ),
            );
        }
        Ok(url.strip_suffix('/').unwrap_or(&url).to_owned())
    }

    /// One registry entry. `manifest` is `None` for an external capability with no manifest
    /// here: its manifest lives in whichever repository owns its host.
    fn service(&self, name: &str, manifest: Option<&Path>, scope: &str, endpoint: &str) -> Service {
        let table = manifest.map(|p| self.table(p)).unwrap_or_default();
        let mut kind = get_str(&table, "kind");
        if kind.is_empty() {
            // "container" is the manifest default, but only where there is a manifest; calling
            // someone else's deployment a container would invent a fact about it.
            kind = if manifest.is_none() {
                "external"
            } else {
                "container"
            }
            .to_owned();
        }
        let external = scope == "external";
        let fields = FIELDS
            .iter()
            .map(|&k| {
                // A manifest says how its OWNER runs the capability. On a machine that only
                // consumes it, every field but the two that describe how to ASK would be a claim
                // of authority it does not have: a watchdog for another host's process, a backup
                // button on someone else's database. Blanked here rather than per consumer.
                let keep = !external || k == "health_path" || k == "ready_path";
                (
                    k,
                    if keep {
                        get_str(&table, k)
                    } else {
                        String::new()
                    },
                )
            })
            .collect();
        let (proxy_extra, requires) = if external {
            (Vec::new(), Vec::new())
        } else {
            (
                get_array(&table, "proxy_extra"),
                get_array(&table, "requires"),
            )
        };
        Service {
            name: name.to_owned(),
            kind,
            scope: scope.to_owned(),
            fields,
            endpoint: endpoint.to_owned(),
            proxy_extra,
            requires,
        }
    }

    /// The registry: enabled set in dependency order, then external capabilities, then the
    /// spine. Spine last: it consumes the capabilities, so its first discovery call already sees
    /// them. Externals only when `with_externals`: `--lines` drives the runner's fan-out, and a
    /// row there would walk the runner over a service on another host.
    fn registry(&self, with_externals: bool) -> Result<Vec<Service>, Fail> {
        let mut out = Vec::new();
        for n in self.resolved_enabled()? {
            // `_has_service` in the script: a name declared twice is skipped here; resolve
            // already reported it.
            let Manifest::Found(mf) = self.paths.manifest(&n) else {
                continue;
            };
            // scope names the root, so consumers do not re-derive it from a path. `tools/self`
            // uses it to keep overlay capabilities out of the tracked, public self.json.
            let scope = match &self.paths.overlay_caps_dir {
                Some(o) if mf.starts_with(o) => "overlay-capability",
                _ => "capability",
            };
            out.push(self.service(&n, Some(&mf), scope, ""));
        }
        if with_externals {
            // An unresolvable reference is fatal: a silent omission would reach every consumer as
            // "not configured on this machine", which is precisely the wrong answer.
            for (n, provider) in self.externals()? {
                let endpoint = self.endpoint(&n, &provider)?;
                let mf = match self.paths.manifest(&n) {
                    Manifest::Found(p) => Some(p),
                    Manifest::None => None,
                    Manifest::Duplicate(c, o) => return fail(2, duplicate_message(&n, &c, &o)),
                };
                out.push(self.service(&n, mf.as_deref(), "external", &endpoint));
            }
        }
        for n in self.spine() {
            let mf = self.paths.root.join(&n).join("service.toml");
            out.push(self.service(&n, Some(&mf), "spine", ""));
        }
        Ok(out)
    }
}

/// The registry without external capabilities, in dependency order: what `registry --lines`
/// prints and what the service runner's whole-machine fan-out walks.
pub fn runner_rows(paths: &Paths) -> Result<Vec<Service>, Fail> {
    Ctx::new(paths)?.registry(false)
}

/// The registry, for in-process callers (`sjel capability`, `sjel search`).
pub fn services(paths: &Paths) -> Result<Vec<Service>, Fail> {
    Ctx::new(paths)?.registry(true)
}

fn get_str(t: &toml::Table, key: &str) -> String {
    t.get(key)
        .and_then(toml::Value::as_str)
        .unwrap_or("")
        .to_owned()
}

/// A single-line string array; empty elements are dropped, as the line reader dropped them.
fn get_array(t: &toml::Table, key: &str) -> Vec<String> {
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

/// The table at a dotted section name (`[homepi]`, `[a.b]`).
fn lookup<'t>(t: &'t toml::Table, dotted: &str) -> Option<&'t toml::Table> {
    dotted
        .split('.')
        .try_fold(t, |cur, k| cur.get(k)?.as_table())
}

/// Visible subdirectory names, sorted.
fn subdirs(root: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut v: Vec<String> = rd
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    v.sort();
    v
}

// ---- the command --------------------------------------------------------------------------

pub fn run(args: &[String]) -> ExitCode {
    if matches!(args.first().map(String::as_str), Some("-h" | "--help")) {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let result = Paths::from_env()
        .map_err(|msg| Fail {
            code: 1,
            msg: format!("capability.sh: {msg}"),
        })
        .and_then(|paths| dispatch(&paths, args));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(f) => {
            if !f.msg.is_empty() {
                eprintln!("{}", f.msg);
            }
            ExitCode::from(f.code)
        }
    }
}

fn dispatch(paths: &Paths, args: &[String]) -> Result<(), Fail> {
    let ctx = Ctx::new(paths)?;
    let arg = |i: usize| args.get(i).map_or("", String::as_str);
    match args.first().map_or("list", String::as_str) {
        "list" => {
            list(&ctx);
            Ok(())
        }
        "enable" if !arg(1).is_empty() => enable(&ctx, arg(1)),
        "enable" => fail(1, "usage: capability.sh enable <name>"),
        "disable" if !arg(1).is_empty() => disable(&ctx, arg(1)),
        "disable" => fail(1, "usage: capability.sh disable <name>"),
        "registry" => {
            let text = if arg(1) == "--lines" {
                lines(&ctx.registry(false)?)
            } else {
                json(&ctx.registry(true)?)?
            };
            print!("{text}");
            Ok(())
        }
        _ => fail(
            1,
            "usage: capability.sh list | enable <name> | disable <name> | registry [--lines]",
        ),
    }
}

fn list(ctx: &Ctx) {
    let enabled = ctx.enabled();
    for name in ctx.cap_dirs() {
        let status = if enabled.contains(&name) {
            "enabled"
        } else {
            "disabled"
        };
        let reqs = ctx.requires(&name);
        if reqs.is_empty() {
            println!("  {name:<18} [{status}]");
        } else {
            println!("  {name:<18} [{status}]  requires: {}", reqs.join(" "));
        }
    }
}

fn enable(ctx: &Ctx, name: &str) -> Result<(), Fail> {
    if ctx.paths.cap_dir(name).is_none() {
        let options: String = ctx.cap_dirs().iter().map(|d| format!("\n  {d}")).collect();
        return fail(
            1,
            format!("capability.sh: no such capability '{name}'\nvalid options:{options}"),
        );
    }
    let (mut resolved, mut visiting) = (Vec::new(), Vec::new());
    ctx.resolve(name, &mut resolved, &mut visiting)?;
    println!(
        "Resolution chain (dependencies first): {}",
        resolved.join(" ")
    );

    let enabled = ctx.enabled();
    let mut newly = Vec::new();
    for n in &resolved {
        if enabled.contains(n) {
            println!("  = {n} (already enabled)");
        } else {
            println!("  + {n} (enabling)");
            newly.push(n.clone());
        }
    }
    if newly.is_empty() {
        println!("Nothing to do — already enabled.");
        return Ok(());
    }

    // Appended to the existing order, which stays untouched, so an enable/disable round trip
    // is byte-identical.
    let mut all = enabled;
    all.extend(newly.iter().cloned());
    write_capabilities(&ctx.machine, &all)?;
    println!("machine.toml: capabilities updated.");

    let suggested: Vec<&String> = newly
        .iter()
        .filter(|n| ctx.paths.manifest_for(n).is_some())
        .collect();
    if suggested.is_empty() {
        return Ok(());
    }
    println!();
    println!("Next step (not run automatically — start each when ready):");
    for n in suggested {
        // A scheduled capability has no useful `start`: starting it by hand runs one tick. The
        // unit IS the way it runs.
        if !ctx.manifest_field(n, "schedule").is_empty() {
            println!("  tools/service-runner.sh install-persistence {n}   # declares a schedule — the timer IS how it runs");
            continue;
        }
        // kind=data has nothing to start: the runner refuses `start` for it by name.
        if ctx.manifest_field(n, "kind") == "data" {
            println!("  tools/backup.sh {n}   # declares data, not a process — nothing to start");
            continue;
        }
        println!("  tools/service-runner.sh start {n}");
        // `start` alone does not survive a reboot, which is how a capability could run all day
        // and be gone the next morning (#9). Named, not done: this tool starts nothing.
        if ctx.manifest_field(n, "autostart") == "true" {
            println!("  tools/service-runner.sh install-persistence {n}   # declares autostart — survives a reboot only with this");
        }
    }
    Ok(())
}

fn disable(ctx: &Ctx, name: &str) -> Result<(), Fail> {
    let enabled = ctx.enabled();
    if !enabled.iter().any(|e| e == name) {
        println!("capability.sh: '{name}' is not enabled — nothing to do.");
        return Ok(());
    }
    // Direct requires suffice: a transitive dependent reaches `name` only through an enabled
    // intermediate that directly requires it, and that intermediate is caught here.
    let mut dependents = Vec::new();
    for e in enabled.iter().filter(|e| *e != name) {
        for dep in ctx.requires(e) {
            if dep == name {
                dependents.push(e.clone());
            }
        }
    }
    if !dependents.is_empty() {
        return fail(
            1,
            format!(
                "capability.sh: cannot disable '{name}' — still required by: {}\ndisable the dependent(s) first.",
                dependents.join(" ")
            ),
        );
    }
    let kept: Vec<String> = enabled.into_iter().filter(|n| n != name).collect();
    write_capabilities(&ctx.machine, &kept)?;
    println!("Disabled '{name}'. machine.toml: capabilities updated.");

    // A leftover persistence unit is not inert: watchdog.sh starts the capability every 30s
    // and consults nothing about the enabled set (#9). Reported, not removed: unloading a unit
    // is a machine-level side effect, and tools/doctor repeats it for as long as it is true.
    let state = Command::new(ctx.paths.root.join("tools/service-runner.sh"))
        .args(["persistence-status", name])
        .stderr(Stdio::null())
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.split('\t').nth(1).unwrap_or(l).to_owned())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    if matches!(
        state.as_str(),
        "installed" | "installed-not-loaded" | "stale"
    ) {
        println!();
        println!("Persistence is still installed for '{name}'. Its watchdog will keep starting it every 30s.");
        println!("  tools/service-runner.sh remove-persistence {name}");
    }
    Ok(())
}

/// Rewrite the one `capabilities = [...]` line, or append it with its comment when machine.toml
/// predates the field. More than one such line is a corrupted file, and a best-effort write
/// would quietly lose whichever line the reader was not using.
fn write_capabilities(machine: &Path, names: &[String]) -> Result<(), Fail> {
    let body = std::fs::read_to_string(machine).map_err(|e| Fail {
        code: 1,
        msg: format!("capability.sh: cannot read {}: {e}", machine.display()),
    })?;
    let is_line = |l: &str| {
        l.strip_prefix("capabilities")
            .is_some_and(|r| r.trim_start_matches([' ', '\t']).starts_with('='))
    };
    let count = body.lines().filter(|l| is_line(l)).count();
    if count > 1 {
        return fail(
            1,
            format!(
                "capability.sh: found {count} 'capabilities = [...]' lines in {} — fix that file by hand first.",
                machine.display()
            ),
        );
    }
    let quoted: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
    let newline = format!("capabilities = [{}]", quoted.join(", "));
    let next = if count == 0 {
        format!(
            "{body}\n# Capabilities enabled on THIS machine — written by tools/capability.sh\n# (enable/disable resolve service.toml `requires =` transitively); hand-editing\n# is legal, and tools/doctor re-checks that the set stays dependency-closed.\n# Single-line array per tools/lib/toml.sh's contract.\n{newline}\n"
        )
    } else {
        body.split_inclusive('\n')
            .map(|l| {
                if is_line(l) {
                    let eol = if l.ends_with('\n') { "\n" } else { "" };
                    format!("{newline}{eol}")
                } else {
                    l.to_owned()
                }
            })
            .collect()
    };
    replace_file(machine, &next).map_err(|e| Fail {
        code: 1,
        msg: format!("capability.sh: cannot write {}: {e}", machine.display()),
    })
}

/// Write beside the target and rename over it, keeping its permissions, so a crash mid-write
/// leaves the old file whole.
fn replace_file(path: &Path, body: &str) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("toml.tmp.{}", std::process::id()));
    std::fs::write(&tmp, body)?;
    if let Ok(meta) = std::fs::metadata(path) {
        std::fs::set_permissions(&tmp, meta.permissions())?;
    }
    std::fs::rename(&tmp, path)
}

// ---- rendering ----------------------------------------------------------------------------

/// A JSON string. Refuses anything that would need escaping rather than escaping it: no
/// manifest value is supposed to contain a quote or a backslash, and one that does is a
/// manifest to fix, not a value to carry.
fn json_str(v: &str) -> Result<String, Fail> {
    if v.contains(['"', '\\']) {
        return fail(
            1,
            format!("capability.sh: manifest value needs JSON escaping, which this emitter deliberately does not do: {v}"),
        );
    }
    Ok(format!("\"{v}\""))
}

fn json_array(v: &[String]) -> Result<String, Fail> {
    let items: Result<Vec<String>, Fail> = v.iter().map(|s| json_str(s)).collect();
    Ok(format!("[{}]", items?.join(", ")))
}

/// The registry as JSON, byte for byte the layout the shell emitter printed: consumers parse
/// it, and tests compare it.
fn json(services: &[Service]) -> Result<String, Fail> {
    let mut rows = Vec::with_capacity(services.len());
    for s in services {
        let mut r = format!(
            "{{\"name\": {}, \"kind\": {}, \"scope\": {}",
            json_str(&s.name)?,
            json_str(&s.kind)?,
            json_str(&s.scope)?
        );
        for (k, v) in &s.fields {
            let _ = write!(r, ", \"{k}\": {}", json_str(v)?);
        }
        let _ = write!(
            r,
            ", \"endpoint\": {}, \"proxy_extra\": {}, \"requires\": {}}}",
            json_str(&s.endpoint)?,
            json_array(&s.proxy_extra)?,
            json_array(&s.requires)?
        );
        rows.push(r);
    }
    Ok(format!("[\n  {}\n]\n", rows.join(",\n  ")))
}

/// The runner's view: `name kind scope autostart`, one per line.
fn lines(services: &[Service]) -> String {
    services
        .iter()
        .map(|s| {
            let autostart = s.field("autostart");
            format!(
                "{} {} {} {}\n",
                s.name,
                s.kind,
                s.scope,
                if autostart.is_empty() {
                    "false"
                } else {
                    autostart
                }
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_refuses_values_that_need_escaping() {
        assert!(json_str("plain").is_ok());
        assert!(json_str("a\"b").is_err());
        assert!(json_str("a\\b").is_err());
    }

    #[test]
    fn an_empty_registry_keeps_the_shell_layout() {
        assert_eq!(json(&[]).unwrap(), "[\n  \n]\n");
    }

    #[test]
    fn lines_default_autostart_to_false() {
        let s = Service {
            name: "a".into(),
            kind: "process".into(),
            scope: "capability".into(),
            fields: FIELDS.iter().map(|&k| (k, String::new())).collect(),
            endpoint: String::new(),
            proxy_extra: vec![],
            requires: vec![],
        };
        assert_eq!(lines(&[s]), "a process capability false\n");
    }

    #[test]
    fn external_sections_keep_file_order() {
        #[derive(Deserialize)]
        struct Machine {
            capability: BTreeMap<String, toml::Spanned<toml::Table>>,
        }
        let m: Machine = toml::from_str(
            "capabilities = [\"a\"]\n[capability.zeta]\nprovided_by = \"h\"\n[capability.alpha]\nprovided_by = \"h\"\n",
        )
        .unwrap();
        let mut v: Vec<_> = m
            .capability
            .iter()
            .map(|(n, t)| (t.span().start, n.clone()))
            .collect();
        v.sort();
        assert_eq!(
            v.into_iter().map(|(_, n)| n).collect::<Vec<_>>(),
            ["zeta", "alpha"]
        );
    }

    #[test]
    fn the_capabilities_line_is_rewritten_in_place() {
        let dir = std::env::temp_dir().join(format!("sjel-cli-machine-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("machine.toml");
        std::fs::write(
            &p,
            "os = \"macos\"\ncapabilities = [\"a\"]   # trailing\n[capability.x]\n",
        )
        .unwrap();
        write_capabilities(&p, &["a".into(), "b".into()]).unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "os = \"macos\"\ncapabilities = [\"a\", \"b\"]\n[capability.x]\n"
        );
        std::fs::write(&p, "capabilities = []\ncapabilities = [\"a\"]\n").unwrap();
        assert_eq!(write_capabilities(&p, &[]).unwrap_err().code, 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
