//! The report half of `tools/updates`: the surfaces, the gatherers, and the two renderers.
//!
//! `--json` is the stable surface and the CLI table is derived from the same rows, so a
//! dashboard panel renders the facts the terminal does instead of a second implementation of
//! them. `row.status` is one of current | stale | unknown | n/a and `row.owner` is one of
//! scheduled | manual | unowned | self; those two vocabularies are the whole contract.

use super::Ctx;
use crate::time;
use crate::updates::parse::*;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

// ── the surfaces and their owners ─────────────────────────────────────────────
// One row per class of installed software, not per binary. `owner` is the verdict this tool
// publishes, and it is the thing that was missing: a class nobody owns is not a bug in the
// class, it is a fact the operator is entitled to see stated once instead of discovered.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Owner {
    #[serde(rename = "scheduled")]
    Scheduled,
    #[serde(rename = "manual")]
    Manual,
    #[serde(rename = "unowned")]
    Unowned,
    #[serde(rename = "self")]
    SelfManaged,
}

impl Owner {
    pub fn as_str(self) -> &'static str {
        match self {
            Owner::Scheduled => "scheduled",
            Owner::Manual => "manual",
            Owner::Unowned => "unowned",
            Owner::SelfManaged => "self",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Status {
    #[serde(rename = "current")]
    Current,
    #[serde(rename = "stale")]
    Stale,
    #[serde(rename = "unknown")]
    Unknown,
    #[serde(rename = "n/a")]
    NotApplicable,
}

impl Status {
    fn mark(self) -> &'static str {
        match self {
            Status::Current => "✓",
            Status::Stale => "✗",
            Status::Unknown => "?",
            Status::NotApplicable => "·",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Surface {
    pub id: &'static str,
    pub title: &'static str,
    pub owner: Owner,
    #[serde(rename = "ownerDetail")]
    pub owner_detail: &'static str,
    /// True when `apply` moves this class, by delegation or directly.
    pub actionable: bool,
    pub why: &'static str,
}

pub static SURFACES: &[Surface] = &[
    Surface {
        id: "brew",
        title: "Homebrew formulae and casks",
        owner: Owner::Scheduled,
        owner_detail: "capabilities/host-patch · 24h",
        actionable: true,
        why: "brew owns its binaries and answers `brew outdated` itself; the nightly job moves them, including auto_updates casks via --greedy (Q77)",
    },
    Surface {
        id: "uv",
        title: "uv tools",
        owner: Owner::Scheduled,
        owner_detail: "capabilities/host-patch · 24h",
        actionable: true,
        why: "one upgrade step per tool, not --all, so one broken tool cannot leave every other one unpatched; uv has no 'what is new' verb, so this row cites the job's receipt",
    },
    Surface {
        id: "rustup",
        title: "rustup toolchain",
        owner: Owner::Scheduled,
        owner_detail: "capabilities/host-patch · 24h",
        actionable: true,
        why: "the toolchain, not the crates built with it — those are the `cargo` surface below",
    },
    Surface {
        id: "containers",
        title: "Container images",
        owner: Owner::Scheduled,
        owner_detail: "capabilities/container-refresh · 24h",
        actionable: false,
        why: "the declared tag is a channel and the digest is the fact (ISA.md C4); reported from its receipt and never pulled here",
    },
    Surface {
        id: "graphify",
        title: "graphify harness integration",
        owner: Owner::Manual,
        owner_detail: "tools/agent-integrations.sh update graphify",
        actionable: true,
        why: "the binary is a uv tool and moves nightly; the skill/plugin files upstream's installer wrote into each harness are re-derived only when this verb is run",
    },
    Surface {
        id: "interceptor",
        title: "interceptor CLI and skills",
        owner: Owner::Manual,
        owner_detail: "tools/agent-integrations.sh update interceptor",
        actionable: true,
        why: "the product owns its own updater (`interceptor upgrade`) and nothing schedules it; the skills it links into each harness are re-adopted beside it",
    },
    Surface {
        id: "checkout",
        title: "This checkout",
        owner: Owner::Manual,
        owner_detail: "tools/update.sh",
        actionable: false,
        why: "fast-forward only, and a diverged checkout is left alone with instructions rather than discarded",
    },
    Surface {
        id: "cargo",
        title: "cargo-installed binaries",
        owner: Owner::Unowned,
        owner_detail: "nothing — this tool moves them",
        actionable: true,
        why: "`cargo install` has no update verb and no register; toolchain.toml names the install path for [macmon] and nothing named an update path",
    },
    Surface {
        id: "npm",
        title: "npm global packages",
        owner: Owner::Unowned,
        owner_detail: "nothing — this tool moves them",
        actionable: true,
        why: "the harnesses self-update but the packages beside them do not, and nothing ran `npm outdated -g`",
    },
    Surface {
        id: "vendor",
        title: "Vendor-managed apps",
        owner: Owner::SelfManaged,
        owner_detail: "their own updater",
        actionable: false,
        why: "the harnesses and Ollama.app update themselves; a second updater for them would be a second owner",
    },
];

pub fn surface(id: &str) -> Option<&'static Surface> {
    SURFACES.iter().find(|s| s.id == id)
}

#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub surface: String,
    pub name: String,
    pub owner: Owner,
    #[serde(rename = "ownerDetail")]
    pub owner_detail: String,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    /// The command that moves it. The owner's entry point, never a reimplementation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// True only when this row is a leftover nothing requires: a deprecated package, or a
    /// duplicate whose parents bundle their own copy. It is exactly what `--prune` removes. A
    /// row PINNED by a parent is deliberately never marked — that copy is load-bearing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Row {
    fn new(surface_id: &str, status: Status) -> Row {
        let s = surface(surface_id).expect("a row names a known surface");
        Row {
            surface: s.id.to_owned(),
            name: String::new(),
            owner: s.owner,
            owner_detail: s.owner_detail.to_owned(),
            status,
            installed: None,
            latest: None,
            action: None,
            removable: None,
            note: None,
        }
    }

    fn name(mut self, n: impl Into<String>) -> Row {
        self.name = n.into();
        self
    }
    fn installed(mut self, v: impl Into<String>) -> Row {
        self.installed = Some(v.into());
        self
    }
    fn latest(mut self, v: impl Into<String>) -> Row {
        self.latest = Some(v.into());
        self
    }
    fn action(mut self, v: impl Into<String>) -> Row {
        self.action = Some(v.into());
        self
    }
    fn removable(mut self) -> Row {
        self.removable = Some(true);
        self
    }
    fn note(mut self, v: impl Into<String>) -> Row {
        self.note = Some(v.into());
        self
    }
}

// ── receipts ──────────────────────────────────────────────────────────────────

pub fn apply_receipt_path(overlay: &str) -> String {
    format!("{overlay}/data/updates/last-apply.json")
}

pub fn read_apply_receipt(overlay: &str) -> Option<ApplyReceipt> {
    let path = apply_receipt_path(overlay);
    let text = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_apply_receipt(overlay: &str, receipt: &ApplyReceipt) {
    // A receipt that cannot be written is not a reason to abandon an install that is already
    // running: the CLI prints its result either way, and only the panel loses the progress line.
    let _ = std::fs::create_dir_all(format!("{overlay}/data/updates"));
    if let Ok(text) = serde_json::to_string(receipt) {
        let _ = std::fs::write(apply_receipt_path(overlay), format!("{text}\n"));
    }
}

/// The scheduled job's own receipt — the only honest answer to "did the daily job run". A
/// launchd StartInterval unit does not fire while the Mac sleeps, so "scheduled" and "ran" are
/// different questions, and doctor asks the same one.
pub fn receipt_summary(path: &str) -> Option<ReceiptSummary> {
    if !Path::new(path).exists() {
        return None;
    }
    let r = parse_receipt(&std::fs::read_to_string(path).ok()?)?;
    let at = r.at.as_deref()?;
    let t = time::parse_iso_ms(at)?;
    Some(ReceiptSummary {
        age_h: (time::now_ms() - t) / 3_600_000.0,
        audit: r.audit.unwrap_or_else(|| "unknown".to_owned()),
        failed: r.failed.unwrap_or_default().trim().to_owned(),
        ran: r.ran.unwrap_or_default().trim().to_owned(),
    })
}

/// The job's own verdict, as a note rather than a status. The audit's verdict describes the whole
/// machine — secrets, config, source scans — and is reported by `tools/doctor`; folding it into
/// "brew is stale" would name the wrong thing and send the reader to the wrong command.
pub fn receipt_note(r: &ReceiptSummary) -> String {
    let mut bits = vec![if r.age_h < 24.0 {
        format!("job last ran {}h ago", r.age_h.round())
    } else {
        format!("job last ran {}d ago", (r.age_h / 24.0).round())
    }];
    if !r.failed.is_empty() {
        bits.push(format!("failed steps:{}", r.failed));
    }
    if r.audit != "clean" {
        bits.push(format!("audit {} — run tools/audit", r.audit));
    }
    bits.join(" · ")
}

pub fn receipt_is_stale(r: Option<&ReceiptSummary>) -> bool {
    match r {
        None => true,
        Some(r) => r.age_h > 48.0 || !r.failed.is_empty(),
    }
}

/// The audit that closes an apply. It runs on EVERY apply, including one whose plan already
/// delegated to host-patch — one extra read-only pass is cheaper than a field nobody can define.
/// Report-only: the caller's exit code stays a statement about the install.
pub fn run_audit(ctx: &Ctx) -> AuditVerdict {
    let script = format!("{}/tools/audit", ctx.root);
    audit_verdict(ctx.run(&[&script]).code)
}

// ── gatherers ─────────────────────────────────────────────────────────────────
// Each never throws: a missing binary or an unreachable registry becomes a row that says so,
// because a report that dies on the first absent tool answers no question at all.

/// Homebrew: the owner answers this itself, via its own `outdated`.
pub fn gather_brew(ctx: &Ctx, receipt: Option<&ReceiptSummary>) -> Vec<Row> {
    // The display form, not the argv form: a reader wants the command they would type.
    // `plan_apply` builds the absolute argv from `ctx.root`, so only one of the two has to be right.
    let action = "tools/host-patch.sh";
    let receipt_note = || match receipt {
        Some(r) => receipt_note(r),
        None => "no host-patch receipt — the job has never run".to_owned(),
    };
    if ctx.have("brew").is_none() {
        return vec![Row::new("brew", Status::NotApplicable)
            .name("brew")
            .note("brew not installed")];
    }
    if ctx.offline {
        return vec![Row::new("brew", Status::Unknown)
            .name("not checked (--offline)")
            .note(receipt_note())
            .action(action)];
    }
    let res = ctx.run(&["brew", "outdated", "--json=v2"]);
    if res.code != 0 {
        return vec![Row::new("brew", Status::Unknown)
            .name("brew outdated failed")
            .note(first_line(&res.stderr))];
    }
    let outdated = parse_brew_outdated(&res.stdout);
    if outdated.is_empty() {
        return vec![Row::new("brew", Status::Current)
            .name("all formulae and casks current")
            .note(receipt_note())];
    }
    outdated
        .into_iter()
        .map(|o| {
            Row::new("brew", Status::Stale)
                .name(o.name)
                .installed(o.installed)
                .latest(o.latest)
                .action(action)
                .note("moved by its owner, not by this tool")
        })
        .collect()
}

/// uv tools: no "what is new" verb exists, so the nightly job's receipt is the honest answer.
pub fn gather_uv(ctx: &Ctx, receipt: Option<&ReceiptSummary>) -> Vec<Row> {
    let action = "tools/host-patch.sh";
    if ctx.have("uv").is_none() {
        return vec![Row::new("uv", Status::NotApplicable)
            .name("uv")
            .note("uv not installed")];
    }
    vec![Row::new(
        "uv",
        if receipt_is_stale(receipt) {
            Status::Stale
        } else {
            Status::Current
        },
    )
    .name("tools")
    .note(match receipt {
        Some(r) => receipt_note(r),
        None => "no host-patch receipt — the job has never run".to_owned(),
    })
    .action(action)]
}

/// rustup: it answers for itself with `check`.
pub fn gather_rustup(ctx: &Ctx, receipt: Option<&ReceiptSummary>) -> Vec<Row> {
    let action = "tools/host-patch.sh";
    if ctx.have("rustup").is_none() {
        return vec![Row::new("rustup", Status::NotApplicable)
            .name("rustup")
            .note("rustup not installed")];
    }
    if ctx.offline {
        return vec![Row::new("rustup", Status::Unknown)
            .name("not checked (--offline)")
            .note(match receipt {
                Some(r) => receipt_note(r),
                None => "no host-patch receipt".to_owned(),
            })
            .action(action)];
    }
    let res = ctx.run(&["rustup", "check"]);
    let components = parse_rustup_check(&res.stdout);
    if res.code != 0 || components.is_empty() {
        return vec![Row::new("rustup", Status::Unknown)
            .name("rustup check failed")
            .note(first_line(&res.stderr))];
    }
    let behind: Vec<&RustupComponent> = components.iter().filter(|c| c.latest.is_some()).collect();
    if behind.is_empty() {
        let first = &components[0];
        return vec![Row::new("rustup", Status::Current)
            .name(&first.component)
            .installed(&first.installed)
            .note("up to date")];
    }
    behind
        .into_iter()
        .map(|c| {
            Row::new("rustup", Status::Stale)
                .name(&c.component)
                .installed(&c.installed)
                .latest(c.latest.clone().unwrap_or_default())
                .action(action)
        })
        .collect()
}

/// Containers: reported from container-refresh's receipt; never pulled here.
pub fn gather_containers(ctx: &Ctx) -> Vec<Row> {
    let path = format!("{}/data/container-refresh/last.json", ctx.overlay);
    if !Path::new(&path).exists() {
        return vec![Row::new("containers", Status::NotApplicable)
            .name("last run")
            .note("not enabled on this machine")];
    }
    let r = parse_receipt(&std::fs::read_to_string(&path).unwrap_or_default());
    let age_h = r
        .as_ref()
        .and_then(|r| r.at.as_deref())
        .and_then(time::parse_iso_ms)
        .map(|t| (time::now_ms() - t) / 3_600_000.0);
    let Some(age_h) = age_h else {
        return vec![Row::new("containers", Status::Unknown)
            .name("last run")
            .note("receipt unreadable")];
    };
    let failed = r
        .as_ref()
        .and_then(|r| r.failed.clone())
        .unwrap_or_default()
        .trim()
        .to_owned();
    vec![Row::new(
        "containers",
        if age_h > 48.0 || !failed.is_empty() {
            Status::Stale
        } else {
            Status::Current
        },
    )
    .name("last run")
    .note(format!(
        "ran {}h ago{}",
        age_h.round(),
        if failed.is_empty() {
            String::new()
        } else {
            format!(" · failed:{failed}")
        }
    ))]
}

/// graphify and interceptor integrations. Both markers record the DATE of the last
/// re-derivation, never a version (Q77 deleted every pin), so there is nothing to compare — the
/// row states the date it read. The harness config dirs come from agent-integrations' own JSON
/// rather than a table here: a second copy of that mapping would drift the day a harness is added.
pub fn gather_integrations(ctx: &Ctx) -> Vec<Row> {
    let script = format!("{}/tools/agent-integrations.sh", ctx.root);
    if !Path::new(&script).exists() {
        return vec![Row::new("graphify", Status::Unknown)
            .name("integration")
            .note("tools/agent-integrations.sh is missing")];
    }
    let res = ctx.run(&[&script, "status", "--json"]);
    if res.code != 0 {
        return vec![Row::new("graphify", Status::Unknown)
            .name("integration")
            .note("agent-integrations status failed")];
    }
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&res.stdout) else {
        return vec![Row::new("graphify", Status::Unknown)
            .name("integration")
            .note("agent-integrations emitted no JSON")];
    };

    let mut out = Vec::new();
    for id in ["graphify", "interceptor"] {
        let entry = payload
            .get("integrations")
            .and_then(serde_json::Value::as_array)
            .and_then(|list| {
                list.iter()
                    .find(|i| i.get("upstream").and_then(serde_json::Value::as_str) == Some(id))
            });
        let harnesses = entry
            .and_then(|e| e.get("harnesses"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        if harnesses.is_empty() {
            out.push(
                Row::new(id, Status::NotApplicable)
                    .name("integration")
                    .note("no harness reported"),
            );
            continue;
        }
        let marker_name = if id == "graphify" {
            ".graphify-upstream-installed"
        } else {
            ".interceptor-skills-axoned"
        };
        let mut oldest_days: Option<i64> = None;
        let mut oldest_date = String::new();
        for h in &harnesses {
            let Some(dir) = h.get("config_dir").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let marker = format!("{dir}/{marker_name}");
            if !Path::new(&marker).exists() {
                continue;
            }
            let date = std::fs::read_to_string(&marker)
                .unwrap_or_default()
                .trim()
                .to_owned();
            let Some(t) = time::parse_iso_ms(&date) else {
                continue;
            };
            let days = ((time::now_ms() - t) / 86_400_000.0).floor() as i64;
            if oldest_days.is_none_or(|d| days > d) {
                oldest_days = Some(days);
                oldest_date = date;
            }
        }
        let state = |h: &serde_json::Value| {
            h.get("state")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let integrated = harnesses
            .iter()
            .filter(|h| state(h) == "integrated")
            .count();
        let named: Vec<String> = harnesses
            .iter()
            .filter(|h| {
                let s = state(h);
                s != "missing" && s != "integrated"
            })
            .map(|h| {
                h.get("name")
                    .map(|n| n.as_str().unwrap_or(&n.to_string()).to_owned())
                    .unwrap_or_default()
            })
            .collect();
        let mut bits = vec![format!(
            "{integrated}/{} harness(es) integrated",
            harnesses.len()
        )];
        match oldest_days {
            Some(d) => bits.push(format!(
                "files re-derived from upstream {oldest_date} ({d}d)"
            )),
            None => bits.push("no marker — never derived from upstream".to_owned()),
        }
        if !named.is_empty() {
            bits.push(format!("not integrated: {}", named.join(", ")));
        }
        // 30 days, and it is a judgement rather than a measurement: with no version to compare
        // against, the row states the date so a reader can disagree with the threshold.
        let stale = oldest_days.is_none_or(|d| d > 30);
        out.push(
            Row::new(
                id,
                if stale {
                    Status::Stale
                } else {
                    Status::Current
                },
            )
            .name("harness integration")
            .note(bits.join(" · "))
            .action(format!("tools/agent-integrations.sh update {id}")),
        );
    }
    out
}

/// The checkout itself — `tools/update.sh --check` semantics, without pulling.
pub fn gather_checkout(ctx: &Ctx) -> Vec<Row> {
    if !Path::new(&format!("{}/tools/update.sh", ctx.root)).exists() {
        return vec![Row::new("checkout", Status::Unknown)
            .name("this repo")
            .note("tools/update.sh missing")];
    }
    let ahead = ctx.run(&[
        "git",
        "-C",
        &ctx.root,
        "rev-list",
        "--count",
        "origin/main..HEAD",
    ]);
    let behind = ctx.run(&[
        "git",
        "-C",
        &ctx.root,
        "rev-list",
        "--count",
        "HEAD..origin/main",
    ]);
    let a: Option<i64> = ahead.stdout.trim().parse().ok();
    let b: Option<i64> = behind.stdout.trim().parse().ok();
    let (Some(a), Some(b)) = (a, b) else {
        return vec![Row::new("checkout", Status::Unknown)
            .name("this repo")
            .note("no origin/main to compare against")];
    };
    let dirty = ctx.run(&["git", "-C", &ctx.root, "status", "--porcelain"]);
    let dirty_count = dirty
        .stdout
        .split('\n')
        .filter(|l| !l.trim().is_empty())
        .count();
    let mut bits = vec![format!("{a} ahead / {b} behind origin/main")];
    if dirty_count > 0 {
        bits.push(format!("{dirty_count} uncommitted change(s)"));
    }
    vec![Row::new(
        "checkout",
        if b > 0 {
            Status::Stale
        } else {
            Status::Current
        },
    )
    .name("this repo")
    .note(bits.join(" · "))
    .action("tools/update.sh")]
}

/// cargo: enumerated locally, newest release looked up one crate at a time.
pub fn gather_cargo(ctx: &Ctx) -> Vec<Row> {
    if ctx.have("cargo").is_none() {
        return vec![Row::new("cargo", Status::NotApplicable)
            .name("cargo")
            .note("cargo not installed")];
    }
    let installed = parse_cargo_install_list(&ctx.run(&["cargo", "install", "--list"]).stdout);
    if installed.is_empty() {
        return vec![Row::new("cargo", Status::Current)
            .name("crates")
            .note("nothing installed with `cargo install`")];
    }
    installed
        .into_iter()
        .map(|c| {
            if ctx.offline {
                return Row::new("cargo", Status::Unknown)
                    .name(&c.name)
                    .installed(&c.version)
                    .note("not checked (--offline)")
                    .action(format!("cargo install {} --locked --force", c.name));
            }

            // crates.io first, because only its full version list can say what the newest STABLE
            // is. `curl` rather than `fetch` so this stays inside the injected runner and the
            // tests can drive it. The User-Agent is required: crates.io answers 403 without one.
            let url = format!("https://crates.io/api/v1/crates/{}/versions", c.name);
            let api = ctx.run(&["curl", "-sS", "-A", "sjel-updates", &url]);
            let (stable, newest) = parse_crates_io_versions(&api.stdout);

            let mut latest = stable.clone();
            let mut pre_only: Option<String> = None;
            if latest.is_none() {
                if let Some(newest) = &newest {
                    // Only pre-releases are published, or the API answered but had no stable.
                    pre_only = Some(newest.clone());
                } else {
                    // No API answer at all: `cargo search` is the fallback, and it can only
                    // report the max.
                    let found = ctx.run(&["cargo", "search", &c.name, "--limit", "1"]);
                    let max = if found.code == 0 {
                        parse_cargo_search(&c.name, &found.stdout)
                    } else {
                        None
                    };
                    if let Some(max) = max {
                        if max.contains('-') {
                            pre_only = Some(max);
                        } else {
                            latest = Some(max);
                        }
                    }
                }
            }

            if latest.is_none() && pre_only.is_none() {
                return Row::new("cargo", Status::Unknown)
                    .name(&c.name)
                    .installed(&c.version)
                    .note("registry did not answer");
            }
            if latest.is_none() {
                // Nothing stable to move to. Named and left alone: adopting an alpha because it
                // sorts higher is exactly the decision a report must not make silently.
                let pre = pre_only.unwrap_or_default();
                return Row::new("cargo", Status::Current)
                    .name(&c.name)
                    .installed(&c.version)
                    .latest(&pre)
                    .note(format!("newer pre-release {pre} exists — not adopted"));
            }

            let latest = latest.unwrap();
            let stale = version_newer(&latest, &c.version);
            // A pre-release above the newest stable is worth naming even when there IS a stable
            // upgrade, so a reader can see the whole picture.
            let pre_note = match &newest {
                Some(n) if *n != latest => {
                    Some(format!("newer pre-release {n} exists — not adopted"))
                }
                _ => None,
            };
            let mut row = Row::new(
                "cargo",
                if stale {
                    Status::Stale
                } else {
                    Status::Current
                },
            )
            .name(&c.name)
            .installed(&c.version)
            .latest(&latest);
            if stale {
                row = row.action(format!("cargo install {} --locked --force", c.name));
            }
            if let Some(note) = pre_note {
                row = row.note(note);
            }
            row
        })
        .collect()
}

/// npm global packages. npm exits 1 when it has something to report, which is not an error.
pub fn gather_npm(ctx: &Ctx, brew_formulae: &std::collections::BTreeSet<String>) -> Vec<Row> {
    if ctx.have("npm").is_none() {
        return vec![Row::new("npm", Status::NotApplicable)
            .name("npm")
            .note("npm not installed")];
    }
    // One call for both facts: `--all` carries the subtree, which is what says whether a package
    // is independently upgradable.
    let tree = parse_npm_global_tree(&ctx.run(&["npm", "ls", "-g", "--json", "--all"]).stdout);
    if tree.installed.is_empty() {
        return vec![Row::new("npm", Status::Current)
            .name("packages")
            .note("no global packages")];
    }

    if ctx.offline {
        return tree
            .installed
            .iter()
            .map(|p| {
                Row::new("npm", Status::Unknown)
                    .name(&p.name)
                    .installed(&p.version)
                    .note("not checked (--offline)")
                    .action(format!("npm install -g {}@latest", p.name))
            })
            .collect();
    }
    let outdated = parse_npm_outdated(&ctx.run(&["npm", "outdated", "-g", "--json"]).stdout);
    let by_name: std::collections::BTreeMap<String, NpmOutdated> =
        outdated.into_iter().map(|o| (o.name.clone(), o)).collect();

    tree.installed
        .iter()
        .map(|p| {
            // The shadowing check: a global npm package whose name is also a brew formula can put
            // two binaries under one name on PATH. It is a hint to check, not a claim they are
            // the same project: npm's `uv` and brew's `uv` are unrelated packages.
            let shadow_note = brew_formulae
                .contains(&p.name)
                .then(|| "name is also a brew formula — check which one PATH resolves".to_owned());
            let reqs = tree.required_by.get(&p.name).cloned().unwrap_or_default();
            // A parent pins this copy only when it resolved the SAME version — the hoisted case.
            // A parent that resolved a different version bundled its own copy, which makes the
            // top-level one a leftover nothing requires.
            let pinned_by: Vec<String> = reqs
                .iter()
                .filter(|r| r.version == p.version)
                .map(|r| r.parent.clone())
                .collect();
            let bundled: Vec<&ParentVersion> =
                reqs.iter().filter(|r| r.version != p.version).collect();
            let Some(o) = by_name.get(&p.name) else {
                return match shadow_note {
                    Some(n) => Row::new("npm", Status::Current)
                        .name(&p.name)
                        .installed(&p.version)
                        .note(n),
                    None => Row::new("npm", Status::Current)
                        .name(&p.name)
                        .installed(&p.version),
                };
            };

            if !pinned_by.is_empty() {
                return Row::new("npm", Status::Stale)
                    .name(&p.name)
                    .installed(&o.current)
                    .latest(&o.latest)
                    .note(join_note(
                        &[format!(
                            "pinned by {} — upgrade those instead",
                            pinned_by.join(", ")
                        )],
                        shadow_note,
                    ));
            }

            // Deprecated: the registry itself says not to use this. Checked only for rows that
            // are otherwise actionable, so it costs one call per genuinely-outdated package.
            let deprecation =
                parse_npm_deprecated(&ctx.run(&["npm", "view", &p.name, "deprecated"]).stdout);
            if let Some(dep) = deprecation {
                return Row::new("npm", Status::Stale)
                    .name(&p.name)
                    .installed(&o.current)
                    .latest(&o.latest)
                    .removable()
                    .note(join_note(&[format!("deprecated — {dep}")], shadow_note));
            }

            // A leftover duplicate: nothing requires this copy at this version, and the parents
            // that mention the name bundle their own. Upgrading would install a third copy, so
            // there is no action — the honest advice is removal, the reader's call.
            if !bundled.is_empty() {
                let detail = bundled
                    .iter()
                    .map(|b| format!("{} bundles its own {}", b.parent, b.version))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Row::new("npm", Status::Stale)
                    .name(&p.name)
                    .installed(&o.current)
                    .latest(&o.latest)
                    .removable()
                    .note(join_note(
                        &[format!("unused duplicate — {detail}")],
                        shadow_note,
                    ));
            }

            let mut row = Row::new("npm", Status::Stale)
                .name(&p.name)
                .installed(&o.current)
                .latest(&o.latest)
                .action(format!("npm install -g {}@latest", p.name));
            if let Some(n) = shadow_note {
                row = row.note(n);
            }
            row
        })
        .collect()
}

/// The top-level global package a nested node belongs to, read from the `location` npm prints.
///
/// The path is `<global root>/<owner>/node_modules/<pkg>` for a direct dependency and deeper for
/// a nested one, so the owner is the OUTERMOST known top-level name bracketed by `node_modules`
/// — `min_by_key` on the index, because a package can sit under another package that is itself
/// a global. Matching against the known owners rather than splitting the path is what keeps a
/// scoped owner (`@scope/name`, two path segments) from being read as one.
fn npm_owner_of(location: &str, owners: &[String]) -> Option<String> {
    owners
        .iter()
        .filter_map(|o| {
            location
                .find(&format!("/node_modules/{o}/node_modules/"))
                .map(|i| (i, o.clone()))
        })
        .min_by_key(|(i, _)| *i)
        .map(|(_, o)| o)
}

/// The nested nodes of every global npm tree that are behind, as one row per owning package.
///
/// The rows above cannot see these. `npm outdated -g` asks about the top level, and a global
/// package at its latest release can still be carrying a dependency it resolved a year ago —
/// which is where every installed npm CVE on this machine lived on 2026-10-04, while the same
/// report called all eleven packages current.
///
/// It reports them as Current and never as Stale, deliberately, and that is the whole judgement
/// in this function. Every nested node of every global tree is behind *someone's* latest, because
/// a parent pins what it was published against; marking the owner stale would make the report
/// red forever on every machine that has a global install, and planning a reinstall every run.
/// A gate nobody can clear is a gate nobody reads — the same rule `tools/audit`'s installed pass
/// now follows. What was actually missing was not a flag but the command, and the note carries
/// it: `--re-resolve` names one of these owners and lands as a step.
fn npm_subtrees_behind(ctx: &Ctx, owners: &[String]) -> Vec<Row> {
    // A registry question, and an offline run asks none.
    if owners.is_empty() || ctx.offline || ctx.have("npm").is_none() {
        return Vec::new();
    }
    let nested = parse_npm_outdated_all(
        &ctx.run(&["npm", "outdated", "-g", "--all", "--json"])
            .stdout,
    );
    let mut by_owner: BTreeMap<String, Vec<NpmNestedOutdated>> = BTreeMap::new();
    for n in nested {
        // `npm outdated` lists a package whose installed version is AHEAD of the registry's
        // `latest` tag too — a major line the publisher has not tagged, or an alpha ahead of the
        // release. Without this filter the report says "accepts 2.0.0 → 1.3.8", which reads as a
        // downgrade nobody should make, and it is one of the first examples a reader would hit.
        // `version_newer` is the crate's own comparison and treats a pre-release as older than
        // its release, which is the same rule the cargo surface already uses.
        if !version_newer(&n.latest, &n.current) {
            continue;
        }
        if let Some(owner) = npm_owner_of(&n.location, owners) {
            by_owner.entry(owner).or_default().push(n);
        }
    }
    by_owner
        .into_iter()
        .map(|(owner, behind)| {
            // Sorted so the example named is the same on two runs of the same machine.
            let mut examples: Vec<&NpmNestedOutdated> = behind.iter().collect();
            examples.sort_by(|a, b| a.name.cmp(&b.name));
            let first = examples[0];
            Row::new("npm", Status::Current)
                .name(owner.clone())
                .note(format!(
                    "{} nested package(s) behind latest ({} {} → {}) — `sjel update apply --only npm --re-resolve {owner}` re-resolves its tree",
                    behind.len(),
                    first.name,
                    first.current,
                    first.latest
                ))
                .action(format!("npm install -g {owner}@latest"))
        })
        .collect()
}

/// Apps that update themselves. Named so the table is complete rather than silently partial.
pub fn gather_vendor(ctx: &Ctx) -> Vec<Row> {
    let mut out = Vec::new();
    for (bin, label) in [
        ("pi", "pi coding agent"),
        ("claude", "Claude Code"),
        ("codex", "Codex"),
        ("opencode", "opencode"),
    ] {
        if ctx.have(bin).is_some() {
            out.push(
                Row::new("vendor", Status::NotApplicable)
                    .name(label)
                    .note("self-updating"),
            );
        }
    }
    if let Some(ollama) = ctx.have("ollama") {
        // Ollama.app installs to /usr/local/bin and is neither a brew formula nor a cask on this
        // host (verified 2026-10-01), so the nightly sweep does not reach it.
        out.push(
            Row::new("vendor", Status::NotApplicable)
                .name("ollama")
                .note(if ollama.contains(".app/") {
                    "app-managed (/Applications/Ollama.app)".to_owned()
                } else {
                    format!("at {ollama} — not brew-managed; update through the app")
                }),
        );
    }
    if out.is_empty() {
        out.push(
            Row::new("vendor", Status::NotApplicable)
                .name("apps")
                .note("none found"),
        );
    }
    out
}

/// The first non-empty line of a stream, or a fixed sentence when there is none.
fn first_line(text: &str) -> String {
    let line = text.trim().split('\n').next().unwrap_or("").trim();
    if line.is_empty() {
        "no output".to_owned()
    } else {
        line.to_owned()
    }
}

fn join_note(bits: &[String], extra: Option<String>) -> String {
    let mut all: Vec<String> = bits.to_vec();
    if let Some(e) = extra {
        all.push(e);
    }
    all.join(" · ")
}

// ── report ────────────────────────────────────────────────────────────────────

pub struct Report {
    pub rows: Vec<Row>,
    pub generated_at: String,
    pub last_apply: Option<ApplyReceipt>,
}

pub fn build_report(ctx: &Ctx) -> Report {
    let receipt = receipt_summary(&format!("{}/data/host-patch/last.json", ctx.overlay));
    let brew_formulae = if ctx.have("brew").is_some() && !ctx.offline {
        parse_brew_formulae(&ctx.run(&["brew", "list", "--formula"]).stdout)
    } else {
        Default::default()
    };
    let mut rows = Vec::new();
    rows.extend(gather_brew(ctx, receipt.as_ref()));
    rows.extend(gather_uv(ctx, receipt.as_ref()));
    rows.extend(gather_rustup(ctx, receipt.as_ref()));
    rows.extend(gather_containers(ctx));
    rows.extend(gather_integrations(ctx));
    rows.extend(gather_checkout(ctx));
    rows.extend(gather_cargo(ctx));
    let npm_rows = gather_npm(ctx, &brew_formulae);
    // The owners the subtree scan attributes to. Taken from the rows just gathered rather than
    // asked of npm again, so the two halves cannot disagree about what is installed at the top.
    let npm_owners: Vec<String> = npm_rows.iter().map(|r| r.name.clone()).collect();
    rows.extend(npm_rows);
    rows.extend(npm_subtrees_behind(ctx, &npm_owners));
    rows.extend(gather_vendor(ctx));
    Report {
        rows,
        generated_at: time::now_iso(),
        last_apply: read_apply_receipt(&ctx.overlay),
    }
}

pub fn grouped(rows: &[Row]) -> Vec<(&'static Surface, Vec<&Row>)> {
    SURFACES
        .iter()
        .map(|s| (s, rows.iter().filter(|r| r.surface == s.id).collect()))
        .filter(|(_, rs): &(&Surface, Vec<&Row>)| !rs.is_empty())
        .collect()
}

fn heading(owner: Owner) -> &'static str {
    match owner {
        Owner::Scheduled => "Managed by a scheduled job",
        Owner::Manual => "Manual — a verb exists, nothing schedules it",
        Owner::Unowned => "Unowned — nothing moves these",
        Owner::SelfManaged => "Self-managed — the vendor updates these",
    }
}

pub fn render_table(rows: &[Row], offline: bool, last_apply: Option<&ApplyReceipt>) -> String {
    let mut out = vec![format!(
        "sjel update — software installed outside this checkout{}",
        if offline { " (offline)" } else { "" }
    )];
    out.push(String::new());
    let mut last_owner: Option<Owner> = None;
    for (s, rs) in grouped(rows) {
        if last_owner != Some(s.owner) {
            out.push(heading(s.owner).to_owned());
            last_owner = Some(s.owner);
        }
        out.push(format!("  {}  [{}]", s.title, s.owner_detail));
        for r in rs {
            let name = if r.name.is_empty() {
                String::new()
            } else {
                format!(" {}", r.name)
            };
            let vers = match &r.installed {
                Some(i) => {
                    let arrow = match &r.latest {
                        Some(l) if *l != *i => format!(" → {l}"),
                        _ => String::new(),
                    };
                    format!(" {i}{arrow}")
                }
                None => String::new(),
            };
            let note = match &r.note {
                Some(n) => format!("  {n}"),
                None => String::new(),
            };
            out.push(format!("    {}{name}{vers}{note}", r.status.mark()));
            if r.status == Status::Stale {
                if let Some(a) = &r.action {
                    out.push(format!("      → {a}"));
                }
            }
        }
    }
    let stale: Vec<&Row> = rows.iter().filter(|r| r.status == Status::Stale).collect();
    let unknown: Vec<&Row> = rows
        .iter()
        .filter(|r| r.status == Status::Unknown)
        .collect();
    out.push(String::new());
    if let Some(la) = last_apply {
        if la.state.as_deref() == Some("running") {
            let started = la
                .at
                .as_deref()
                .and_then(time::parse_iso_ms)
                .map(|t| (time::now_ms() - t) / 60_000.0);
            out.push(format!(
                "an apply is running: {} started {}m ago",
                la.class.clone().unwrap_or_default(),
                match started {
                    Some(m) => format!("{}", m.round()),
                    None => "NaN".to_owned(),
                }
            ));
        } else if la.at.is_some() {
            out.push(format!(
                "last apply: {} {} · {} step(s){}{}",
                la.class.clone().unwrap_or_default(),
                la.state.clone().unwrap_or_else(|| "done".to_owned()),
                la.steps.unwrap_or(0),
                match la.failed {
                    Some(f) if f > 0 => format!(", {f} failed"),
                    _ => String::new(),
                },
                match &la.audit {
                    Some(a) => format!(" · audit {a}"),
                    None => String::new(),
                }
            ));
        }
    }
    if stale.is_empty() {
        out.push(if unknown.is_empty() {
            "nothing stale".to_owned()
        } else {
            format!("nothing stale · {} not checked", unknown.len())
        });
    } else {
        let mut owners: Vec<&str> = stale.iter().map(|r| r.owner.as_str()).collect();
        owners.sort_unstable();
        owners.dedup();
        out.push(format!(
            "{} stale ({}) · 'sjel update apply' moves what this tool owns",
            stale.len(),
            owners.join(", ")
        ));
    }
    out.join("\n")
}

#[derive(Serialize)]
struct Payload<'a> {
    #[serde(rename = "generatedAt")]
    generated_at: &'a str,
    offline: bool,
    #[serde(rename = "lastApply")]
    last_apply: Option<&'a ApplyReceipt>,
    surfaces: &'a [Surface],
    rows: &'a [Row],
    // Present only when it was asked for: it is two orders of magnitude larger than `rows`, and
    // the dashboard reads this payload every four seconds while an apply runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    inventory: Option<&'a [InventoryEntry]>,
}

pub fn render_json(
    rows: &[Row],
    generated_at: &str,
    offline: bool,
    last_apply: Option<&ApplyReceipt>,
    inventory: Option<&[InventoryEntry]>,
) -> String {
    let payload = Payload {
        generated_at,
        offline,
        last_apply,
        surfaces: SURFACES,
        rows,
        inventory,
    };
    serde_json::to_string_pretty(&payload).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

/// One installed thing, as `tools/audit` needs it. `ecosystem` is the OSV ecosystem string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InventoryEntry {
    pub ecosystem: &'static str,
    pub name: String,
    pub version: String,
}

/// Everything installed outside this checkout, as name+version pairs — the input `tools/audit`
/// scans for CVEs. Deliberately NOT `rows`: a row is the actionable view and for npm that is the
/// top level only, and a CVE does not stop at the top level.
pub fn build_inventory(ctx: &Ctx) -> Vec<InventoryEntry> {
    let mut out = Vec::new();
    if ctx.have("npm").is_some() {
        let tree = parse_npm_global_tree(&ctx.run(&["npm", "ls", "-g", "--json", "--all"]).stdout);
        for p in tree.tree {
            out.push(InventoryEntry {
                ecosystem: "npm",
                name: p.name,
                version: p.version,
            });
        }
    }
    if ctx.have("cargo").is_some() {
        for c in parse_cargo_install_list(&ctx.run(&["cargo", "install", "--list"]).stdout) {
            out.push(InventoryEntry {
                ecosystem: "crates.io",
                name: c.name,
                version: c.version,
            });
        }
    }
    out
}

// ── apply planning ────────────────────────────────────────────────────────────
// Split by owner on purpose. `delegated` execs the tools that already own a class; `direct` runs
// the steps for the two classes nothing owns. Keeping the lists apart is what stops this file
// growing a second `brew upgrade`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub surface_id: String,
    pub label: String,
    pub argv: Vec<String>,
    pub slow: bool,
    pub note: Option<String>,
}

/// Rows `--prune` removes: leftovers nothing requires, and only those. Narrow on purpose, and
/// the narrowness is the safety property. A row a parent PINS is never here, whatever its
/// status — that copy exists to satisfy someone, and deleting it breaks its parent.
pub fn plan_prune(rows: &[Row]) -> Vec<&Row> {
    rows.iter().filter(|r| r.removable == Some(true)).collect()
}

#[derive(Debug, Default, Clone)]
pub struct PlanOpts {
    pub prune: bool,
    pub re_resolve: Vec<String>,
}

pub fn plan_apply(rows: &[Row], only: &[String], ctx: &Ctx, opts: &PlanOpts) -> Vec<Step> {
    let stale: Vec<&Row> = rows.iter().filter(|r| r.status == Status::Stale).collect();
    let wanted = |id: &str| only.is_empty() || only.iter().any(|o| o == id);
    let mut steps: Vec<Step> = Vec::new();

    // Removals first: a leftover going out frees the name before anything new arrives under it.
    if opts.prune && wanted("npm") {
        for r in plan_prune(rows) {
            steps.push(Step {
                surface_id: "npm".to_owned(),
                label: format!("prune: {}", r.name),
                argv: vec![
                    "npm".into(),
                    "uninstall".into(),
                    "-g".into(),
                    r.name.clone(),
                ],
                slow: false,
                note: r.note.clone(),
            });
        }
    }

    // Delegated. The host-patch job is one job covering three managers, so one step runs it and
    // one step calls the owner, not three.
    let host_managed = ["brew", "uv", "rustup"]
        .iter()
        .any(|id| wanted(id) && stale.iter().any(|r| r.surface == *id));
    if host_managed {
        steps.push(Step {
            surface_id: "hostpatch".to_owned(),
            label: "brew, uv and rustup — via their owner (capabilities/host-patch)".to_owned(),
            argv: vec![format!("{}/tools/host-patch.sh", ctx.root)],
            slow: true,
            note: None,
        });
    }
    for id in ["graphify", "interceptor"] {
        if !wanted(id) || !stale.iter().any(|r| r.surface == id) {
            continue;
        }
        steps.push(Step {
            surface_id: id.to_owned(),
            label: format!("{id} harness integration"),
            argv: vec![
                format!("{}/tools/agent-integrations.sh", ctx.root),
                "update".to_owned(),
                id.to_owned(),
            ],
            slow: false,
            note: None,
        });
    }

    // Direct: the two classes nothing else owns.
    for r in &stale {
        if r.surface == "cargo" && wanted("cargo") {
            if let Some(action) = &r.action {
                // `--locked` is the default because the published lockfile is what makes an
                // install reproducible. The one case where that costs more than it buys is a
                // crate whose published lockfile ALREADY pins a flagged dependency.
                let dropping = opts.re_resolve.iter().any(|n| n == &r.name);
                steps.push(Step {
                    surface_id: "cargo".to_owned(),
                    label: format!("cargo: {}", r.name),
                    argv: if dropping {
                        vec!["cargo".into(), "install".into(), r.name.clone(), "--force".into()]
                    } else {
                        action.split(' ').map(str::to_owned).collect()
                    },
                    slow: true,
                    note: dropping.then(|| {
                        "re-resolving — --locked dropped, its published lockfile pins a flagged dependency"
                            .to_owned()
                    }),
                });
            }
        }
        if r.surface == "npm" && wanted("npm") {
            if let Some(action) = &r.action {
                steps.push(Step {
                    surface_id: "npm".to_owned(),
                    label: format!("npm: {}", r.name),
                    argv: action.split(' ').map(str::to_owned).collect(),
                    slow: false,
                    note: None,
                });
            }
        }
    }

    // A NAMED npm re-resolve, for an owner whose own version is current and whose tree is not.
    // It is not in `stale` above, so it is named rather than planned — the same reason
    // `tools/audit`'s installed pass accepts rather than blocks. Cargo's half drops `--locked`;
    // npm has no lock to drop, and reinstalling the package is what makes npm resolve its ranges
    // again, so the argv is the ordinary one and the note says why it was asked for. An owner
    // that is already stale is skipped here: the loop above already installed it, which
    // re-resolves its tree as a side effect.
    if wanted("npm") {
        for name in &opts.re_resolve {
            let known = rows.iter().any(|r| r.surface == "npm" && &r.name == name);
            let already = stale.iter().any(|r| r.surface == "npm" && &r.name == name);
            if !known || already {
                continue;
            }
            steps.push(Step {
                surface_id: "npm".to_owned(),
                label: format!("npm: {name} (re-resolve)"),
                argv: vec![
                    "npm".into(),
                    "install".into(),
                    "-g".into(),
                    format!("{name}@latest"),
                ],
                slow: false,
                note: Some(
                    "re-resolving — its own version is current, its tree is behind; named because the audit flagged a dependency inside it"
                        .to_owned(),
                ),
            });
        }
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updates::{Ctx, RunResult, Runner};
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    // Output captured from `cargo install --list` on this host, 2026-10-01.
    const CARGO_LIST: &str =
        "bottom v0.14.7:\n    btm\nmacmon v0.7.0:\n    macmon\ntauri-cli v2.12.0:\n    cargo-tauri\nxberg-cli v1.0.14:\n    xberg\n";
    const NPM_OUTDATED: &str = r#"{"@marckrenn/pi-sub-bar":{"current":"1.4.0","wanted":"1.5.0","latest":"1.5.0"},"pnpm":{"current":"10.23.0","wanted":"12.8.1","latest":"12.8.1"}}"#;
    const NPM_INSTALLED: &str = r#"{"dependencies":{"@marckrenn/pi-sub-bar":{"version":"1.4.0"},"@earendil-works/pi-coding-agent":{"version":"0.99.2"},"uv":{"version":"1.4.0"}}}"#;
    const NPM_TREE_CONSTRAINED: &str = r#"{"dependencies":{"claude-agent-sdk-pi":{"version":"1.0.16","dependencies":{"@mariozechner/pi-coding-agent":{"version":"0.52.12","dependencies":{"@mariozechner/pi-agent-core":{"version":"0.52.12"}}}}},"@mariozechner/pi-agent-core":{"version":"0.52.12"},"defuddle":{"version":"0.19.3"}}}"#;

    /// A runner that answers from a table and records what it was asked, as the TypeScript
    /// `fakeRun` did.
    struct Fake {
        table: BTreeMap<String, String>,
        codes: BTreeMap<String, i32>,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl Fake {
        fn new(table: &[(&str, &str)]) -> Fake {
            Fake {
                table: table
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
                codes: BTreeMap::new(),
                calls: RefCell::new(Vec::new()),
            }
        }
        fn codes(mut self, codes: &[(&str, i32)]) -> Fake {
            self.codes = codes.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect();
            self
        }
        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.borrow().clone()
        }
    }

    impl Runner for Fake {
        fn run(&self, argv: &[&str]) -> RunResult {
            self.calls
                .borrow_mut()
                .push(argv.iter().map(|s| (*s).to_owned()).collect());
            let key = argv.join(" ");
            let hit = self.table.keys().find(|k| key.starts_with(k.as_str()));
            let code = self
                .codes
                .get(&key)
                .or_else(|| self.codes.get(argv[0]))
                .copied()
                .unwrap_or(if hit.is_some() { 0 } else { 1 });
            RunResult {
                code,
                stdout: hit.map(|k| self.table[k].clone()).unwrap_or_default(),
                stderr: String::new(),
            }
        }
        fn have(&self, bin: &str) -> Option<String> {
            let prefix = format!("{bin} ");
            self.table
                .keys()
                .any(|k| k == bin || k.starts_with(&prefix))
                .then(|| format!("/usr/bin/{bin}"))
        }
    }

    fn ctx<'a>(fake: &'a Fake, offline: bool) -> Ctx<'a> {
        Ctx {
            runner: fake,
            root: "/repo".to_owned(),
            overlay: "/overlay".to_owned(),
            offline,
        }
    }

    fn git_table(table: &mut Vec<(&'static str, &'static str)>) {
        table.push(("git -C /repo rev-list", "0\n"));
        table.push(("git -C /repo status", ""));
    }

    fn row<'a>(rows: &'a [Row], name: &str) -> &'a Row {
        rows.iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("no row named {name}"))
    }

    #[test]
    fn a_stale_crate_names_the_exact_command_that_moves_it() {
        let fake = Fake::new(&[
            ("cargo install --list", CARGO_LIST),
            ("curl -sS", r#"{"versions":[{"num":"0.8.2"}]}"#),
        ]);
        let rows = gather_cargo(&ctx(&fake, false));
        let macmon = row(&rows, "macmon");
        assert_eq!(macmon.status, Status::Stale);
        assert_eq!(macmon.installed.as_deref(), Some("0.7.0"));
        assert_eq!(macmon.latest.as_deref(), Some("0.8.2"));
        assert_eq!(
            macmon.action.as_deref(),
            Some("cargo install macmon --locked --force")
        );
    }

    #[test]
    fn a_stable_patch_is_adopted_while_the_alpha_above_it_is_only_named() {
        let fake = Fake::new(&[
            (
                "cargo install --list",
                "tauri-cli v2.12.0:\n    cargo-tauri\n",
            ),
            (
                "curl -sS",
                r#"{"versions":[{"num":"3.0.0-alpha.4"},{"num":"2.12.1"},{"num":"2.12.0"}]}"#,
            ),
        ]);
        let rows = gather_cargo(&ctx(&fake, false));
        let r = &rows[0];
        assert_eq!(r.status, Status::Stale);
        assert_eq!(r.latest.as_deref(), Some("2.12.1"));
        assert_eq!(
            r.action.as_deref(),
            Some("cargo install tauri-cli --locked --force")
        );
        let note = r.note.clone().unwrap();
        assert!(
            note.contains("3.0.0-alpha.4") && note.contains("not adopted"),
            "{note}"
        );
    }

    #[test]
    fn when_only_prereleases_exist_nothing_is_adopted() {
        let fake = Fake::new(&[
            (
                "cargo install --list",
                "tauri-cli v2.12.0:\n    cargo-tauri\n",
            ),
            ("curl -sS", r#"{"versions":[{"num":"3.0.0-alpha.4"}]}"#),
        ]);
        let rows = gather_cargo(&ctx(&fake, false));
        assert_eq!(rows[0].status, Status::Current);
        assert!(rows[0].action.is_none());
        assert!(rows[0].note.clone().unwrap().contains("pre-release"));
    }

    #[test]
    fn an_unreachable_crates_io_falls_back_to_cargo_search() {
        let fake = Fake::new(&[
            ("cargo install --list", "macmon v0.7.0:\n    macmon\n"),
            ("cargo search macmon", "macmon = \"0.8.2\""),
        ]);
        let rows = gather_cargo(&ctx(&fake, false));
        assert_eq!(rows[0].status, Status::Stale);
        assert_eq!(rows[0].latest.as_deref(), Some("0.8.2"));
    }

    #[test]
    fn a_registry_that_does_not_answer_is_unknown_never_current() {
        let fake =
            Fake::new(&[("cargo install --list", CARGO_LIST)]).codes(&[("cargo search macmon", 1)]);
        let rows = gather_cargo(&ctx(&fake, false));
        assert_eq!(row(&rows, "macmon").status, Status::Unknown);
    }

    #[test]
    fn offline_reports_the_installed_version_and_claims_nothing_about_newer_ones() {
        let fake = Fake::new(&[("cargo install --list", CARGO_LIST)]);
        let rows = gather_cargo(&ctx(&fake, true));
        assert!(rows.iter().all(|r| r.status == Status::Unknown));
        assert!(!fake
            .calls()
            .iter()
            .any(|c| c[0] == "cargo" && c.get(1).map(String::as_str) == Some("search")));
    }

    #[test]
    fn a_package_on_latest_still_appears_so_the_list_is_an_inventory() {
        let fake = Fake::new(&[
            ("npm ls -g", NPM_INSTALLED),
            ("npm outdated -g", NPM_OUTDATED),
        ]);
        let rows = gather_npm(&ctx(&fake, false), &Default::default());
        let pi = row(&rows, "@earendil-works/pi-coding-agent");
        assert_eq!(pi.status, Status::Current);
        assert_eq!(pi.installed.as_deref(), Some("0.99.2"));
    }

    #[test]
    fn a_name_that_is_also_a_brew_formula_is_flagged_to_check_path() {
        let fake = Fake::new(&[("npm ls -g", NPM_INSTALLED), ("npm outdated -g", "{}")]);
        let formulae = parse_brew_formulae("uv\njq\nnettle\n");
        let rows = gather_npm(&ctx(&fake, false), &formulae);
        assert!(row(&rows, "uv")
            .note
            .clone()
            .unwrap()
            .contains("brew formula"));
        assert!(row(&rows, "@marckrenn/pi-sub-bar").note.is_none());
    }

    #[test]
    fn a_package_another_global_pins_is_stale_names_its_parent_and_carries_no_action() {
        let fake = Fake::new(&[
            ("npm ls -g", NPM_TREE_CONSTRAINED),
            (
                "npm outdated -g",
                r#"{"@mariozechner/pi-agent-core":{"current":"0.52.12","latest":"0.73.1"},"defuddle":{"current":"0.19.3","latest":"0.19.4"}}"#,
            ),
        ]);
        let rows = gather_npm(&ctx(&fake, false), &Default::default());
        let pinned = row(&rows, "@mariozechner/pi-agent-core");
        assert_eq!(pinned.status, Status::Stale);
        assert!(pinned.action.is_none());
        let note = pinned.note.clone().unwrap();
        assert!(note.contains("pinned by claude-agent-sdk-pi"), "{note}");
        assert!(note.contains("upgrade those instead"), "{note}");
        let free = row(&rows, "defuddle");
        assert_eq!(
            free.action.as_deref(),
            Some("npm install -g defuddle@latest")
        );
    }

    #[test]
    fn a_duplicate_nothing_requires_is_named_as_unused_and_carries_no_action() {
        let tree = r#"{"dependencies":{"@marckrenn/pi-sub-bar":{"version":"1.5.0","dependencies":{"@mariozechner/pi-coding-agent":{"version":"0.73.1","dependencies":{"@mariozechner/pi-agent-core":{"version":"0.73.1"}}}}},"@mariozechner/pi-agent-core":{"version":"0.52.12"}}}"#;
        let fake = Fake::new(&[
            ("npm ls -g", tree),
            (
                "npm outdated -g",
                r#"{"@mariozechner/pi-agent-core":{"current":"0.52.12","latest":"0.73.1"}}"#,
            ),
        ]);
        let rows = gather_npm(&ctx(&fake, false), &Default::default());
        let r = row(&rows, "@mariozechner/pi-agent-core");
        assert!(r.action.is_none());
        let note = r.note.clone().unwrap();
        assert!(
            note.contains("unused duplicate") && note.contains("bundles its own 0.73.1"),
            "{note}"
        );
        assert_eq!(r.removable, Some(true));
    }

    #[test]
    fn a_deprecated_package_is_named_as_deprecated_and_carries_no_action() {
        let fake = Fake::new(&[
            ("npm ls -g", NPM_INSTALLED),
            (
                "npm outdated -g",
                r#"{"uv":{"current":"1.4.0","latest":"1.5.0"}}"#,
            ),
            (
                "npm view uv deprecated",
                "please use @earendil-works/uv instead going forward",
            ),
        ]);
        let rows = gather_npm(&ctx(&fake, false), &Default::default());
        let uv = row(&rows, "uv");
        assert!(uv.action.is_none());
        let note = uv.note.clone().unwrap();
        assert!(
            note.contains("deprecated") && note.contains("@earendil-works/uv"),
            "{note}"
        );
    }

    #[test]
    fn a_live_package_is_offered_and_asked_about_deprecation_only_once() {
        let fake = Fake::new(&[
            ("npm ls -g", NPM_INSTALLED),
            (
                "npm outdated -g",
                r#"{"uv":{"current":"1.4.0","latest":"1.5.0"}}"#,
            ),
        ]);
        let rows = gather_npm(&ctx(&fake, false), &Default::default());
        assert_eq!(
            row(&rows, "uv").action.as_deref(),
            Some("npm install -g uv@latest")
        );
        let views = fake
            .calls()
            .iter()
            .filter(|c| c[0] == "npm" && c.get(1).map(String::as_str) == Some("view"))
            .count();
        assert_eq!(views, 1);
    }

    fn plan_ctx<'a>(fake: &'a Fake) -> Ctx<'a> {
        ctx(fake, false)
    }

    fn stale_rows() -> Vec<Row> {
        let mut brew = Row::new("brew", Status::Stale).name("nettle");
        brew.action = None;
        vec![
            brew,
            Row::new("cargo", Status::Stale)
                .name("macmon")
                .action("cargo install macmon --locked --force"),
            Row::new("npm", Status::Stale)
                .name("pnpm")
                .action("npm install -g pnpm@latest"),
            Row::new("graphify", Status::Stale)
                .name("harness integration")
                .action("tools/agent-integrations.sh update graphify"),
        ]
    }

    #[test]
    fn brew_is_moved_by_its_owner_never_by_a_command_this_tool_invents() {
        let fake = Fake::new(&[]);
        let steps = plan_apply(&stale_rows(), &[], &plan_ctx(&fake), &PlanOpts::default());
        let brew = steps.iter().find(|s| s.surface_id == "hostpatch").unwrap();
        assert_eq!(brew.argv, vec!["/repo/tools/host-patch.sh"]);
        let all = steps
            .iter()
            .flat_map(|s| s.argv.clone())
            .collect::<Vec<_>>()
            .join(" ");
        for forbidden in ["brew upgrade", "uv tool", "rustup update"] {
            assert!(!all.contains(forbidden), "{all}");
        }
    }

    #[test]
    fn the_unowned_classes_are_moved_directly() {
        let fake = Fake::new(&[]);
        let steps = plan_apply(&stale_rows(), &[], &plan_ctx(&fake), &PlanOpts::default());
        assert_eq!(
            steps.iter().find(|s| s.surface_id == "cargo").unwrap().argv,
            vec!["cargo", "install", "macmon", "--locked", "--force"]
        );
        assert_eq!(
            steps.iter().find(|s| s.surface_id == "npm").unwrap().argv,
            vec!["npm", "install", "-g", "pnpm@latest"]
        );
        assert_eq!(
            steps
                .iter()
                .find(|s| s.surface_id == "graphify")
                .unwrap()
                .argv,
            vec!["/repo/tools/agent-integrations.sh", "update", "graphify"]
        );
    }

    #[test]
    fn only_scopes_the_plan_and_nothing_stale_plans_nothing() {
        let fake = Fake::new(&[]);
        let steps = plan_apply(
            &stale_rows(),
            &["cargo".to_owned()],
            &plan_ctx(&fake),
            &PlanOpts::default(),
        );
        assert_eq!(
            steps
                .iter()
                .map(|s| s.surface_id.as_str())
                .collect::<Vec<_>>(),
            vec!["cargo"]
        );
        assert_eq!(
            plan_apply(&[], &[], &plan_ctx(&fake), &PlanOpts::default()),
            vec![]
        );
    }

    fn prunable() -> Vec<Row> {
        vec![
            Row::new("npm", Status::Stale)
                .name("dead-scope")
                .removable()
                .note("deprecated — use @earendil-works"),
            Row::new("npm", Status::Stale)
                .name("unused-dup")
                .removable()
                .note("unused duplicate — x bundles its own 2.0.0"),
            Row::new("npm", Status::Stale)
                .name("pinned-copy")
                .note("pinned by y — upgrade those instead"),
            Row::new("npm", Status::Stale)
                .name("normal")
                .action("npm install -g normal@latest"),
        ]
    }

    #[test]
    fn only_leftovers_nothing_requires_are_prunable() {
        let rows = prunable();
        let names: Vec<&str> = plan_prune(&rows).iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["dead-scope", "unused-dup"]);
    }

    #[test]
    fn without_the_flag_a_leftover_is_reported_and_never_removed() {
        let fake = Fake::new(&[]);
        let steps = plan_apply(&prunable(), &[], &plan_ctx(&fake), &PlanOpts::default());
        assert!(!steps
            .iter()
            .any(|s| s.argv.iter().any(|a| a == "uninstall")));
        assert_eq!(
            steps
                .iter()
                .find(|s| s.label == "npm: normal")
                .unwrap()
                .argv,
            vec!["npm", "install", "-g", "normal@latest"]
        );
    }

    #[test]
    fn with_it_each_removal_is_its_own_uninstall_and_removals_come_first() {
        let fake = Fake::new(&[]);
        let opts = PlanOpts {
            prune: true,
            re_resolve: vec![],
        };
        let steps = plan_apply(&prunable(), &[], &plan_ctx(&fake), &opts);
        let removals: Vec<Vec<String>> = steps
            .iter()
            .filter(|s| s.argv.get(1).map(String::as_str) == Some("uninstall"))
            .map(|s| s.argv.clone())
            .collect();
        assert_eq!(
            removals,
            vec![
                vec!["npm", "uninstall", "-g", "dead-scope"],
                vec!["npm", "uninstall", "-g", "unused-dup"],
            ]
        );
        assert_eq!(steps[0].argv.get(1).map(String::as_str), Some("uninstall"));
        assert!(!steps
            .iter()
            .any(|s| s.label == "pinned-copy" || s.argv.iter().any(|a| a == "pinned-copy")));
    }

    #[test]
    fn re_resolve_drops_locked_for_the_crate_named_and_no_other() {
        let fake = Fake::new(&[]);
        let two = vec![
            Row::new("cargo", Status::Stale)
                .name("bottom")
                .action("cargo install bottom --locked --force"),
            Row::new("cargo", Status::Stale)
                .name("macmon")
                .action("cargo install macmon --locked --force"),
        ];
        let opts = PlanOpts {
            prune: false,
            re_resolve: vec!["bottom".to_owned()],
        };
        let steps = plan_apply(&two, &[], &plan_ctx(&fake), &opts);
        assert_eq!(
            steps
                .iter()
                .find(|s| s.label == "cargo: bottom")
                .unwrap()
                .argv,
            vec!["cargo", "install", "bottom", "--force"]
        );
        assert_eq!(
            steps
                .iter()
                .find(|s| s.label == "cargo: macmon")
                .unwrap()
                .argv,
            vec!["cargo", "install", "macmon", "--locked", "--force"]
        );
        assert!(steps
            .iter()
            .find(|s| s.label == "cargo: bottom")
            .unwrap()
            .note
            .clone()
            .unwrap()
            .contains("--locked dropped"));
    }

    #[test]
    fn re_resolve_moves_a_current_npm_owner_and_never_doubles_a_stale_one() {
        // bobshell is the case this exists for: its own version is current, its tree is behind,
        // so it is not a stale row and nothing above would have planned it. pnpm is stale, so the
        // ordinary loop already installs it — which re-resolves its tree as a side effect, and a
        // second step for it would install the same package twice.
        let fake = Fake::new(&[]);
        let rows = vec![
            Row::new("npm", Status::Current).name("bobshell"),
            Row::new("npm", Status::Stale)
                .name("pnpm")
                .action("npm install -g pnpm@latest"),
        ];
        let opts = PlanOpts {
            prune: false,
            re_resolve: vec![
                "bobshell".to_owned(),
                "pnpm".to_owned(),
                "absent".to_owned(),
            ],
        };
        let steps = plan_apply(&rows, &["npm".to_owned()], &plan_ctx(&fake), &opts);
        assert_eq!(
            steps.iter().map(|s| s.argv.clone()).collect::<Vec<_>>(),
            vec![
                vec!["npm", "install", "-g", "pnpm@latest"],
                vec!["npm", "install", "-g", "bobshell@latest"],
            ],
            "the stale row once, the current owner once, and a name in no row at all never"
        );
        let re = steps
            .iter()
            .find(|s| s.label == "npm: bobshell (re-resolve)")
            .expect("the re-resolve step");
        assert!(re.note.as_deref().unwrap_or("").contains("tree is behind"));
    }

    #[test]
    fn a_nested_location_names_its_outermost_global_owner() {
        let owners = vec![
            "npm".to_owned(),
            "@scope/a".to_owned(),
            "bobshell".to_owned(),
        ];
        let root = "/opt/homebrew/lib/node_modules";
        assert_eq!(
            npm_owner_of(&format!("{root}/bobshell/node_modules/simple-git"), &owners),
            Some("bobshell".to_owned())
        );
        // A scoped owner is two path segments, which is why this matches whole names rather than
        // splitting on `/`.
        assert_eq!(
            npm_owner_of(&format!("{root}/@scope/a/node_modules/ip-address"), &owners),
            Some("@scope/a".to_owned())
        );
        // Deeper: the OUTERMOST owner wins, because that is the install that moves the tree. A
        // nested `npm` must not claim a package that lives inside bobshell.
        assert_eq!(
            npm_owner_of(
                &format!("{root}/bobshell/node_modules/x/node_modules/npm/node_modules/y"),
                &owners
            ),
            Some("bobshell".to_owned())
        );
        // A top-level package is not nested, and an unknown global root is not guessed at.
        assert_eq!(npm_owner_of(&format!("{root}/pnpm"), &owners), None);
        assert_eq!(
            npm_owner_of(&format!("{root}/other/node_modules/x"), &owners),
            None
        );
    }

    #[test]
    fn the_inventory_names_the_osv_ecosystem_and_skips_a_manager_that_is_not_installed() {
        let all = r#"{"dependencies":{"top":{"version":"1.0.0","dependencies":{"nested":{"version":"2.0.0"}}}}}"#;
        let fake = Fake::new(&[
            ("npm ls -g --json --all", all),
            ("cargo install --list", "macmon v0.8.2:\n    macmon\n"),
        ]);
        assert_eq!(
            build_inventory(&ctx(&fake, true)),
            vec![
                InventoryEntry {
                    ecosystem: "npm",
                    name: "top".into(),
                    version: "1.0.0".into()
                },
                InventoryEntry {
                    ecosystem: "npm",
                    name: "nested".into(),
                    version: "2.0.0".into()
                },
                InventoryEntry {
                    ecosystem: "crates.io",
                    name: "macmon".into(),
                    version: "0.8.2".into()
                },
            ]
        );
        let npm_only = Fake::new(&[("npm ls -g --json --all", all)]);
        assert!(build_inventory(&ctx(&npm_only, true))
            .iter()
            .all(|e| e.ecosystem == "npm"));
    }

    #[test]
    fn only_json_inventory_carries_it_absent_is_absent_not_an_empty_array() {
        let mut table = vec![];
        git_table(&mut table);
        let fake = Fake::new(&table);
        let c = ctx(&fake, true);
        let report = build_report(&c);
        let without = serde_json::from_str::<serde_json::Value>(&render_json(
            &report.rows,
            &report.generated_at,
            true,
            None,
            None,
        ))
        .unwrap();
        assert!(without.get("inventory").is_none());
        let with = serde_json::from_str::<serde_json::Value>(&render_json(
            &report.rows,
            &report.generated_at,
            true,
            None,
            Some(&[]),
        ))
        .unwrap();
        assert_eq!(with.get("inventory").unwrap(), &serde_json::json!([]));
    }

    #[test]
    fn a_report_over_planted_input_groups_every_row_under_a_known_surface() {
        let mut table = vec![
            ("cargo install --list", CARGO_LIST),
            ("npm ls -g", NPM_INSTALLED),
            ("npm outdated -g", NPM_OUTDATED),
        ];
        git_table(&mut table);
        let fake = Fake::new(&table);
        let report = build_report(&ctx(&fake, true));
        for r in &report.rows {
            assert!(
                surface(&r.surface).is_some(),
                "unknown surface {}",
                r.surface
            );
        }
        let grouped_count: usize = grouped(&report.rows).iter().map(|(_, rs)| rs.len()).sum();
        assert_eq!(grouped_count, report.rows.len());
    }

    #[test]
    fn the_table_names_the_owners_command_for_every_stale_row() {
        let mut table = vec![
            ("cargo install --list", "macmon v0.7.0:\n    macmon\n"),
            ("cargo search macmon", "macmon = \"0.8.2\""),
            ("npm ls -g", NPM_INSTALLED),
            ("npm outdated -g", "{}"),
        ];
        git_table(&mut table);
        let fake = Fake::new(&table);
        let c = ctx(&fake, false);
        let report = build_report(&c);
        let table_text = render_table(&report.rows, false, None);
        let stale: Vec<&Row> = report
            .rows
            .iter()
            .filter(|r| r.status == Status::Stale)
            .collect();
        assert!(!stale.is_empty());
        for r in &stale {
            let note = r.note.clone().unwrap_or_default();
            let explained = r.action.is_some()
                || (note.starts_with("pinned by ") && note.contains("upgrade those instead"))
                || note.starts_with("unused duplicate — ")
                || note.starts_with("deprecated — ");
            assert!(
                explained,
                "{} {} is stale with neither an action nor a reason",
                r.surface, r.name
            );
        }
        assert!(table_text.contains("Unowned — nothing moves these"));
        let json = serde_json::from_str::<serde_json::Value>(&render_json(
            &report.rows,
            &report.generated_at,
            false,
            None,
            None,
        ))
        .unwrap();
        assert_eq!(json["rows"].as_array().unwrap().len(), report.rows.len());
        assert_eq!(json["surfaces"].as_array().unwrap().len(), SURFACES.len());
        assert_eq!(json["generatedAt"].as_str().unwrap(), report.generated_at);
    }

    #[test]
    fn every_surface_has_an_owner_and_a_reason_and_the_two_unowned_ones_are_actionable() {
        for s in SURFACES {
            assert!(s.why.len() > 10);
            assert!(s.owner_detail.len() > 3);
        }
        let unowned: Vec<&str> = SURFACES
            .iter()
            .filter(|s| s.owner == Owner::Unowned)
            .map(|s| s.id)
            .collect();
        assert_eq!(unowned, vec!["cargo", "npm"]);
        for s in SURFACES.iter().filter(|s| s.owner == Owner::Unowned) {
            assert!(s.actionable);
        }
        for s in SURFACES.iter().filter(|s| s.owner == Owner::SelfManaged) {
            assert!(!s.actionable);
        }
        assert!(surface("apt").is_none());
    }

    // ---- receipts ---------------------------------------------------------------------------

    fn temp_overlay(tag: &str) -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "sjel-updates-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.display().to_string()
    }

    #[test]
    fn the_apply_receipt_round_trips_and_a_missing_one_is_absent() {
        let overlay = temp_overlay("receipt");
        assert!(read_apply_receipt(&overlay).is_none());
        write_apply_receipt(
            &overlay,
            &ApplyReceipt {
                at: Some("2026-10-01T00:00:00Z".into()),
                class: Some("cargo".into()),
                steps: Some(3),
                state: Some("running".into()),
                ..Default::default()
            },
        );
        let read = read_apply_receipt(&overlay).unwrap();
        assert_eq!(read.at.as_deref(), Some("2026-10-01T00:00:00Z"));
        assert_eq!(read.class.as_deref(), Some("cargo"));
        assert_eq!(read.steps, Some(3));
        assert_eq!(read.state.as_deref(), Some("running"));
        assert!(apply_receipt_path(&overlay).contains("data/updates/last-apply.json"));
        let _ = std::fs::remove_dir_all(&overlay);
    }

    #[test]
    fn a_corrupt_receipt_reads_as_absent_because_the_report_must_still_render() {
        let overlay = temp_overlay("corrupt");
        write_apply_receipt(
            &overlay,
            &ApplyReceipt {
                at: Some("x".into()),
                class: Some("cargo".into()),
                state: Some("running".into()),
                ..Default::default()
            },
        );
        std::fs::write(apply_receipt_path(&overlay), "{ not json").unwrap();
        assert!(read_apply_receipt(&overlay).is_none());
        let _ = std::fs::remove_dir_all(&overlay);
    }

    #[test]
    fn the_json_payload_carries_last_apply_so_a_panel_needs_no_second_endpoint() {
        let mut table = vec![];
        git_table(&mut table);
        let fake = Fake::new(&table);
        let c = ctx(&fake, true);
        let report = build_report(&c);
        let receipt = ApplyReceipt {
            at: Some("2026-10-01T00:00:00Z".into()),
            class: Some("npm".into()),
            state: Some("done".into()),
            steps: Some(11),
            ..Default::default()
        };
        let json = serde_json::from_str::<serde_json::Value>(&render_json(
            &report.rows,
            &report.generated_at,
            true,
            Some(&receipt),
            None,
        ))
        .unwrap();
        assert_eq!(json["lastApply"]["class"], "npm");
        assert_eq!(json["lastApply"]["steps"], 11);
        // Absent lastApply is an explicit null, not a missing key.
        let none = serde_json::from_str::<serde_json::Value>(&render_json(
            &report.rows,
            &report.generated_at,
            true,
            None,
            None,
        ))
        .unwrap();
        assert!(none.get("lastApply").unwrap().is_null());
    }

    #[test]
    fn a_running_apply_is_visible_and_a_finished_one_says_how_many_steps() {
        let mut table = vec![];
        git_table(&mut table);
        let fake = Fake::new(&table);
        let report = build_report(&ctx(&fake, true));
        let running = ApplyReceipt {
            at: Some(crate::time::now_iso()),
            class: Some("cargo".into()),
            state: Some("running".into()),
            ..Default::default()
        };
        assert!(render_table(&report.rows, false, Some(&running))
            .contains("an apply is running: cargo"));
        let done = ApplyReceipt {
            at: Some("2026-10-01T00:00:00Z".into()),
            class: Some("npm".into()),
            state: Some("done".into()),
            steps: Some(11),
            failed: Some(1),
            ..Default::default()
        };
        assert!(render_table(&report.rows, false, Some(&done))
            .contains("last apply: npm done · 11 step(s), 1 failed"));
    }

    #[test]
    fn the_audit_verdict_survives_the_receipt_and_reaches_the_table() {
        let overlay = temp_overlay("audit");
        write_apply_receipt(
            &overlay,
            &ApplyReceipt {
                at: Some("2026-10-03T00:00:00Z".into()),
                class: Some("cargo".into()),
                steps: Some(1),
                failed: Some(0),
                still_stale: Some(0),
                state: Some("done".into()),
                audit: Some("finding(s)".into()),
            },
        );
        assert_eq!(
            read_apply_receipt(&overlay).unwrap().audit.as_deref(),
            Some("finding(s)")
        );
        let _ = std::fs::remove_dir_all(&overlay);

        let mut table = vec![];
        git_table(&mut table);
        let fake = Fake::new(&table);
        let report = build_report(&ctx(&fake, true));
        let receipt = ApplyReceipt {
            at: Some("2026-10-03T00:00:00Z".into()),
            class: Some("cargo".into()),
            state: Some("done".into()),
            steps: Some(1),
            audit: Some("scanner-missing".into()),
            ..Default::default()
        };
        assert!(render_table(&report.rows, false, Some(&receipt)).contains("audit scanner-missing"));
    }

    #[test]
    fn a_receipt_written_before_the_field_existed_still_renders() {
        let mut table = vec![];
        git_table(&mut table);
        let fake = Fake::new(&table);
        let report = build_report(&ctx(&fake, true));
        let receipt = ApplyReceipt {
            at: Some("2026-10-01T00:00:00Z".into()),
            class: Some("npm".into()),
            state: Some("done".into()),
            steps: Some(2),
            ..Default::default()
        };
        let table_text = render_table(&report.rows, false, Some(&receipt));
        assert!(table_text.contains("last apply: npm done · 2 step(s)"));
        assert!(!table_text.contains("audit"));
    }

    #[test]
    fn it_runs_the_repositorys_own_audit_at_the_root_it_was_given() {
        let fake = Fake::new(&[]).codes(&[("/repo/tools/audit", 1)]);
        let c = ctx(&fake, false);
        assert_eq!(run_audit(&c), AuditVerdict::Finding);
        assert_eq!(fake.calls(), vec![vec!["/repo/tools/audit".to_owned()]]);
    }
}
