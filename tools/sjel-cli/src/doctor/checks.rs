//! The doctor's sections, in the order doctor.ts ran them. Each keeps the reasoning its
//! TypeScript original stated where the reasoning is not obvious from the code.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;
use serde_json::Value as Json;

use crate::harnesses::engine::SkillStatus;
use crate::harnesses::registry::{is_installed, Model, Registry};
use crate::harnesses::statuses_for;

use super::overlay::{resolve_machine_toml, resolve_overlay_root};
use super::pure::{self, AgeState, Level, Outcome, Producer, Target};
use super::{
    capture, cmd, exists, expand_home, git, home, js_num, js_number_str, js_round, js_value,
    mtime_secs, now_secs, read_toml, read_toml_ordered, to_fixed, Ctx, Out,
};

type Check = fn(&mut Ctx);

/// The report, in the order doctor.ts printed it. `Packs` stands for the per-harness Pack
/// sections, however many installed harnesses they report.
enum Entry {
    One(&'static str, Check),
    Packs,
}

/// Sections whose results later sections read: the overlay, machine.toml, the state mounts
/// (Systems compares them) and systems.toml (reachability and the connection sweep read it).
/// They run first, in order; every other section only reads the context they leave.
const CONTEXT: [&str; 4] = [
    "Overlay",
    "Machine identity",
    "State mounts",
    "Systems (systems.toml)",
];

fn entries() -> Vec<Entry> {
    use Entry::{One, Packs};
    vec![
        One("Overlay", overlay),
        One("Machine identity", machine_identity),
        One("Host toolchain (tools/toolchain-check)", host_toolchain),
        One("Global Bun/npm install policy", bun_policy),
        One(
            "Local inference roles (tools/model-check --local)",
            inference_roles,
        ),
        One("AI assistant integrations", integrations),
        One("Claude Code settings (sjel claude)", claude_settings),
        One("Capabilities (enabled set)", enabled_set),
        One("Capabilities (external references)", external_refs),
        One(
            "Boot persistence (autostart + schedule set)",
            boot_persistence,
        ),
        One("Scheduled producers (did they run)", scheduled_producers),
        One("Shared store (SQLite)", shared_store),
        One("State mounts", state_mounts),
        One(
            "Capability env templates (public/private split)",
            env_templates,
        ),
        One("Systems (systems.toml)", systems),
        One("Systems reachability (--online)", reachability),
        One(
            "Undeclared connections (grep sweep)",
            undeclared_connections,
        ),
        One("Server bind policy (sjel-server)", bind_policy),
        One(
            "Tailnet identity gate (SJEL_TAILNET_OPERATOR)",
            tailnet_gate,
        ),
        One(
            "Vault pointers (stored paths that must resolve)",
            vault_pointers,
        ),
        One("Data freshness (declared contracts)", data_freshness),
        One("Backups (receipts, and the archives they name)", backups),
        One("Port uniqueness (declared, both roots)", port_uniqueness),
        Packs,
        One("Doctrine freshness (README why-blocks)", doctrine_freshness),
        One("Self-model freshness (self.json)", self_model),
        One(
            "Architecture-generator input visibility (Axon#30)",
            generator_inputs,
        ),
        One("Service manifests (both roots)", service_manifests),
        One("Publication hygiene (tracked tree)", publication_hygiene),
        One("UI type-check coverage (Axon#139)", ui_coverage),
        One("Host patch (capabilities/host-patch)", host_patch),
        One(
            "Container refresh (capabilities/container-refresh)",
            container_refresh,
        ),
        One("Build artifacts (PRD §9 R6)", build_artifacts),
        One("Repo freshness (origin/main)", repo_freshness),
        One("Session orientation", session_orientation),
    ]
}

/// Run one entry against `ctx`, which starts with an empty buffer, and return its rendered
/// text and failure count. `SJEL_DOCTOR_TIMING=1` prints each entry's wall time to stderr.
fn render(entry: &Entry, ctx: &mut Ctx) {
    let start = std::time::Instant::now();
    let name = match entry {
        Entry::One(name, f) => {
            ctx.line(format!("\n{name}"));
            f(ctx);
            *name
        }
        Entry::Packs => {
            pack_sections(ctx);
            "Packs (per harness)"
        }
    };
    if std::env::var_os("SJEL_DOCTOR_TIMING").is_some() {
        eprintln!("{:>7.2}s  {name}", start.elapsed().as_secs_f64());
    }
}

/// The context sections in order, then every other section at once on a snapshot of what they
/// left. The report prints in the original order, so running in parallel changes the wall time
/// and nothing a reader sees: the doctor spends its time waiting on the tools it delegates to
/// (measured 2026-10-02: 26 s sequential, 14 s of it one `sjel-storage target` walk).
pub fn run_all(ctx: &mut Ctx) {
    let entries = entries();
    let mut rendered: Vec<Option<(String, u32)>> = entries.iter().map(|_| None).collect();
    for (i, e) in entries.iter().enumerate() {
        if let Entry::One(name, _) = e {
            if CONTEXT.contains(name) {
                ctx.out.clear();
                ctx.failed = 0;
                render(e, ctx);
                rendered[i] = Some((std::mem::take(&mut ctx.out), ctx.failed));
            }
        }
    }
    let snapshot = ctx.clone();
    let done: Vec<(usize, String, u32)> = std::thread::scope(|s| {
        let handles: Vec<_> = entries
            .iter()
            .enumerate()
            .filter(|(i, _)| rendered[*i].is_none())
            .map(|(i, e)| {
                let mut c = snapshot.clone();
                s.spawn(move || {
                    c.out.clear();
                    c.failed = 0;
                    render(e, &mut c);
                    (i, c.out, c.failed)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| {
                    (
                        usize::MAX,
                        "\n  ✗ a doctor section panicked — run it again with RUST_BACKTRACE=1\n"
                            .to_owned(),
                        1,
                    )
                })
            })
            .collect()
    });
    let mut panicked = Vec::new();
    for (i, out, failed) in done {
        match rendered.get_mut(i) {
            Some(slot) => *slot = Some((out, failed)),
            None => panicked.push((out, failed)),
        }
    }
    ctx.failed = 0;
    for (out, failed) in rendered.into_iter().flatten().chain(panicked) {
        print!("{out}");
        ctx.failed += failed;
    }
}

fn section(ctx: &mut Ctx, name: &str, f: impl FnOnce(&mut Ctx)) {
    ctx.line(format!("\n{name}"));
    f(ctx);
}

fn self_model(c: &mut Ctx) {
    print_and_judge(
        c,
        "self",
        &["check"],
        Level::Warn,
        "self.json is stale — run: tools/self generate",
    );
}

fn generator_inputs(c: &mut Ctx) {
    print_and_judge(
        c,
        "check-generator-inputs-tracked.sh",
        &[],
        Level::Bad,
        "the architecture generator reads an input others cannot see — see above",
    );
}

fn publication_hygiene(c: &mut Ctx) {
    print_and_judge(
        c,
        "check-publication-hygiene.sh",
        &[],
        Level::Bad,
        "tracked content is not safe for a public checkout — see above",
    );
}

fn ui_coverage(c: &mut Ctx) {
    print_and_judge(
        c,
        "discover-ui-packages",
        &[],
        Level::Bad,
        "a UI package declares a surface CI cannot type-check — see above",
    );
}

// ---- JSON as JavaScript reads it ----------------------------------------------------------

/// `${v}` for a JSON value, `undefined` when absent.
fn jstr(v: Option<&Json>) -> String {
    match v {
        None => "undefined".to_owned(),
        Some(Json::String(s)) => s.clone(),
        Some(Json::Number(n)) => n
            .as_i64()
            .map(|i| i.to_string())
            .unwrap_or_else(|| js_num(n.as_f64().unwrap_or(f64::NAN))),
        Some(Json::Bool(b)) => b.to_string(),
        Some(Json::Null) => "null".to_owned(),
        Some(Json::Array(a)) => a
            .iter()
            .map(|x| {
                if x.is_null() {
                    String::new()
                } else {
                    jstr(Some(x))
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Json::Object(_)) => "[object Object]".to_owned(),
    }
}

/// JavaScript truthiness.
fn truthy(v: Option<&Json>) -> bool {
    match v {
        None | Some(Json::Null) => false,
        Some(Json::Bool(b)) => *b,
        Some(Json::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Json::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn jfield<'a>(v: &'a Json, k: &str) -> Option<&'a Json> {
    v.get(k)
}

/// The string at `k`, or "" when absent or not a string; `||` defaults read it.
fn jtext(v: &Json, k: &str) -> String {
    v.get(k).and_then(Json::as_str).unwrap_or("").to_owned()
}

fn tool(ctx: &Ctx, name: &str) -> PathBuf {
    ctx.root.join("tools").join(name)
}

fn overlay_dir(ctx: &Ctx) -> PathBuf {
    PathBuf::from(&ctx.overlay_path)
}

/// `tools/capability.sh registry`, as doctor.ts read it: a failed or non-JSON registry skips
/// the section with a warning.
fn registry(ctx: &mut Ctx) -> Option<Vec<Json>> {
    let o = cmd(tool(ctx, "capability.sh"), &["registry"]);
    if !o.success() {
        ctx.warn("capability.sh registry failed — skipping");
        return None;
    }
    match serde_json::from_str::<Vec<Json>>(&o.stdout) {
        Ok(v) => Some(v),
        Err(_) => {
            ctx.warn("capability.sh registry did not return JSON — skipping");
            None
        }
    }
}

fn registry_field(s: &Json, k: &str) -> String {
    jtext(s, k)
}

// ---- sections -----------------------------------------------------------------------------

fn overlay(ctx: &mut Ctx) {
    let Some(o) = resolve_overlay_root(&ctx.root) else {
        ctx.bad("no 'overlay' in axon.local.toml or axon.toml — run tools/install.sh");
        return;
    };
    ctx.overlay_path = o.root;
    if exists(&ctx.overlay_path) {
        let msg = format!("overlay at {} (from {})", ctx.overlay_path, o.source);
        ctx.ok(msg);
        if o.source == "axon.toml" {
            ctx.warn("no axon.local.toml — this machine is running on the shipped default; run tools/install.sh to pin it");
        }
    } else {
        let msg = format!(
            "overlay declared (from {}) but missing at {} — run tools/install.sh",
            o.source, ctx.overlay_path
        );
        ctx.bad(msg);
    }
}

fn machine_identity(ctx: &mut Ctx) {
    if ctx.overlay_path.is_empty() || !exists(&ctx.overlay_path) {
        ctx.warn("skipped — no overlay to check");
        return;
    }
    let m = resolve_machine_toml(&ctx.root, &ctx.overlay_path);
    if !m.path.exists() {
        let named = if m.source == "axon.local.toml" {
            format!(
                " — axon.local.toml names machine '{}', which has no manifest",
                m.name.clone().unwrap_or_default()
            )
        } else {
            " — run tools/install.sh".to_owned()
        };
        ctx.bad(format!("missing {}{named}", m.path.display()));
    } else {
        if m.source == "config/machine.toml" {
            ctx.ok("machine: single-file layout");
        } else {
            ctx.ok(format!(
                "machine: {} (from {})",
                m.name.clone().unwrap_or_default(),
                m.source
            ));
        }
        match read_toml(&m.path) {
            Ok(t) => {
                ctx.machine = t;
                ctx.machine_src = std::fs::read_to_string(&m.path).unwrap_or_default();
            }
            Err(e) => ctx.bad(format!("{} is not valid TOML — {e}", m.path.display())),
        }
        match ctx.machine_str("os") {
            Some(os) => ctx.ok(format!("os = {os}")),
            None => ctx.bad("machine.toml: missing 'os'"),
        }
        match ctx.machine_str("container_runtime") {
            Some(rt) => ctx.ok(format!("container_runtime = {rt}")),
            None => ctx.bad("machine.toml: missing 'container_runtime'"),
        }
    }
    let overlay = overlay_dir(ctx);
    let strays: Vec<PathBuf> = [
        overlay.join("config/machine.toml.example"),
        overlay.join("config/machines/machine.toml.example"),
    ]
    .into_iter()
    .filter(|p| p.exists())
    .collect();
    if strays.is_empty() {
        ctx.ok("machine schema: not duplicated into the overlay");
    }
    for s in strays {
        ctx.bad(format!(
            "{} duplicates schemas/machine.toml.example — delete it and drop its .gitignore allowlist line; the schema lives in Axon only",
            s.display()
        ));
    }
}

fn host_toolchain(ctx: &mut Ctx) {
    let checker = tool(ctx, "toolchain-check");
    if !checker.exists() {
        ctx.warn(format!("missing {}", checker.display()));
        return;
    }
    let mut args = vec!["--json".to_owned()];
    if let Some(os) = ctx.machine_str("os") {
        args.extend(["--os".to_owned(), os]);
    }
    if let Some(rt) = ctx.machine_str("container_runtime") {
        args.extend(["--runtime".to_owned(), rt]);
    }
    let o = capture(Command::new(&checker).args(&args));
    let Ok(data) = serde_json::from_str::<Json>(&o.stdout) else {
        ctx.warn("toolchain-check did not emit JSON — run tools/toolchain-check for detail");
        return;
    };
    let entries: Vec<Json> = data
        .get("entries")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    let status = |e: &Json| jtext(e, "status");
    let class = |e: &Json| jtext(e, "class");
    let missing: Vec<&Json> = entries.iter().filter(|e| status(e) == "missing").collect();
    let outdated_req: Vec<&Json> = entries
        .iter()
        .filter(|e| status(e) == "outdated" && class(e) != "optional")
        .collect();
    let absent: Vec<&Json> = entries.iter().filter(|e| status(e) == "absent").collect();
    let outdated_opt: Vec<&Json> = entries
        .iter()
        .filter(|e| status(e) == "outdated" && class(e) == "optional")
        .collect();
    let f = |e: &Json, k: &str| jstr(jfield(e, k));
    for e in &missing {
        ctx.bad(format!(
            "{} missing ({}) — install: {}",
            f(e, "bin"),
            f(e, "class"),
            f(e, "install")
        ));
    }
    for e in &outdated_req {
        ctx.bad(format!(
            "{} {} — install: {}",
            f(e, "bin"),
            f(e, "note"),
            f(e, "install")
        ));
    }
    for e in &absent {
        ctx.warn(format!(
            "{} absent (optional) — install: {}",
            f(e, "bin"),
            f(e, "install")
        ));
    }
    for e in &outdated_opt {
        ctx.warn(format!("{} {}", f(e, "bin"), f(e, "note")));
    }
    let na = entries.iter().filter(|e| status(e) == "n/a").count();
    if missing.is_empty() && outdated_req.is_empty() {
        let tail = if absent.is_empty() {
            String::new()
        } else {
            format!(", {} optional absent", absent.len())
        };
        let scope = if na == 0 {
            String::new()
        } else {
            format!(", {na} n/a here")
        };
        let totals = data.get("totals");
        let t = |k: &str| {
            totals
                .and_then(|t| t.get(k))
                .map_or("0".to_owned(), |v| jstr(Some(v)))
        };
        ctx.ok(format!(
            "{}/{} required present{tail}{scope}",
            t("ok"),
            t("count")
        ));
    }
    if na > 0 {
        ctx.ok("scoped to this machine — 'tools/toolchain-check --workflow backup|restore|audit|build' before running one");
    }
}

fn bun_policy(ctx: &mut Ctx) {
    let home = home();
    if home.is_empty() {
        ctx.warn("HOME is unset — cannot inspect ~/.bunfig.toml");
        return;
    }
    let path = Path::new(&home).join(".bunfig.toml");
    if !path.exists() {
        ctx.warn("~/.bunfig.toml is absent — run tools/install.sh and accept the optional 24h npm/Bun hold");
        return;
    }
    match read_toml(&path) {
        Ok(t) => match t.get("install").and_then(|i| i.get("minimumReleaseAge")) {
            Some(v) if super::js_number(Some(v)) == 86400.0 && !v.is_str() => ctx.ok("~/.bunfig.toml minimumReleaseAge = 86400 (24h)"),
            None => ctx.warn("~/.bunfig.toml has no install.minimumReleaseAge = 86400 — run tools/install.sh to add the hold"),
            Some(v) => ctx.warn(format!("~/.bunfig.toml minimumReleaseAge is {}, expected 86400 — run tools/install.sh to reconcile it", js_value(v))),
        },
        Err(e) => ctx.bad(format!("~/.bunfig.toml is not valid TOML — {e}")),
    }
}

fn inference_roles(ctx: &mut Ctx) {
    let checker = tool(ctx, "model-check.ts");
    if !checker.exists() {
        ctx.warn(format!("missing {}", checker.display()));
        return;
    }
    let o = capture(
        Command::new("bun")
            .arg(&checker)
            .args(["--local", "--json"]),
    );
    let Ok(data) = serde_json::from_str::<Json>(&o.stdout) else {
        ctx.warn(
            "model-check did not emit JSON — run 'bun tools/model-check.ts --local' for detail",
        );
        return;
    };
    let entries: Vec<Json> = data
        .get("entries")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    if entries.is_empty() {
        ctx.ok("no inference role names a loopback backend on this machine");
        return;
    }
    for e in &entries {
        let f = |k: &str| jstr(e.get(k));
        let status = jtext(e, "status");
        if status == "missing" || status == "incomplete" {
            ctx.bad(format!(
                "{}: {} on {} — {}",
                f("role"),
                f("model"),
                f("backend"),
                f("detail")
            ));
        } else if status == "unreachable" {
            let role = jtext(e, "role");
            let cost = if role == "embedding" {
                " — relevance falls back to its lexical control until it is up"
            } else if role.starts_with("summarization") {
                " — the digest ladder falls through to the remaining rungs"
            } else {
                ""
            };
            ctx.warn(format!(
                "{}: {} on {} — {}{cost}. Axon does not supervise a systems.toml tool (see [{}]); start it, or point the role elsewhere.",
                f("role"),
                f("model"),
                f("backend"),
                f("detail"),
                f("backend")
            ));
        }
    }
    let t = data.get("totals").cloned().unwrap_or(Json::Null);
    if !truthy(t.get("missing")) && !truthy(t.get("incomplete")) && !truthy(t.get("unreachable")) {
        ctx.ok(format!(
            "{}/{} local role(s) answering",
            jstr(t.get("ok")),
            jstr(t.get("count"))
        ));
    }
}

fn integrations(ctx: &mut Ctx) {
    let script = tool(ctx, "agent-integrations.sh");
    if !script.exists() {
        ctx.warn(format!("missing {}", script.display()));
        return;
    }
    let o = capture(Command::new(&script).args(["status", "--json"]));
    if !o.success() {
        ctx.warn(
            "agent-integrations status failed (run: tools/agent-integrations.sh status --json)",
        );
        return;
    }
    let Ok(payload) = serde_json::from_str::<Json>(&o.stdout) else {
        ctx.warn("agent-integrations status did not emit JSON — run: tools/agent-integrations.sh status --json");
        return;
    };
    let mut rows: Vec<(String, Json)> = Vec::new();
    for i in payload
        .get("integrations")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
    {
        for h in i
            .get("harnesses")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
        {
            // `{ upstream: h.upstream ?? i.upstream ?? "unknown", ...h }`: a key h carries wins.
            let upstream = match h.get("upstream") {
                Some(v) => jtext_value(v),
                None => i
                    .get("upstream")
                    .filter(|v| !v.is_null())
                    .map_or("unknown".to_owned(), jtext_value),
            };
            if truthy(h.get("name")) {
                rows.push((upstream, h.clone()));
            }
        }
    }
    if rows.is_empty() {
        ctx.warn("no assistant integration rows reported");
        return;
    }
    for (upstream, item) in rows {
        let or = |k: &str, d: &str| {
            Some(jtext(&item, k))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| d.to_owned())
        };
        let name = or("name", "unknown");
        let upstream = if upstream.is_empty() {
            "graphify".to_owned()
        } else {
            upstream
        };
        let state = or("state", "unknown");
        let install = or(
            "install_command",
            &format!("tools/agent-integrations.sh install {upstream} {name}"),
        )
        .trim()
        .to_owned();
        let location = Some(jtext(&item, "config_dir"))
            .filter(|s| !s.is_empty())
            .map(|d| format!(" ({d})"))
            .unwrap_or_default();
        let graph = or("graph_state", "unknown");
        let command = or("command", "unknown");
        let version = Some(jtext(&item, "command_version"))
            .filter(|s| !s.is_empty())
            .map(|v| format!(" ({v})"))
            .unwrap_or_default();
        let suffix = format!("graph={graph}; command={command}{version}");
        match state.as_str() {
            "integrated" if upstream == "graphify" && graph != "present" => {
                ctx.warn(format!("{name}: {state}{location}; {suffix}; check graph with tools/graphify.sh"));
            }
            "integrated" => ctx.ok(format!("{name}: {state}{location}; {suffix}")),
            "runnable" => ctx.warn(format!("{name}: {state}{location}; {suffix}; install with: {install}")),
            "configured" => ctx.warn(format!("{name}: {state}{location}; {suffix}; install command failed partially — check: {install}")),
            "stale" => ctx.warn(format!("{name}: {state}{location}; {suffix}; refresh with: {install}")),
            "missing" => ctx.warn(format!("{name}: {state}{location}; {suffix}; install flow depends on harness presence")),
            _ => ctx.warn(format!("{name}: {state}{location}; {suffix}; run: tools/agent-integrations.sh status --json")),
        }
    }
}

/// A JSON value as `||` would keep it: a non-empty string, else "".
fn jtext_value(v: &Json) -> String {
    if truthy(Some(v)) {
        jstr(Some(v))
    } else {
        String::new()
    }
}

fn cargo_target(ctx: &Ctx) -> PathBuf {
    std::env::var("CARGO_TARGET_DIR")
        .ok()
        .filter(|v| !v.is_empty())
        .map_or_else(|| ctx.root.join("target"), PathBuf::from)
}

fn claude_settings(ctx: &mut Ctx) {
    let bin = cargo_target(ctx).join("release/sjel-claude-config");
    if !bin.exists() {
        ctx.warn("sjel-claude-config not built — run `sjel claude check` to compare settings");
        return;
    }
    let dir = std::env::var("CLAUDE_CONFIG_DIR")
        .ok()
        .filter(|v| !v.is_empty())
        .map_or_else(|| format!("{}/.claude", home()), |d| expand_home(&d));
    let target = Path::new(&dir).join("settings.json");
    if !target.exists() {
        ctx.warn(format!(
            "no {} yet — apply Sjel's baseline with: sjel claude",
            target.display()
        ));
        return;
    }
    let mut c = Command::new(&bin);
    c.arg("check").env("SJEL_ROOT", &ctx.root);
    if !ctx.overlay_path.is_empty() {
        c.env("SJEL_OVERLAY_ROOT", &ctx.overlay_path);
    }
    let o = capture(&mut c);
    let output = format!("{}{}", o.stdout, o.stderr);
    if o.code == Some(0) {
        let line = output
            .split('\n')
            .find(|l| l.contains("matches the baseline"));
        let msg = line.map_or("matches the baseline".to_owned(), |l| {
            let re = Regex::new(r"^claude-code-config:\s*").expect("pattern");
            re.replace(l, "").into_owned()
        });
        ctx.ok(msg);
        return;
    }
    if o.code != Some(3) {
        let first = output
            .split('\n')
            .find(|l| !l.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "exit {}",
                    o.code.map_or("null".to_owned(), |c| c.to_string())
                )
            });
        ctx.warn(format!("check did not run: {first}"));
        return;
    }
    let re = Regex::new(r"^\s+(changed|missing)\s{2}\S").expect("pattern");
    let drift: Vec<String> = output
        .split('\n')
        .filter(|l| re.is_match(l))
        .map(|l| l.trim().to_owned())
        .collect();
    let shown = if drift.is_empty() {
        "unknown keys".to_owned()
    } else {
        drift.join("; ")
    };
    ctx.warn(format!(
        "{} has drifted from Sjel's baseline: {shown} · restore: sjel claude --force",
        target.display()
    ));
}

fn enabled_set(ctx: &mut Ctx) {
    let enabled = ctx.enabled();
    if ctx.machine.is_empty() {
        ctx.warn("skipped — no machine.toml to read");
        return;
    }
    if enabled.is_empty() {
        ctx.warn("none enabled (tools/capability.sh enable <name>)");
        return;
    }
    let mut requires: Vec<(String, Vec<String>)> = Vec::new();
    let mut all_present = true;
    for name in &enabled {
        let root_dir = ctx.root.join("capabilities").join(name);
        let overlay_cap = overlay_dir(ctx).join("capabilities").join(name);
        let dir = if root_dir.exists() {
            root_dir.clone()
        } else {
            overlay_cap.clone()
        };
        if !dir.exists() {
            ctx.bad(format!(
                "'{name}' is enabled but exists in neither capabilities/{name}/ nor the overlay's"
            ));
            all_present = false;
            continue;
        }
        if root_dir.exists() && overlay_cap.exists() {
            ctx.bad(format!("'{name}' is declared in both roots — rename one"));
            all_present = false;
            continue;
        }
        let svc = dir.join("service.toml");
        let reqs = if svc.exists() {
            read_toml(&svc)
                .ok()
                .and_then(|t| {
                    t.get("requires")
                        .and_then(toml::Value::as_array)
                        .map(|a| a.iter().map(js_value).collect())
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if let Some(slot) = requires.iter_mut().find(|(n, _)| n == name) {
            slot.1 = reqs;
        } else {
            requires.push((name.clone(), reqs));
        }
    }
    if all_present {
        ctx.ok(format!(
            "{} enabled, every one a real capabilities/<name>/ dir in Axon or the overlay",
            enabled.len()
        ));
    }
    let missing: Vec<String> = requires
        .iter()
        .flat_map(|(n, reqs)| {
            reqs.iter()
                .filter(|d| !enabled.contains(d))
                .map(move |d| format!("{n} requires '{d}', which is not enabled"))
        })
        .collect();
    if missing.is_empty() {
        ctx.ok("enabled set is dependency-closed");
    }
    for m in missing {
        ctx.bad(m);
    }
}

/// `[capability.<name>]` sections of machine.toml, in file order.
fn capability_sections(ctx: &Ctx) -> Vec<(String, toml::Table)> {
    #[derive(serde::Deserialize)]
    struct M {
        #[serde(default)]
        capability: BTreeMap<String, toml::Spanned<toml::Value>>,
    }
    let Ok(m) = toml::from_str::<M>(&ctx.machine_src) else {
        return Vec::new();
    };
    let mut v: Vec<(usize, String, toml::Table)> = m
        .capability
        .into_iter()
        .filter_map(|(k, s)| {
            let start = s.span().start;
            s.into_inner().as_table().cloned().map(|t| (start, k, t))
        })
        .collect();
    v.sort_by_key(|(s, _, _)| *s);
    v.into_iter().map(|(_, k, t)| (k, t)).collect()
}

fn external_refs(ctx: &mut Ctx) {
    if ctx.machine.is_empty() {
        ctx.warn("skipped — no machine.toml to read");
        return;
    }
    let declared: Vec<(String, String)> = capability_sections(ctx)
        .into_iter()
        .filter_map(|(n, t)| {
            t.get("provided_by")
                .and_then(toml::Value::as_str)
                .filter(|p| !p.is_empty())
                .map(|p| (n, p.to_owned()))
        })
        .collect();
    if declared.is_empty() {
        ctx.ok("none declared — every capability here is locally managed");
        return;
    }
    let systems_path = overlay_dir(ctx).join("config/systems.local.toml");
    let systems = systems_path
        .exists()
        .then(|| read_toml(&systems_path).unwrap_or_default());
    let enabled = ctx.enabled();
    for (name, provider) in declared {
        if enabled.contains(&name) {
            ctx.bad(format!(
                "'{name}' is enabled AND declared as provided by '{provider}' — it cannot be both"
            ));
            continue;
        }
        let Some(systems) = &systems else {
            ctx.bad(format!("'{name}' names provider '{provider}', but the overlay has no config/systems.local.toml"));
            continue;
        };
        let url = systems
            .get(&provider)
            .and_then(|e| e.get("url"))
            .and_then(toml::Value::as_str)
            .unwrap_or("");
        if url.is_empty() {
            ctx.bad(format!("'{name}' names provider '{provider}', which has no url in config/systems.local.toml"));
            continue;
        }
        ctx.ok(format!(
            "'{name}' resolves through systems.local.toml [{provider}]"
        ));
    }
}

fn boot_persistence(ctx: &mut Ctx) {
    let runner = tool(ctx, "service-runner.sh");
    if !runner.exists() {
        ctx.warn(format!("missing {}", runner.display()));
        return;
    }
    let o = cmd(&runner, &["persistence"]);
    let lines: Vec<String> = o
        .stdout
        .trim()
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    if lines.is_empty() {
        ctx.warn("service-runner.sh persistence returned nothing — persistence state unverified");
    }
    let mut owed = 0;
    for line in &lines {
        let cols: Vec<&str> = line.split('\t').collect();
        let name = cols.first().copied().unwrap_or("undefined");
        let state = cols.get(1).copied().unwrap_or("undefined");
        let detail = cols.get(2).copied().unwrap_or("undefined");
        match state {
            "installed" | "n/a" => {}
            "missing" => {
                ctx.bad(format!(
                    "'{name}' owes a supervisor unit and has none installed — it will not run after a reboot (tools/service-runner.sh install-persistence {name})"
                ));
                owed += 1;
            }
            "misdeclared" => {
                ctx.bad(format!("'{name}': {detail}"));
                owed += 1;
            }
            "stale" | "installed-not-loaded" => {
                ctx.warn(format!("'{name}': {detail}"));
                owed += 1;
            }
            "unsupported" => ctx.warn(format!("'{name}': {detail}")),
            other => ctx.warn(format!("'{name}': unexpected persistence state '{other}'")),
        }
    }

    // A unit installed for a capability this machine does not enable: its watchdog starts it
    // anyway. Spine components (a root-level service.toml) and their sidecars are exempt.
    let enabled = ctx.enabled();
    let os = ctx.machine_str("os").unwrap_or_default();
    let unit_dir = match os.as_str() {
        "macos" => Some(PathBuf::from(home()).join("Library/LaunchAgents")),
        "linux" => Some(
            std::env::var_os("XDG_CONFIG_HOME")
                .map_or_else(|| PathBuf::from(home()).join(".config"), PathBuf::from)
                .join("systemd/user"),
        ),
        _ => None,
    };
    let mut exempt = BTreeSet::new();
    for name in sorted_dir_names(&ctx.root) {
        let svc = ctx.root.join(&name).join("service.toml");
        if !svc.exists() {
            continue;
        }
        exempt.insert(name);
        if let Ok(t) = read_toml(&svc) {
            for s in t
                .get("sidecars")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(s) = s.as_str().filter(|s| !s.is_empty()) {
                    exempt.insert(s.to_owned());
                }
            }
        }
    }
    let mut orphans = Vec::new();
    if let Some(dir) = unit_dir.filter(|d| d.exists()) {
        for f in std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
        {
            let f = f.file_name().to_string_lossy().into_owned();
            let cap = if os == "macos" {
                pure::launchd_unit_capability(&f)
            } else {
                f.strip_prefix("axon-")
                    .and_then(|r| r.strip_suffix(".service"))
                    .map(str::to_owned)
            };
            if let Some(c) = cap.filter(|c| !exempt.contains(c) && !enabled.contains(c)) {
                orphans.push(c);
            }
        }
    }
    orphans.sort();
    for cap in &orphans {
        ctx.warn(format!(
            "persistence is installed for '{cap}', which this machine does not enable — its watchdog will start it anyway (tools/service-runner.sh remove-persistence {cap})"
        ));
    }
    if owed == 0 && orphans.is_empty() && !lines.is_empty() {
        ctx.ok(format!(
            "{} enabled capabilities checked, persistence matches the declaration",
            lines.len()
        ));
    }
}

/// Directory names under `dir`, sorted the way Node's readdirSync returns them.
fn sorted_dir_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn scheduled_producers(ctx: &mut Ctx) {
    let os = ctx.machine_str("os");
    if os.as_deref() != Some("macos") {
        ctx.ok(format!(
            "skipped — os = {}; this reads launchd units, and systemd timers are not covered yet",
            os.unwrap_or_else(|| "unknown".to_owned())
        ));
        return;
    }
    let Some(reg) = registry(ctx) else { return };
    let enabled = ctx.enabled();
    let scheduled: Vec<String> = reg
        .iter()
        .filter(|s| !registry_field(s, "schedule").trim().is_empty())
        .map(|s| registry_field(s, "name"))
        .filter(|n| enabled.is_empty() || enabled.contains(n))
        .collect();
    if scheduled.is_empty() {
        ctx.ok("no capability on this machine declares a schedule");
        return;
    }
    let list = cmd("launchctl", &["list"]);
    if !list.success() {
        ctx.warn("launchctl list failed — scheduled producers unverified");
        return;
    }
    let jobs = pure::parse_launchd_jobs(&list.stdout);
    let unit_dir = PathBuf::from(home()).join("Library/LaunchAgents");
    let now = now_secs();
    for name in scheduled {
        let candidates = [format!("com.sjel.{name}"), format!("com.axon.{name}")];
        let label = candidates
            .iter()
            .find(|l| unit_dir.join(format!("{l}.plist")).exists() || jobs.contains_key(*l))
            .cloned()
            .unwrap_or_else(|| format!("com.sjel.{name}"));
        let unit_path = unit_dir.join(format!("{label}.plist"));
        let installed = unit_path.exists();
        let unit = if installed {
            pure::parse_launchd_schedule(&std::fs::read_to_string(&unit_path).unwrap_or_default())
        } else {
            pure::LaunchdSchedule {
                interval: None,
                stdout: None,
                stderr: None,
            }
        };
        let newest = [unit.stdout.as_deref(), unit.stderr.as_deref()]
            .into_iter()
            .flatten()
            .filter_map(mtime_secs)
            .fold(None, |a: Option<f64>, m| Some(a.map_or(m, |a| a.max(m))));
        let job = jobs.get(&label);
        let (level, msg) = pure::classify_scheduled_producer(&Producer {
            name: name.clone(),
            unit_installed: installed,
            loaded: job.is_some(),
            last_exit: job.and_then(|j| j.last_exit),
            interval: unit.interval,
            last_output_age: newest.map(|n| (now - n).max(0.0)),
        });
        emit(ctx, level, msg);
    }
}

fn emit(ctx: &mut Ctx, level: Level, msg: String) {
    match level {
        Level::Ok => ctx.ok(msg),
        Level::Warn => ctx.warn(msg),
        Level::Bad => ctx.bad(msg),
    }
}

fn db_path(ctx: &Ctx) -> (String, &'static str, bool) {
    let env = std::env::var("SJEL_DB_PATH")
        .unwrap_or_default()
        .trim()
        .to_owned();
    if env.is_empty() {
        (
            overlay_dir(ctx)
                .join("data/axon/axon.db")
                .display()
                .to_string(),
            "overlay default",
            false,
        )
    } else {
        (expand_home(&env), "SJEL_DB_PATH", true)
    }
}

fn shared_store(ctx: &mut Ctx) {
    let (db, from, from_env) = db_path(ctx);
    if ctx.overlay_path.is_empty() && !from_env {
        ctx.warn("skipped — no overlay to resolve the database path from");
        return;
    }
    if !exists(&db) {
        ctx.warn(format!(
            "no database at {db} ({from}) — created on the first write by any capability"
        ));
        return;
    }
    let o = cmd(
        "sqlite3",
        &[&format!("file:{db}?mode=ro"), "pragma integrity_check;"],
    );
    if !o.success() {
        let why = Some(o.stderr.trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "sqlite3 is not on PATH".to_owned());
        ctx.bad(format!("{db} could not be read: {why}"));
    } else {
        let first = o.stdout.trim().split('\n').next().unwrap_or("").to_owned();
        if first == "ok" {
            ctx.ok(format!("{db} ({from}) — integrity_check ok"));
        } else {
            ctx.bad(format!("{db} failed integrity_check: {first}"));
        }
    }
    if !ctx.enabled().iter().any(|e| e == "store") {
        ctx.warn("'store' is not in this machine's enabled set — the database exists and no backup contract covers it (tools/capability.sh enable store)");
    }
}

fn state_mounts(ctx: &mut Ctx) {
    ctx.mounts = ctx
        .machine
        .get("state_mount")
        .and_then(toml::Value::as_array)
        .map(|a| {
            a.iter()
                .map(|m| {
                    (
                        m.get("tool")
                            .map(js_value)
                            .unwrap_or_else(|| "undefined".to_owned()),
                        m.get("path").map(js_value).unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if ctx.mounts.is_empty() {
        ctx.warn("none declared");
        return;
    }
    for (t, p) in ctx.mounts.clone() {
        let p = expand_home(&p);
        if exists(&p) {
            ctx.ok(format!("{t} — {p}"));
        } else {
            ctx.bad(format!("{t} — {p} missing"));
        }
    }
}

fn env_templates(ctx: &mut Ctx) {
    let caps = ctx.root.join("capabilities");
    if !caps.exists() {
        ctx.warn("no capabilities/ dir");
        return;
    }
    let (mut checked, mut found) = (0, 0);
    for name in sorted_dir_names(&caps) {
        let dir = caps.join(&name);
        let svc = dir.join("service.toml");
        if !svc.exists() {
            continue;
        }
        let env_file = read_toml(&svc)
            .ok()
            .and_then(|t| {
                t.get("env_file")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        if env_file.trim().is_empty() {
            continue;
        }
        checked += 1;
        let base = env_file.rsplit('/').next().unwrap_or(&env_file).to_owned();
        if !base.ends_with(".env") {
            ctx.warn(format!(
                "capabilities/{name}: env_file should probably end with .env"
            ));
            continue;
        }
        let template = dir.join(format!("{base}.example"));
        if !template.exists() {
            ctx.bad(format!(
                "capabilities/{name}: missing {base}.example for env-backed service"
            ));
            continue;
        }
        found += 1;
        let Ok(text) = std::fs::read(&template) else {
            ctx.bad(format!("capabilities/{name}: cannot read {base}.example"));
            continue;
        };
        for key in pure::find_plaintext_secrets_in_env_template(&String::from_utf8_lossy(&text)) {
            ctx.bad(format!("capabilities/{name}: {base}.example contains raw-looking secret-like value for {key} (use placeholders only)"));
        }
    }
    if checked == 0 {
        ctx.ok("no env_file-backed capabilities found");
    } else if found == checked {
        ctx.ok(format!(
            "{found}/{checked} env-backed capabilities ship .env.example"
        ));
    } else {
        ctx.bad(format!(
            "{found}/{checked} env-backed capabilities ship .env.example"
        ));
    }
}

fn systems(ctx: &mut Ctx) {
    let path = ctx.root.join("systems.toml");
    if !path.exists() {
        ctx.warn("no systems.toml — skipped");
        return;
    }
    ctx.systems = read_toml_ordered(&path).unwrap_or_default();
    let ids: BTreeSet<String> = ctx.systems.iter().map(|(k, _)| k.clone()).collect();
    let tools: Vec<String> = ctx.mounts.iter().map(|(t, _)| t.clone()).collect();
    let (covered, uncovered) = pure::check_state_mount_coverage(&tools, &ids);
    for t in covered {
        ctx.ok(format!(
            "{t} — state_mount has a matching systems.toml identity"
        ));
    }
    for t in uncovered {
        ctx.bad(format!(
            "{t} — machine.toml [[state_mount]] with no systems.toml entry — undeclared system"
        ));
    }
    let local = ctx
        .systems
        .iter()
        .filter(|(_, v)| v.get("local").and_then(toml::Value::as_str) == Some("yes"))
        .count();
    let mounted = ctx
        .systems
        .iter()
        .filter(|(id, _)| tools.contains(id))
        .count();
    ctx.ok(format!(
        "{mounted}/{local} local=\"yes\" systems have a state_mount (rest are mount-less by design — tools/services with no persisted state)"
    ));
}

fn reachability(ctx: &mut Ctx) {
    if !ctx.online {
        ctx.ok("skipped — run 'tools/doctor --online' to probe declared endpoints");
        return;
    }
    if ctx.systems.is_empty() {
        ctx.warn("no systems.toml entries — nothing to probe");
        return;
    }
    let mut overlay_systems = toml::Table::new();
    if !ctx.overlay_path.is_empty() {
        let p = overlay_dir(ctx).join("config/systems.local.toml");
        if p.exists() {
            match read_toml(&p) {
                Ok(t) => overlay_systems = t,
                Err(_) => ctx.warn(
                    "overlay systems.local.toml is unreadable — private endpoints not resolved",
                ),
            }
        }
    }
    let targets = pure::resolve_probe_targets(&ctx.systems, &overlay_systems);
    let probed: Vec<(String, String, f64)> = targets
        .iter()
        .filter_map(|t| match t {
            Target::Probe {
                id,
                url,
                timeout_ms,
            } => Some((id.clone(), url.clone(), *timeout_ms)),
            Target::Skip { .. } => None,
        })
        .collect();
    // All at once, as Promise.all ran them: one slow endpoint must not stretch the report.
    let results: Vec<(String, Result<String, Outcome>, f64)> = std::thread::scope(|s| {
        let handles: Vec<_> = probed
            .iter()
            .map(|(id, url, t)| {
                s.spawn(move || {
                    let secs = format!("{:.3}", t / 1000.0);
                    let o = cmd(
                        "curl",
                        &[
                            "-s",
                            "-I",
                            "-o",
                            "/dev/null",
                            "-w",
                            "%{http_code}",
                            "--max-time",
                            &secs,
                            url,
                        ],
                    );
                    let r = match o.code {
                        Some(0) => Ok(o.stdout.trim().to_owned()),
                        Some(c) => Err(pure::classify_probe_outcome(c)),
                        None => Err(Outcome::Unavailable),
                    };
                    (id.clone(), r, *t)
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    for (id, r, t) in results {
        match r {
            Ok(code) => ctx.ok(format!(
                "{id} — reachable (HTTP {})",
                code.trim_start_matches('0')
                    .parse::<u16>()
                    .map_or(code.clone(), |c| c.to_string())
            )),
            Err(Outcome::Refused) => ctx.bad(format!("{id} — connection refused")),
            Err(Outcome::Timeout) => ctx.warn(format!(
                "{id} — no answer within {}ms (overlay probe_timeout_ms raises it)",
                js_num(t)
            )),
            Err(Outcome::Unavailable) => ctx.bad(format!(
                "{id} — unavailable (no route, DNS failure, or TLS error)"
            )),
        }
    }
    for t in &targets {
        if let Target::Skip { id, why } = t {
            ctx.ok(format!("{id} — skipped: {why}"));
        }
    }
    if probed.is_empty() {
        ctx.warn("no probeable endpoint among the declared systems");
    }
}

fn tracked_files(root: &Path) -> Option<Vec<String>> {
    let o = capture(Command::new("git").arg("-C").arg(root).arg("ls-files"));
    o.success().then(|| {
        o.stdout
            .split('\n')
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

fn basename(p: &str) -> String {
    p.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_owned()
}

fn undeclared_connections(ctx: &mut Ctx) {
    let declared: BTreeSet<String> = ctx.systems.iter().map(|(k, _)| k.clone()).collect();
    let self_roots: Vec<String> = [ctx.root.display().to_string(), ctx.overlay_path.clone()]
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|p| basename(&p))
        .collect();
    let mut protected = BTreeSet::new();
    if !ctx.overlay_path.is_empty() {
        if let Ok(text) =
            std::fs::read_to_string(overlay_dir(ctx).join("config/protection-zones.toml"))
        {
            let re = Regex::new(r#""([^"]+)""#).expect("pattern");
            for c in re.captures_iter(&text) {
                let b = basename(&c[1]);
                if !b.is_empty() {
                    protected.insert(b);
                }
            }
        }
    }
    let Some(files) = tracked_files(&ctx.root) else {
        ctx.warn("git ls-files failed — skipping sweep");
        return;
    };
    // Insertion order, as the Map and Sets in doctor.ts kept it.
    let mut hits: Vec<(String, Vec<String>)> = Vec::new();
    for rel in files {
        let Ok(bytes) = std::fs::read(ctx.root.join(&rel)) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        if pure::is_sweep_exempt(&rel, &text) {
            continue;
        }
        for name in pure::extract_sibling_repo_refs(&text, &self_roots) {
            let slot = match hits.iter_mut().position(|(n, _)| *n == name) {
                Some(i) => &mut hits[i].1,
                None => {
                    hits.push((name, Vec::new()));
                    &mut hits.last_mut().expect("just pushed").1
                }
            };
            if !slot.contains(&rel) {
                slot.push(rel.clone());
            }
        }
    }
    if hits.is_empty() {
        ctx.ok("no hardcoded sibling-repo paths found outside tools/lib/paths.sh");
        return;
    }
    for (name, files) in hits {
        let slug = name.to_lowercase();
        if protected.contains(&name) {
            ctx.ok(format!("{name} — declared a protected path in the overlay's protection-zones.toml; a deny rule, not a connection"));
        } else if declared.contains(&slug) {
            ctx.warn(format!("{name} — hardcoded path in {} (declared in systems.toml as '{slug}', but bypasses paths.sh indirection)", files.join(", ")));
        } else {
            ctx.bad(format!("{name} — hardcoded path in {}, no matching systems.toml entry — undeclared connection", files.join(", ")));
        }
    }
}

fn bind_policy(ctx: &mut Ctx) {
    let roots = [
        (ctx.root.join("capabilities"), "capabilities"),
        (
            overlay_dir(ctx).join("capabilities"),
            "overlay capabilities",
        ),
    ];
    if !ctx.root.join("capabilities").exists() {
        ctx.warn("no capabilities/ dir");
        return;
    }
    let router = Regex::new(r"\bRouter::new\s*\(").expect("pattern");
    let serve = Regex::new(r"sjel_server::serve(_local)?\s*\(").expect("pattern");
    let (mut checked, mut offenders) = (0, 0);
    for (caps, label) in &roots {
        if !caps.exists() {
            continue;
        }
        for cap in sorted_dir_names(caps) {
            let src = caps.join(&cap).join("src");
            if !src.exists() {
                continue;
            }
            for path in pure::find_rust_sources(&src) {
                let rel = path
                    .strip_prefix(&src)
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                let text =
                    String::from_utf8_lossy(&std::fs::read(&path).unwrap_or_default()).into_owned();
                let production = pure::strip_rust_cfg_test_items(&text);
                if !router.is_match(&production) {
                    continue;
                }
                checked += 1;
                if pure::find_production_listener_constructs(&text).is_empty() {
                    if !serve.is_match(&production) {
                        ctx.warn(format!("{label}/{cap}/src/{rel} builds a Router but neither serves it nor uses sjel_server"));
                    }
                    continue;
                }
                offenders += 1;
                ctx.bad(format!("{label}/{cap}/src/{rel} binds its own listener — use sjel_server::serve_local (loopback + port contract)"));
            }
        }
    }
    if checked == 0 {
        ctx.bad("no capability server sources found — this check is looking in the wrong place");
    } else if offenders == 0 {
        ctx.ok(format!(
            "{checked} capability server(s) across both roots, none binds by hand"
        ));
    }
}

fn tailnet_gate(ctx: &mut Ctx) {
    if ctx.overlay_path.is_empty() || !exists(&ctx.overlay_path) {
        ctx.warn("no overlay — cannot read deployment.env");
        return;
    }
    let env_path = overlay_dir(ctx).join("config/deployment.env");
    if !env_path.exists() {
        ctx.ok("no deployment.env — no tailnet gate declared");
        return;
    }
    // Only the one key is read; deployment.env holds other values this check never touches.
    let declared = std::fs::read_to_string(&env_path)
        .unwrap_or_default()
        .split('\n')
        .map(str::trim)
        .find_map(|l| l.strip_prefix("SJEL_TAILNET_OPERATOR="))
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty());
    let Some(declared) = declared else {
        ctx.ok("no operator declared — the identity header is ignored, not trusted");
        return;
    };
    let serve = cmd("tailscale", &["serve", "status", "--json"]);
    if !serve.success() {
        ctx.bad("operator declared but 'tailscale serve status' failed — the gate depends on a proxy this machine cannot describe");
        return;
    }
    let body = if serve.stdout.is_empty() {
        "{}"
    } else {
        &serve.stdout
    };
    let Ok(config) = serde_json::from_str::<Json>(body) else {
        ctx.bad("operator declared but 'tailscale serve status --json' did not parse");
        return;
    };
    let tcp: Vec<String> = config
        .get("TCP")
        .and_then(Json::as_object)
        .map(|m| {
            m.iter()
                .filter(|(_, v)| !truthy(v.get("HTTPS")))
                .map(|(k, _)| k.clone())
                .collect()
        })
        .unwrap_or_default();
    let proxied = config
        .get("Web")
        .and_then(Json::as_object)
        .map(|hosts| {
            hosts
                .values()
                .filter_map(|h| h.get("Handlers").and_then(Json::as_object))
                .flat_map(|hs| hs.values())
                .filter(|h| h.get("Proxy").is_some_and(Json::is_string))
                .count()
        })
        .unwrap_or(0);
    if proxied == 0 {
        ctx.bad(format!(
            "operator '{declared}' is declared but 'tailscale serve' publishes no HTTPS web handler — nothing injects an identity header, so the gate admits every tailnet caller as loopback"
        ));
        return;
    }
    for port in &tcp {
        ctx.bad(format!("'tailscale serve' forwards raw TCP on {port} — a TCP forward injects no identity header, so the gate cannot see who is calling"));
    }
    if tcp.is_empty() {
        ctx.ok(format!("operator '{declared}', {proxied} HTTPS web handler(s) — identity is injected and overwritten by the proxy"));
    }
    if cmd("tailscale", &["funnel", "status"])
        .stdout
        .contains("Funnel on")
    {
        ctx.bad("'tailscale funnel' is on — PRD N3 refuses internet exposure; the identity gate covers the tailnet, not the public internet");
    }
}

fn vault_pointers(ctx: &mut Ctx) {
    let (db, _, _) = db_path(ctx);
    if !exists(&db) {
        ctx.warn("no database — nothing to resolve");
        return;
    }
    let config_dir = overlay_dir(ctx).join("config");
    let root_for = |config: &str| -> Option<String> {
        let text = std::fs::read_to_string(config_dir.join(config)).ok()?;
        let v: Json = serde_json::from_str(&text).ok()?;
        v.get("obsidian")?
            .get("root")?
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(expand_home)
    };
    let sources = [
        (
            "trips",
            "trips.json",
            "trips_plans",
            "source_ref",
            "source_ref",
            "source_kind = 'obsidian'",
        ),
        (
            "trips",
            "trips.json",
            "trips_plan_items",
            "payload.vault_path",
            "json_extract(payload,'$.vault_path')",
            "item_type = 'note'",
        ),
        (
            "finance",
            "finance.json",
            "finance_subscriptions",
            "source_path",
            "source_path",
            "1=1",
        ),
        (
            "scouting",
            "scouting.json",
            "scouting_opportunities",
            "vault_link",
            "vault_link",
            "1=1",
        ),
        (
            "scouting",
            "scouting.json",
            "scouting_links",
            "vault_path",
            "vault_path",
            "1=1",
        ),
    ];
    let (mut checked, mut dangling, mut skipped) = (0, 0, 0);
    for (cap, config, table, label, column, where_) in sources {
        let Some(root) = root_for(config) else {
            skipped += 1;
            continue;
        };
        let sql = format!("SELECT DISTINCT {column} FROM {table} WHERE {where_} AND {column} IS NOT NULL AND TRIM({column}) <> '';");
        let o = cmd("sqlite3", &[&format!("file:{db}?mode=ro"), &sql]);
        if !o.success() {
            continue;
        }
        let mut missing = Vec::new();
        for line in o.stdout.split('\n') {
            let rel = line.trim();
            if rel.is_empty() {
                continue;
            }
            checked += 1;
            if !Path::new(&root).join(rel).exists() {
                missing.push(rel.to_owned());
            }
        }
        if !missing.is_empty() {
            dangling += missing.len();
            let shown: Vec<String> = missing.iter().take(3).map(|m| format!("'{m}'")).collect();
            let rest = if missing.len() > 3 {
                format!(" (+{} more)", missing.len() - 3)
            } else {
                String::new()
            };
            ctx.warn(format!(
                "{cap}: {} of {table}.{label} point at nothing — {}{rest}",
                missing.len(),
                shown.join(", ")
            ));
        }
    }
    if skipped == sources.len() {
        ctx.warn("no capability declares a vault root — nothing to resolve");
    } else if checked == 0 {
        ctx.ok("no stored vault pointers on this machine");
    } else if dangling == 0 {
        ctx.ok(format!("{checked} stored vault pointer(s) resolve"));
    }
}

fn declared_number(v: &str) -> f64 {
    if v.trim().is_empty() {
        f64::NAN
    } else {
        js_number_str(v)
    }
}

fn data_freshness(ctx: &mut Ctx) {
    let Some(reg) = registry(ctx) else { return };
    let enabled = ctx.enabled();
    let declaring: Vec<&Json> = reg
        .iter()
        .filter(|s| !registry_field(s, "freshness_stale_hours").is_empty())
        .filter(|s| enabled.is_empty() || enabled.contains(&registry_field(s, "name")))
        .collect();
    if declaring.is_empty() {
        ctx.ok("no capability declares a freshness contract");
        return;
    }
    for s in declaring {
        let name = registry_field(s, "name");
        let advise = declared_number(&registry_field(s, "freshness_advise_hours"));
        let stale = declared_number(&registry_field(s, "freshness_stale_hours"));
        let port = registry_field(s, "port");
        if port.is_empty() {
            ctx.warn(format!(
                "{name} declares a freshness contract but has no port to ask"
            ));
            continue;
        }
        let url = format!("http://127.0.0.1:{port}/__axon/freshness");
        let o = cmd(
            "curl",
            &["-s", "--max-time", "4", "-w", "\n%{http_code}", &url],
        );
        if !o.success() {
            ctx.ok(format!("{name} — skipped, not running"));
            continue;
        }
        let (body, code) = o.stdout.rsplit_once('\n').unwrap_or(("", &o.stdout));
        let code: u16 = code.trim().parse().unwrap_or(0);
        if !(200..300).contains(&code) {
            ctx.ok(format!("{name} — skipped, not answering (HTTP {code})"));
            continue;
        }
        let Ok(v) = serde_json::from_str::<Json>(body) else {
            ctx.bad(format!("{name} — /__axon/freshness did not answer JSON"));
            continue;
        };
        let last = v.get("last_arrival_at").filter(|x| !x.is_null());
        let Some(last) = last else {
            ctx.bad(format!(
                "{name} — nothing has ever arrived, and a contract says something should"
            ));
            continue;
        };
        let hours = (now_secs() - last.as_f64().unwrap_or(f64::NAN)) / 3600.0;
        let age = if hours < 1.0 {
            format!("{}m", js_num(js_round(hours * 60.0)))
        } else {
            format!("{}h", to_fixed(hours, 1))
        };
        if stale.is_finite() && hours >= stale {
            ctx.bad(format!("{name} — nothing has arrived for {age} (stale past {}h); its producer is not running", js_num(stale)));
        } else if advise.is_finite() && hours >= advise {
            ctx.warn(format!(
                "{name} — last arrival {age} ago (due past {}h)",
                js_num(advise)
            ));
        } else {
            ctx.ok(format!("{name} — data arrived {age} ago"));
        }
    }
}

fn backups(ctx: &mut Ctx) {
    if ctx.overlay_path.is_empty() || !exists(&ctx.overlay_path) {
        ctx.warn("skipped — no overlay to read backup receipts from");
        return;
    }
    let Some(reg) = registry(ctx) else { return };
    let enabled = ctx.enabled();
    let contracts: Vec<&Json> = reg
        .iter()
        .filter(|s| {
            !registry_field(s, "backup_target").is_empty()
                && registry_field(s, "scope") != "external"
        })
        .filter(|s| enabled.is_empty() || enabled.contains(&registry_field(s, "name")))
        .collect();
    if contracts.is_empty() {
        ctx.ok("no capability on this machine declares a backup contract");
        return;
    }
    let mut targets = toml::Table::new();
    let systems_local = overlay_dir(ctx).join("config/systems.local.toml");
    if systems_local.exists() {
        match read_toml(&systems_local) {
            Ok(t) => targets = t,
            Err(_) => {
                ctx.warn("overlay systems.local.toml is unreadable — archives cannot be located")
            }
        }
    }
    let now = now_secs().floor();
    let receipts = overlay_dir(ctx).join("backup/receipts");
    for s in contracts {
        let name = registry_field(s, "name");
        let advise = declared_number(&registry_field(s, "backup_advise_days"));
        let stale = declared_number(&registry_field(s, "backup_stale_days"));
        let receipt_path = receipts.join(format!("{name}.json"));
        let mut receipt = Json::Null;
        if receipt_path.exists() {
            match std::fs::read_to_string(&receipt_path)
                .ok()
                .and_then(|t| serde_json::from_str::<Json>(&t).ok())
            {
                Some(r) => receipt = r,
                None => {
                    ctx.bad(format!(
                        "{name} — {} is not readable JSON; treat this contract as unverified",
                        receipt_path.display()
                    ));
                    continue;
                }
            }
        }
        let at = receipt
            .get("completed_at")
            .filter(|v| truthy(Some(v)))
            .and_then(Json::as_str)
            .and_then(pure::parse_receipt_timestamp);
        let age = at.map(|a| ((now as i64) - a).max(0));
        let state = pure::backup_age_state(age, advise, stale);
        let days = age.map_or("0".to_owned(), |a| to_fixed(a as f64 / 86_400.0, 1));

        let attempt_path = receipts.join("attempts").join(format!("{name}.json"));
        if attempt_path.exists() {
            match std::fs::read_to_string(&attempt_path)
                .ok()
                .and_then(|t| serde_json::from_str::<Json>(&t).ok())
            {
                Some(a) => match a.get("at_epoch").and_then(Json::as_f64) {
                    Some(at_epoch) => {
                        let exit = a.get("exit_code").and_then(Json::as_f64).unwrap_or(-1.0);
                        let detail = a.get("detail").and_then(Json::as_str).unwrap_or("");
                        ctx.bad(format!(
                            "{name} — {}",
                            pure::attempt_finding(exit, at_epoch, detail, now)
                        ));
                    }
                    None => ctx.warn(format!(
                        "{name} — {} has no timestamp; a failed attempt cannot be dated",
                        attempt_path.display()
                    )),
                },
                None => ctx.warn(format!(
                    "{name} — {} is not readable JSON; whether the last attempt failed is unknown",
                    attempt_path.display()
                )),
            }
        }

        match state {
            AgeState::Never => {
                ctx.bad(format!("{name} — declares a backup contract and has no usable receipt; nothing has ever landed (tools/backup.sh {name})"));
                continue;
            }
            AgeState::Overdue => ctx.bad(format!(
                "{name} — last backup {days}d ago, past its {}d stale threshold; the schedule that should refresh it is not working",
                js_num(stale)
            )),
            AgeState::Due => ctx.warn(format!("{name} — last backup {days}d ago (due past {}d)", js_num(advise))),
            AgeState::Unknown => ctx.warn(format!("{name} — last backup {days}d ago, and the manifest declares no cadence to judge that against")),
            AgeState::Ok => ctx.ok(format!("{name} — backed up {days}d ago")),
        }

        let target_id = jtext(&receipt, "target");
        let tarball = jtext(&receipt, "tarball");
        let bytes = receipt
            .get("bytes")
            .and_then(Json::as_f64)
            .unwrap_or(f64::NAN);
        if target_id.is_empty() || tarball.is_empty() || !bytes.is_finite() {
            ctx.warn(format!("{name} — its receipt names no target/tarball/bytes, so the archive cannot be verified"));
            continue;
        }
        let Some(target) = targets.get(&target_id).and_then(toml::Value::as_table) else {
            ctx.warn(format!(
                "{name} — receipt names target '{target_id}', which the overlay does not declare"
            ));
            continue;
        };
        let kind = target
            .get("kind")
            .and_then(toml::Value::as_str)
            .filter(|k| !k.is_empty())
            .unwrap_or("ssh");
        if kind != "local" {
            ctx.ok(format!("{name} — archive not verified: target '{target_id}' is kind={kind}, which needs ssh and an unlocked vault"));
            continue;
        }
        let raw = target
            .get("path")
            .and_then(toml::Value::as_str)
            .unwrap_or("");
        if raw.is_empty() {
            ctx.warn(format!(
                "{name} — target '{target_id}' is kind=local and declares no path"
            ));
            continue;
        }
        let archive = Path::new(&expand_home(raw)).join(&name).join(&tarball);
        let meta = std::fs::metadata(&archive).ok();
        let flags = if meta.is_some() && cfg!(target_os = "macos") {
            let o = capture(Command::new("stat").arg("-f").arg("%Sf").arg(&archive));
            if o.success() {
                o.stdout.trim().to_owned()
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        let (level, detail) =
            pure::classify_archive_at_target(meta.is_some(), meta.map(|m| m.len()), &flags, bytes);
        emit(ctx, level, format!("{name} — {tarball}: {detail}"));
    }
}

fn port_uniqueness(ctx: &mut Ctx) {
    let mut claims: Vec<(String, Vec<String>)> = Vec::new();
    for (dir, label) in [
        (ctx.root.join("capabilities"), ""),
        (overlay_dir(ctx).join("capabilities"), "overlay:"),
    ] {
        if !dir.exists() {
            continue;
        }
        for cap in sorted_dir_names(&dir) {
            let svc = dir.join(&cap).join("service.toml");
            if !svc.exists() {
                continue;
            }
            let Ok(t) = read_toml(&svc) else { continue };
            for field in ["port", "panel_port"] {
                let value = t
                    .get(field)
                    .map(js_value)
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if value.is_empty() {
                    continue;
                }
                let owner = format!("{label}{cap}");
                match claims.iter_mut().find(|(p, _)| *p == value) {
                    Some((_, owners)) => {
                        if !owners.contains(&owner) {
                            owners.push(owner);
                        }
                    }
                    None => claims.push((value, vec![owner])),
                }
            }
        }
    }
    let collisions: Vec<&(String, Vec<String>)> =
        claims.iter().filter(|(_, o)| o.len() > 1).collect();
    if collisions.is_empty() {
        ctx.ok(format!(
            "{} declared port(s) across both roots, each claimed by one capability",
            claims.len()
        ));
        return;
    }
    let lines: Vec<String> = collisions
        .iter()
        .map(|(port, owners)| {
            format!(
                "port {port} is declared by {} — whichever starts second cannot bind",
                owners.join(" and ")
            )
        })
        .collect();
    for l in lines {
        ctx.bad(l);
    }
}

/// Pack deployment state, read in-process from tools/sjel-cli/src/harnesses/.
///
/// The ledger and hashing logic is the same Rust reader `tools/harnesses status` uses, so the
/// doctor's verdict and the tool's matrix cannot disagree. The tools/doctor-packs.ts sidecar
/// existed from 2026-10-02 to the next commit, until this port landed.
fn pack_sections(ctx: &mut Ctx) {
    match pack_state(&ctx.root) {
        Ok(sections) => {
            for (name, lines) in sections {
                section(ctx, &name, |c| {
                    for (level, message) in lines {
                        emit(c, level, message);
                    }
                });
            }
        }
        Err(e) => section(ctx, "Packs", |c| {
            c.bad(format!("Pack state unreadable: {e}"))
        }),
    }
}

/// One named section and its verdict lines, as the TypeScript sidecar handed them over.
type PackSection = (String, Vec<(Level, String)>);

fn pack_state(root: &Path) -> Result<Vec<PackSection>, String> {
    let registry = Registry::new(root);
    let mut sections = Vec::new();

    // One section per INSTALLED harness: which harnesses are here is the registry's answer,
    // never a stale list (the 2026-09-07 scar — three Packs sat for an uninstalled Codex while
    // the installed pi got zero rows).
    for harness in &registry.harnesses {
        if !is_installed(harness) {
            continue;
        }
        let mut lines: Vec<(Level, String)> = Vec::new();
        match statuses_for(harness, None) {
            Ok(rows) if rows.is_empty() => {
                lines.push((Level::Warn, "no Packs/*/pack.toml found".to_owned()));
            }
            Ok(rows) => {
                let mut unselected: Vec<String> = Vec::new();
                for row in &rows {
                    let label = format!("{}/{}", row.pack, row.skill);
                    let detail = row
                        .detail
                        .as_ref()
                        .map(|d| format!(" — {d}"))
                        .unwrap_or_default();
                    let (deploy, sync) = pack_commands(harness.id, &row.pack);
                    match row.status {
                        SkillStatus::Current => lines.push((Level::Ok, format!("{label} current"))),
                        SkillStatus::NotDeployed => {
                            if harness.model == Model::Registry {
                                unselected.push(row.pack.clone());
                            } else {
                                lines.push((
                                    Level::Warn,
                                    format!("{label} not deployed ({deploy})"),
                                ));
                            }
                        }
                        SkillStatus::Discovered => lines.push((
                            Level::Ok,
                            format!("{label} loaded by pi via discovery, not the ledger{detail}"),
                        )),
                        SkillStatus::Outdated => {
                            lines.push((Level::Warn, format!("{label} outdated ({sync}){detail}")))
                        }
                        SkillStatus::Drifted => lines.push((
                            Level::Bad,
                            format!(
                                "{label} has destination-side changes; sync/remove will refuse"
                            ),
                        )),
                        SkillStatus::MigrationRequired => {
                            let hint = if harness.id == "codex" {
                                format!(
                                    " (tools/packs-codex migrate-generated {} --accept-current)",
                                    row.pack
                                )
                            } else {
                                String::new()
                            };
                            lines.push((
                                Level::Warn,
                                format!(
                                    "{label} needs generated-artifact ledger migration{hint}{detail}"
                                ),
                            ));
                        }
                        SkillStatus::Missing => lines.push((
                            Level::Bad,
                            format!("{label} is ledger-owned but missing{detail}"),
                        )),
                        SkillStatus::Collision => {
                            let hint = if harness.id == "claude" {
                                format!(
                                    " (tools/packs-claude adopt {} if it is identical)",
                                    row.pack
                                )
                            } else {
                                String::new()
                            };
                            lines.push((
                                Level::Bad,
                                format!(
                                    "{label} destination is occupied by an unowned skill{hint}"
                                ),
                            ));
                        }
                        SkillStatus::Invalid => {
                            lines.push((Level::Bad, format!("{label} invalid{detail}")))
                        }
                    }
                }
                if harness.model == Model::Registry && !unselected.is_empty() {
                    let packs: BTreeSet<&String> = unselected.iter().collect();
                    let joined = packs
                        .iter()
                        .map(|p| p.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    lines.push((
                        Level::Ok,
                        format!(
                            "{} Pack(s) not selected for pi: {joined} (selection is the design — profiles decide)",
                            packs.len()
                        ),
                    ));
                }
            }
            Err(e) => lines.push((
                Level::Bad,
                format!("{} Pack state unreadable: {e}", harness.label),
            )),
        }
        sections.push((format!("Packs ({} deployed)", harness.label), lines));
    }

    // One warning per ABSENT harness that still holds deployed units.
    for harness in &registry.harnesses {
        if is_installed(harness) || harness.model != Model::Materialized {
            continue;
        }
        let deployed: Vec<_> = statuses_for(harness, None)?
            .into_iter()
            .filter(|row| row.status != SkillStatus::NotDeployed)
            .collect();
        if deployed.is_empty() {
            continue;
        }
        let packs: BTreeSet<String> = deployed.iter().map(|r| r.pack.clone()).collect();
        let joined = packs.iter().cloned().collect::<Vec<_>>().join(" ");
        let pi_reads = is_installed(registry.by_id("pi")?)
            && harness.config.destination == PathBuf::from(home()).join(".agents").join("skills");
        let base = format!(
            "{} units from {} Pack(s) sit at {} for a {} that is not installed",
            deployed.len(),
            packs.len(),
            harness.config.destination.display(),
            harness.label
        );
        let message = if pi_reads {
            format!(
                "{base} — pi IS installed and discovers that directory, so pi is loading these right now. \
                 Keep them in pi (tools/packs-pi deploy {joined}) before removing; otherwise they leave both harnesses."
            )
        } else {
            format!(
                "{base}; nothing reads them. Remove: {} remove {joined}",
                harness.cli
            )
        };
        sections.push((
            format!("Packs ({} NOT installed)", harness.label),
            vec![(Level::Warn, message)],
        ));
    }

    Ok(sections)
}

/// The deploy and sync commands an operator is told to run for one harness.
fn pack_commands(id: &str, pack: &str) -> (String, String) {
    match id {
        "claude" => (
            format!("tools/packs.sh link {pack}"),
            format!("tools/packs-claude sync {pack}"),
        ),
        "codex" => (
            format!("tools/packs-codex deploy {pack}"),
            format!("tools/packs-codex sync {pack}"),
        ),
        "opencode" => (
            format!("tools/packs-opencode deploy {pack}"),
            format!("tools/packs-opencode sync {pack}"),
        ),
        _ => (
            format!("tools/packs-pi deploy {pack}"),
            format!("tools/packs-pi sync {pack}"),
        ),
    }
}

fn doctrine_freshness(ctx: &mut Ctx) {
    let tracked: Vec<String> = git(&ctx.root, &["ls-files"])
        .split('\n')
        .map(str::to_owned)
        .collect();
    let mut blocks = Vec::new();
    for f in &tracked {
        if !f.ends_with(".md") || !ctx.root.join(f).exists() {
            continue;
        }
        match std::fs::read(ctx.root.join(f)) {
            Ok(b) => blocks.extend(pure::collect_why_blocks(f, &String::from_utf8_lossy(&b))),
            Err(e) => {
                ctx.bad(format!("doctrine sweep failed: {e}"));
                return;
            }
        }
    }
    let bases = pure::why_block_bases(&tracked);
    let overlay = std::env::var("SJEL_PERSONAL_ROOT").unwrap_or_default();
    let root = ctx.root.clone();
    let rot = pure::find_decision_path_rot(
        &blocks,
        &|p| root.join(p).exists() || (!overlay.is_empty() && Path::new(&overlay).join(p).exists()),
        &bases,
    );

    let test_file = Regex::new(r"\.(test|spec)\.[tj]s$").expect("pattern");
    let mut files = Vec::new();
    for f in tracked.iter().filter(|f| {
        !f.is_empty()
            && ctx.root.join(f).exists()
            && !test_file.is_match(f)
            && !f.ends_with("test.sh")
    }) {
        match std::fs::read(ctx.root.join(f)) {
            Ok(b) => files.push((f.clone(), String::from_utf8_lossy(&b).into_owned())),
            Err(e) => {
                ctx.bad(format!("doctrine sweep failed: {e}"));
                return;
            }
        }
    }
    let dangling = pure::find_dangling_decision_refs(&files, &|_| false);
    for r in &rot {
        if r.missing {
            ctx.bad(format!(
                "{} names {}, which no longer exists",
                r.slug, r.path
            ));
        } else {
            ctx.bad(format!(
                "{} declares {} absent, but it exists now",
                r.slug, r.path
            ));
        }
    }
    for (file, slug) in &dangling {
        ctx.bad(format!("{file} cites decisions/{slug}; that directory was dissolved (CONTRIBUTING.md#decisions-live-with-their-owner)"));
    }
    if rot.is_empty() && dangling.is_empty() {
        ctx.ok(format!(
            "{} why-blocks — every named path resolves, every asserted absence holds",
            blocks.len()
        ));
    } else {
        ctx.line("  → repair the paths if the reasoning still holds, delete it if it is spent, or mark the\n    absence deliberate with <!-- asserts-absent: <path> --> inside the block");
    }
}

/// Run a tools/ gate, print its output indented, and judge its exit status.
fn print_and_judge(ctx: &mut Ctx, name: &str, args: &[&str], level: Level, msg: &str) {
    let path = tool(ctx, name);
    if !path.exists() {
        ctx.warn(format!("missing {}", path.display()));
        return;
    }
    let o = cmd(&path, args);
    print_indented(ctx, &o);
    if !o.success() {
        emit(ctx, level, msg.to_owned());
    }
}

fn print_indented(ctx: &mut Ctx, o: &Out) {
    let out: Vec<&str> = [o.stdout.trim(), o.stderr.trim()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    for l in out.join("\n").split('\n').filter(|_| !out.is_empty()) {
        ctx.line(format!("  {l}"));
    }
}

fn service_manifests(ctx: &mut Ctx) {
    let o = capture(
        Command::new("bash")
            .arg(tool(ctx, "check-service-tomls.sh"))
            .env("SJEL_CHECK_OVERLAY", "1"),
    );
    let failures: Vec<String> = o
        .stderr
        .split('\n')
        .filter(|l| l.starts_with("FAIL"))
        .map(|l| l.strip_prefix("FAIL ").unwrap_or(l).to_owned())
        .collect();
    let n = failures.len();
    for f in failures {
        ctx.bad(f);
    }
    if o.code == Some(0) {
        ctx.ok("every manifest passes tools/check-service-tomls.sh, overlay included");
    } else if n == 0 {
        ctx.bad(format!(
            "tools/check-service-tomls.sh exited {}",
            o.code.map_or("null".to_owned(), |c| c.to_string())
        ));
    }
}

/// A capability's `<overlay>/data/<cap>/last.json` receipt, with its age in hours.
fn receipt(ctx: &mut Ctx, cap: &str) -> Option<(Json, f64)> {
    if !ctx.enabled().iter().any(|e| e == cap) {
        ctx.ok(format!(
            "{cap} not enabled on this machine — nothing to report"
        ));
        return None;
    }
    let path = overlay_dir(ctx).join("data").join(cap).join("last.json");
    if !path.exists() {
        ctx.warn(format!(
            "{cap} is enabled but has never written a receipt — it has not run"
        ));
        return None;
    }
    let Some(r) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Json>(&t).ok())
    else {
        ctx.bad(format!("<overlay>/data/{cap}/last.json is not valid JSON — the last run could not record what it did"));
        return None;
    };
    let at = r
        .get("at")
        .and_then(Json::as_str)
        .and_then(pure::parse_iso_ms);
    let age_h = at.map_or(f64::NAN, |ms| (now_secs() * 1000.0 - ms) / 3_600_000.0);
    if !age_h.is_finite() {
        ctx.bad(format!("the {cap} receipt carries no readable timestamp"));
        return None;
    }
    Some((r, age_h))
}

fn host_patch(ctx: &mut Ctx) {
    let Some((r, age)) = receipt(ctx, "host-patch") else {
        return;
    };
    let h = js_num(js_round(age));
    if age > 48.0 {
        ctx.warn(format!(
            "last patch run was {h}h ago — a 24h job that has not run in two days is not running"
        ));
    }
    let failed = truthy(r.get("failed"));
    if failed {
        ctx.warn(format!(
            "last patch run had failed steps:{}",
            jstr(r.get("failed"))
        ));
    }
    match r.get("audit").and_then(Json::as_str) {
        Some("finding") => ctx.bad("the last patch run's audit found something — run tools/audit"),
        Some("scanner-missing") => ctx.warn("the last patch run's audit could not run a scanner"),
        _ if age <= 48.0 && !failed => ctx.ok(format!("patched {h}h ago, audit clean")),
        _ => {}
    }
}

fn container_refresh(ctx: &mut Ctx) {
    let Some((r, age)) = receipt(ctx, "container-refresh") else {
        return;
    };
    let h = js_num(js_round(age));
    if age > 48.0 {
        ctx.warn(format!("last image refresh was {h}h ago — a 24h job that has not run in two days is not running"));
    }
    let skipped = match r.get("skipped") {
        None | Some(Json::Null) => String::new(),
        v => jstr(v),
    };
    if truthy(r.get("failed")) {
        ctx.warn(format!(
            "last image refresh had failed steps:{}",
            jstr(r.get("failed"))
        ));
    } else if age <= 48.0 && skipped.contains("no-container-capabilities") {
        ctx.ok(format!(
            "checked {h}h ago — no enabled capability declares an image"
        ));
    } else if age <= 48.0 {
        let ran = if truthy(r.get("ran")) {
            format!(" (recreated:{})", jstr(r.get("ran")))
        } else {
            ", none moved".to_owned()
        };
        ctx.ok(format!("images refreshed {h}h ago{ran}"));
    }
}

fn build_artifacts(ctx: &mut Ctx) {
    let launcher = ctx.root.join("tools/storage/storage");
    if !launcher.exists() {
        ctx.warn(format!("missing {}", launcher.display()));
        return;
    }
    let bin = cargo_target(ctx).join("release/sjel-storage");
    if !bin.exists() {
        ctx.warn("sjel-storage not built — run `sjel storage target` to check R6");
        return;
    }
    let o = cmd(&bin, &["target", "--json"]);
    let Ok(data) = serde_json::from_str::<Json>(&o.stdout) else {
        ctx.warn("sjel-storage target did not emit JSON — run `sjel storage target` for detail");
        return;
    };
    let gb = |b: f64| format!("{} GB", to_fixed(b / 1024f64.powi(3), 1));
    let num = |v: Option<&Json>| v.and_then(Json::as_f64).unwrap_or(0.0);
    let units = |name: &str| {
        data.get("profiles")
            .and_then(Json::as_array)
            .and_then(|ps| {
                ps.iter()
                    .find(|p| p.get("name").and_then(Json::as_str) == Some(name))
            })
            .and_then(|p| p.get("units"))
            .map_or("0".to_owned(), |u| jstr(Some(u)))
    };
    match data.get("ratio").filter(|r| !r.is_null()).and_then(Json::as_f64) {
        None => ctx.warn(format!("{} in {} — no release build, so R6 has no control", gb(num(data.get("bytes"))), jstr(data.get("target_dir")))),
        Some(ratio) if data.get("r6").and_then(Json::as_str) == Some("over") => ctx.warn(format!(
            "target/debug is {}× target/release, over R6's {}× ({} vs {} units) — sjel storage prune --incremental, or build both profiles and re-check",
            to_fixed(ratio, 1),
            jstr(data.get("r6_max_ratio")),
            units("debug"),
            units("release")
        )),
        Some(ratio) => ctx.ok(format!(
            "target/debug is {}× target/release, within R6's {}× ({} total)",
            to_fixed(ratio, 1),
            jstr(data.get("r6_max_ratio")),
            gb(num(data.get("bytes")))
        )),
    }
    let tc = data.get("toolchain").cloned().unwrap_or(Json::Null);
    let short = |k: &str| jstr(tc.get(k)).chars().take(9).collect::<String>();
    if tc.get("matches") == Some(&Json::Bool(false)) {
        ctx.warn(format!(
            "target/.rustc_info.json records rustc {} but this machine runs {} — {} of deps and fingerprints was built by a compiler that is gone; sjel storage prune --target",
            short("recorded"),
            short("current"),
            gb(num(tc.get("stale_candidate_bytes")))
        ));
    } else if truthy(tc.get("recorded")) {
        ctx.ok(format!(
            "cargo last recorded rustc {}, which is the one installed",
            short("recorded")
        ));
    }
}

fn ahead_behind(root: &Path) -> Option<(u64, u64)> {
    let o = capture(Command::new("git").arg("-C").arg(root).args([
        "rev-list",
        "--left-right",
        "--count",
        "HEAD...origin/main",
    ]));
    if !o.success() {
        return None;
    }
    let mut it = o
        .stdout
        .split_whitespace()
        .map(|s| s.parse::<u64>().unwrap_or(0));
    Some((it.next().unwrap_or(0), it.next().unwrap_or(0)))
}

fn repo_freshness(ctx: &mut Ctx) {
    if ctx.online {
        capture(
            Command::new("git")
                .arg("-C")
                .arg(&ctx.root)
                .args(["fetch", "--quiet", "origin", "main"]),
        );
    }
    match ahead_behind(&ctx.root) {
        None => ctx.warn("no origin/main ref cached — run 'tools/doctor --online' (or 'git fetch') to check freshness"),
        Some((0, 0)) => ctx.ok("up to date with origin/main"),
        Some((0, behind)) => ctx.warn(format!("{behind} commit(s) behind origin/main — run tools/update.sh")),
        Some((ahead, 0)) => ctx.ok(format!("{ahead} commit(s) ahead of origin/main — push when ready")),
        Some((ahead, behind)) => ctx.warn(format!("diverged from origin/main ({ahead} ahead, {behind} behind) — merge before tools/update.sh")),
    }
}

/// axon.toml `[release] tag_glob`, the pattern that decides which tags are releases
/// (tools/lib/release.ts).
fn release_tag_glob(root: &Path) -> String {
    read_toml(&root.join("axon.toml"))
        .ok()
        .and_then(|t| {
            t.get("release")?
                .get("tag_glob")?
                .as_str()
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

fn session_orientation(ctx: &mut Ctx) {
    let glob = release_tag_glob(&ctx.root);
    let v = pure::format_version(
        &git(
            &ctx.root,
            &[
                "describe", "--tags", "--always", "--dirty", "--match", &glob,
            ],
        ),
        &git(&ctx.root, &["log", "-1", "--format=%cs"]),
    );
    ctx.ok(format!("version: {v}"));
    let branch = capture(
        Command::new("git")
            .arg("-C")
            .arg(&ctx.root)
            .args(["branch", "--show-current"]),
    )
    .stdout
    .trim()
    .to_owned();
    let branch = if branch.is_empty() {
        "(detached HEAD)".to_owned()
    } else {
        branch
    };
    let head = capture(Command::new("git").arg("-C").arg(&ctx.root).args([
        "log",
        "-1",
        "--format=%h %s (%cr)",
    ]))
    .stdout
    .trim()
    .to_owned();
    ctx.ok(format!("{branch} @ {head}"));
    let dirty = capture(
        Command::new("git")
            .arg("-C")
            .arg(&ctx.root)
            .args(["status", "--porcelain"]),
    )
    .stdout
    .split('\n')
    .filter(|l| !l.is_empty())
    .count();
    if dirty == 0 {
        ctx.ok("working tree clean");
    } else {
        ctx.warn(format!(
            "{dirty} uncommitted change(s) — git status for detail"
        ));
    }
    let isas: Vec<String> = git(
        &ctx.root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "ISA.md",
            "*/ISA.md",
            "*/*/ISA.md",
            "*/*/*/ISA.md",
        ],
    )
    .split('\n')
    .filter(|l| !l.is_empty())
    .map(str::to_owned)
    .collect();
    let open: usize = isas
        .iter()
        .map(|f| {
            std::fs::read_to_string(ctx.root.join(f)).map_or(0, |t| {
                t.split('\n').filter(|l| l.starts_with("- [ ] ")).count()
            })
        })
        .sum();
    ctx.ok(format!(
        "open backlog: {open} claim(s) across {} ISA(s) · doctrine: CONTRIBUTING.md",
        isas.len()
    ));
}

/// `tools/doctor --version`: installed against origin/main, read-only.
pub fn print_version(root: &Path, online: bool) {
    println!("Axon doctor --version · {}", root.display());
    if online {
        capture(
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["fetch", "--quiet", "origin", "main"]),
        );
    }
    let glob = release_tag_glob(root);
    let describe = git(
        root,
        &[
            "describe", "--tags", "--always", "--dirty", "--match", &glob,
        ],
    );
    println!(
        "  installed: {}",
        pure::format_version(&describe, &git(root, &["log", "-1", "--format=%cs"]))
    );
    let numeric = Regex::new(r"^v?\d+(\.\d+)*$").expect("pattern");
    let latest = git(root, &["tag", "-l", &glob, "--sort=-v:refname"])
        .split('\n')
        .map(str::trim)
        .find(|t| numeric.is_match(t))
        .unwrap_or("")
        .to_owned();
    if !latest.is_empty() {
        println!(
            "  release:   {} — newest release tag",
            pure::format_version(&latest, &git(root, &["log", "-1", "--format=%cs", &latest]))
        );
    }
    let origin = git(root, &["rev-parse", "--short", "origin/main"]);
    if origin.is_empty() {
        println!("  latest:    unknown — no origin/main ref cached (run tools/doctor --version --online)");
        return;
    }
    let git_dir = Some(git(root, &["rev-parse", "--absolute-git-dir"]))
        .filter(|d| !d.is_empty())
        .map_or_else(|| root.join(".git"), PathBuf::from);
    let fetch = mtime_secs(git_dir.join("FETCH_HEAD")).map(|m| m.floor() as i64);
    let age = pure::format_fetch_age(fetch, now_secs().floor() as i64);
    let liveness = if online {
        ""
    } else {
        " (offline — run with --online for live)"
    };
    println!(
        "  latest:    {} — origin/main, {age}{liveness}",
        pure::format_version(
            &origin,
            &git(root, &["log", "-1", "--format=%cs", "origin/main"])
        )
    );
    match ahead_behind(root) {
        None => {}
        Some((0, 0)) => println!("  up to date with origin/main"),
        Some((0, b)) => println!("  {b} commit(s) behind origin/main — run tools/update.sh"),
        Some((a, 0)) => println!("  {a} commit(s) ahead of origin/main — push when ready"),
        Some((a, b)) => println!(
            "  diverged from origin/main ({a} ahead, {b} behind) — merge before tools/update.sh"
        ),
    }
}
