#!/usr/bin/env bun
// tools/updates — every piece of software installed outside this checkout: who owns moving it,
// what has gone stale, and what nothing moves at all.
//
// ## Why this exists
//
// Sjel installs software from four places and only two of them were ever accounted for.
// capabilities/host-patch moves brew, uv and rustup nightly (Q77, CONTRIBUTING.md#patch-first),
// capabilities/container-refresh moves the images, and the agent harnesses move themselves and
// say so on startup. Everything else — crates installed with `cargo install`, global npm
// packages, an upstream's own harness integration, an app bundle outside brew — moves only when
// somebody remembers that it exists. `toolchain.toml` [macmon] is the shape of the failure: it
// declares the tool, pins a floor, and gives `cargo install macmon --locked` as the install
// hint, so the file names an install path with no update path and the host sits at the floor.
// This tool is the answer to "is everything current?", which had no single one before.
//
// ## The rule it inherits: one binary, one owner
//
// tools/host-patch.sh states it; this tool obeys it rather than restating it in code. Brew, uv
// and rustup already own every binary they installed, and a second updater for one of those is
// the failure this deployment has already paid for — a `~/.local/bin` yt-dlp shadowed brew's
// copy and returned HTTP 403 on every media URL while `--dump-json` kept working (PRD §13). So
// `apply` does NOT reimplement a single brew or uv step: it execs tools/host-patch.sh, which is
// the one owner of those. What `apply` owns directly is exactly the set nothing else owns —
// `cargo install`ed crates and `npm -g` packages — plus delegation to the manual verbs for the
// two upstream integrations. A tool that grew its own `brew upgrade` would be that failure
// again, with a nicer table. `report` therefore does not decide whether a brew formula should
// move either: it asks brew, which is the owner, and repeats the answer.
//
// ## Why this is not doctor
//
// doctor stayed offline by ruling (PRD Q41), and asking three registries what they have today is
// the opposite of offline. tools/agent-integrations.sh settled the question for the same reason
// ("update checking lives HERE — a networked verb ... and deliberately not in doctor"). This is
// the second such verb, and it is the front door in front of the first: `sjel update` reports
// every class in one table, and doctor keeps reporting whether the scheduled jobs ran.
//
// ## The two modes, and why `--offline` exists
//
// The default reachable report answers "is anything stale", which needs the registries. A host
// with no network — or a caller that must not wait on one — asks `--offline` and gets the
// receipts and installed versions only, with the unowned rows marked `unknown` rather than
// guessed at. A report that printed yesterday's answer as today's would be worse than no report.
//
// ## What this report does NOT claim
//
// Staleness it can measure, it measures: brew is asked (`brew outdated`), rustup is asked
// (`rustup check`), cargo crates have their newest release looked up one by one, and npm reports
// its own. Two things it deliberately does not measure, because there is no version to measure
// against (Q77 deleted every pin):
//
//   - `uv tool` has no "what is new" verb, so its row carries the nightly job's receipt instead.
//   - an upstream's harness integration records the DATE it was last re-derived from upstream,
//     never a version. A 30-day marker is a date the operator reads, not a claim this tool
//     proves; the row says which date it read.
//
// Container images are reported from capabilities/container-refresh's receipt and never pulled
// here — that capability owns the digest comparison (ISA.md C4) and a second puller would be the
// two-owners failure again.
//
// ## Output contract
//
// `--json` emits { generatedAt, offline, surfaces: [...], rows: [...] }. That payload is the
// stable surface: the CLI table is derived from it, not the other way round, so a dashboard
// panel renders the same facts the terminal does instead of a second implementation of them.
// row.status is one of current | stale | unknown | n/a and row.owner is one of
// scheduled | manual | unowned | self | app. Those two vocabularies are the whole contract.
//
// ## Exit codes
//
// report: 0 = nothing stale · 1 = something is stale · 2 = usage error.
// apply:  0 = every step succeeded · 1 = nothing to do or aborted · 2 = a step failed.

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

// ── the runner ────────────────────────────────────────────────────────────────
// Injected rather than called directly, so tools/updates.test.ts can drive every gatherer and
// the apply plan against planted output without a brew, a cargo registry or a network.

export type RunResult = { code: number; stdout: string; stderr: string };
export type Runner = (argv: string[]) => RunResult;

export type Ctx = {
  run: Runner;
  have: (bin: string) => string | null;
  root: string;
  overlay: string;
  offline: boolean;
};

export function defaultRunner(argv: string[]): RunResult {
  const p = Bun.spawnSync({ cmd: argv, stdout: "pipe", stderr: "pipe" });
  return {
    code: p.exitCode ?? 1,
    stdout: p.stdout?.toString() ?? "",
    stderr: p.stderr?.toString() ?? "",
  };
}

export function defaultHave(bin: string): string | null {
  return Bun.which(bin) ?? null;
}

// ── the surfaces and their owners ─────────────────────────────────────────────
// One row per class of installed software, not per binary. `owner` is the verdict this tool
// publishes, and it is the thing that was missing: a class nobody owns is not a bug in the
// class, it is a fact the operator is entitled to see stated once instead of discovered.

export type Owner = "scheduled" | "manual" | "unowned" | "self";
export type Status = "current" | "stale" | "unknown" | "n/a";

export type Surface = {
  id: string;
  title: string;
  owner: Owner;
  ownerDetail: string;
  /** True when `apply` moves this class, by delegation or directly. */
  actionable: boolean;
  why: string;
};

export const SURFACES: Surface[] = [
  {
    id: "brew",
    title: "Homebrew formulae and casks",
    owner: "scheduled",
    ownerDetail: "capabilities/host-patch · 24h",
    actionable: true,
    why: "brew owns its binaries and answers `brew outdated` itself; the nightly job moves them, including auto_updates casks via --greedy (Q77)",
  },
  {
    id: "uv",
    title: "uv tools",
    owner: "scheduled",
    ownerDetail: "capabilities/host-patch · 24h",
    actionable: true,
    why: "one upgrade step per tool, not --all, so one broken tool cannot leave every other one unpatched; uv has no 'what is new' verb, so this row cites the job's receipt",
  },
  {
    id: "rustup",
    title: "rustup toolchain",
    owner: "scheduled",
    ownerDetail: "capabilities/host-patch · 24h",
    actionable: true,
    why: "the toolchain, not the crates built with it — those are the `cargo` surface below",
  },
  {
    id: "containers",
    title: "Container images",
    owner: "scheduled",
    ownerDetail: "capabilities/container-refresh · 24h",
    actionable: false,
    why: "the declared tag is a channel and the digest is the fact (ISA.md C4); reported from its receipt and never pulled here",
  },
  {
    id: "graphify",
    title: "graphify harness integration",
    owner: "manual",
    ownerDetail: "tools/agent-integrations.sh update graphify",
    actionable: true,
    why: "the binary is a uv tool and moves nightly; the skill/plugin files upstream's installer wrote into each harness are re-derived only when this verb is run",
  },
  {
    id: "interceptor",
    title: "interceptor CLI and skills",
    owner: "manual",
    ownerDetail: "tools/agent-integrations.sh update interceptor",
    actionable: true,
    why: "the product owns its own updater (`interceptor upgrade`) and nothing schedules it; the skills it links into each harness are re-adopted beside it",
  },
  {
    id: "checkout",
    title: "This checkout",
    owner: "manual",
    ownerDetail: "tools/update.sh",
    actionable: false,
    why: "fast-forward only, and a diverged checkout is left alone with instructions rather than discarded",
  },
  {
    id: "cargo",
    title: "cargo-installed binaries",
    owner: "unowned",
    ownerDetail: "nothing — this tool moves them",
    actionable: true,
    why: "`cargo install` has no update verb and no register; toolchain.toml names the install path for [macmon] and nothing named an update path",
  },
  {
    id: "npm",
    title: "npm global packages",
    owner: "unowned",
    ownerDetail: "nothing — this tool moves them",
    actionable: true,
    why: "the harnesses self-update but the packages beside them do not, and nothing ran `npm outdated -g`",
  },
  {
    id: "vendor",
    title: "Vendor-managed apps",
    owner: "self",
    ownerDetail: "their own updater",
    actionable: false,
    why: "the harnesses and Ollama.app update themselves; a second updater for them would be a second owner",
  },
];

export function surface(id: string): Surface {
  const s = SURFACES.find((x) => x.id === id);
  if (!s) throw new Error(`unknown class '${id}'`);
  return s;
}

export type Row = {
  surface: string;
  name: string;
  owner: Owner;
  ownerDetail: string;
  installed?: string;
  latest?: string;
  status: Status;
  /** The command that moves it. The owner's entry point, never a reimplementation. */
  action?: string;
  /** True only when this row is a leftover nothing requires: a deprecated package, or a
   *  duplicate whose parents bundle their own copy. It is exactly what `--prune` removes.
   *  A row PINNED by a parent is deliberately never marked — that copy is load-bearing, and
   *  removing it breaks the package that named it. */
  removable?: boolean;
  note?: string;
};

function mk(surfaceId: string, p: Partial<Row> & { status: Status }): Row {
  const s = surface(surfaceId);
  return {
    surface: s.id,
    name: p.name ?? "",
    owner: s.owner,
    ownerDetail: s.ownerDetail,
    status: p.status,
    installed: p.installed,
    latest: p.latest,
    action: p.action,
    removable: p.removable,
    note: p.note,
  };
}

// ── parsers ───────────────────────────────────────────────────────────────────
// Every one of these is driven against real captured output in tools/updates.test.ts. The
// formats are the package managers' own, not ours, so the parsers are the part most likely to
// rot silently and the part worth the tests.

/** `cargo install --list`. A crate's binaries are indented continuation lines. */
export function parseCargoInstallList(text: string): { name: string; version: string }[] {
  const out: { name: string; version: string }[] = [];
  for (const line of text.split("\n")) {
    // "macmon v0.7.0:" — the trailing colon is cargo's own and older cargos omitted it, so it
    // is optional here. Anything indented is a binary name, not a crate.
    const m = line.match(/^([A-Za-z0-9_.-]+) v(\S+?):?\s*$/);
    if (m) out.push({ name: m[1], version: m[2] });
  }
  return out;
}

/**
 * crates.io's version list → the newest STABLE release, and the newest release of any kind.
 *
 * The defect this exists to fix: `cargo search` returns only the maximum version, so for
 * `tauri-cli` it answered `3.0.0-alpha.4` and this tool concluded "no stable is newer" — while
 * `2.12.1`, a released patch above the installed `2.12.0`, sat one line further down the list
 * the registry had already sent. A report that hides a legitimate patch upgrade because an
 * unrelated pre-release sorts higher is worse than one that reports nothing.
 *
 * Yanked versions are dropped: they are published but withdrawn, and offering one would be the
 * same mistake in the other direction. The prerelease test is the hyphen semver requires.
 */
export function parseCratesIoVersions(json: string): { stable: string | null; newest: string | null } {
  let parsed: { versions?: { num?: string; yanked?: boolean }[] };
  try {
    parsed = JSON.parse(json);
  } catch {
    return { stable: null, newest: null };
  }
  const nums = (parsed?.versions ?? [])
    .filter((v) => v?.num && !v.yanked)
    .map((v) => v.num as string);
  if (nums.length === 0) return { stable: null, newest: null };
  const max = (list: string[]) => list.reduce((a, b) => (versionNewer(b, a) ? b : a));
  const stables = nums.filter((n) => !n.includes("-"));
  return { stable: stables.length ? max(stables) : null, newest: max(nums) };
}

/** `cargo search <crate> --limit 1` → the newest release, or null when it says nothing. */
export function parseCargoSearch(name: string, text: string): string | null {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const m = text.match(new RegExp(`^${escaped}\\s*=\\s*"([^"]+)"`, "m"));
  return m ? m[1] : null;
}

/** `npm outdated -g --json`. npm exits 1 when it has something to report, which is not an error. */
export function parseNpmOutdated(json: string): { name: string; current: string; latest: string }[] {
  let parsed: Record<string, { current?: string; latest?: string }>;
  try {
    parsed = JSON.parse(json);
  } catch {
    return [];
  }
  return Object.entries(parsed ?? {})
    .filter(([, v]) => v && typeof v === "object" && v.latest)
    .map(([name, v]) => ({ name, current: v.current ?? "?", latest: v.latest as string }));
}

/**
 * `npm view <pkg> deprecated` → the deprecation message, or null when the package is live.
 *
 * The second half of the same lesson as [`parseNpmGlobalTree`]: npm's newest version is not
 * always the right version. `@mariozechner/pi-agent-core` publishes 0.73.1 and npm marks it
 * "please use @earendil-works/pi-agent-core instead going forward" — the whole scope was
 * renamed. A report that offered 0.52.12 → 0.73.1 as an upgrade would be telling a reader to
 * install a newer release of something its own registry says to leave.
 */
export function parseNpmDeprecated(text: string): string | null {
  const line = text.trim().split("\n")[0]?.trim() ?? "";
  if (!line || line.startsWith("npm error") || line.startsWith("npm warn")) return null;
  return line;
}

/**
 * `npm ls -g --json --all` → the inventory AND which globals constrain which.
 *
 * The constraint map is the fix for a defect this tool shipped for one day: `npm outdated -g`
 * reports the newest release of every package it can see, including packages that exist at the
 * top level only because another global hoisted them. On this host it recommended
 * `@mariozechner/pi-agent-core` 0.52.12 → 0.73.1 and `@sinclair/typebox` 0.34.48 → 0.34.52,
 * both of which are `^0.52.12` / `^0.34.48` requirements of `claude-agent-sdk-pi` and
 * `@marckrenn/pi-sub-bar`. `^0.52.12` does not admit 0.73.1, so "upgrading" one would have
 * installed a duplicate beside the pinned copy, or broken the parent — and npm's own `wanted`
 * column cannot see this, because the range that constrains it belongs to a different package.
 *
 * Exits non-zero when the tree has unmet dependencies, which is not a failure: the JSON is still
 * printed, and a broken tree is exactly when a reader needs the report. Same rule as
 * `tools/storage`'s exit code.
 */
export function parseNpmGlobalTree(json: string): {
  installed: { name: string; version: string }[];
  /** Every node in the tree, not just the top level — the CVE inventory [`buildInventory`]
   *  scans. Deduped by name@version, so two copies of one package at different versions both
   *  appear while a package required by five parents appears once. */
  tree: { name: string; version: string }[];
  /** For each top-level package, the parents that carry it in their subtree — with the version
   *  that parent resolved it to, which is what separates a pin from a leftover. */
  requiredBy: Map<string, { parent: string; version: string }[]>;
} {
  let parsed: { dependencies?: Record<string, any> };
  try {
    parsed = JSON.parse(json);
  } catch {
    return { installed: [], tree: [], requiredBy: new Map() };
  }
  const root = parsed?.dependencies ?? {};
  const installed = Object.entries(root)
    .filter(([, v]) => v && v.version)
    .map(([name, v]) => ({ name, version: v.version as string }));

  const treeSeen = new Set<string>();
  const tree: { name: string; version: string }[] = [];
  const note = (name: string, version: unknown) => {
    const v = typeof version === "string" ? version : "";
    if (!v) return;
    const key = `${name}@${v}`;
    if (treeSeen.has(key)) return;
    treeSeen.add(key);
    tree.push({ name, version: v });
  };

  const names = Object.keys(root);
  const collect = (node: any, out: Map<string, string>) => {
    for (const [name, child] of Object.entries<any>(node?.dependencies ?? {})) {
      // First writer wins: a parent that resolves a name twice is a tree npm does not build.
      if (!out.has(name)) out.set(name, String(child?.version ?? "?"));
      note(name, child?.version);
      collect(child, out);
    }
  };
  for (const [name, v] of Object.entries<any>(root)) note(name, (v as any)?.version);

  const requiredBy = new Map<string, { parent: string; version: string }[]>();
  for (const [parent, node] of Object.entries(root)) {
    const sub = new Map<string, string>();
    collect(node, sub);
    for (const name of names) {
      if (name === parent || !sub.has(name)) continue;
      requiredBy.set(name, [...(requiredBy.get(name) ?? []), { parent, version: sub.get(name)! }]);
    }
  }
  return {
    installed,
    tree,
    requiredBy: new Map(
      [...requiredBy].map(([k, v]) => [k, v.sort((a, b) => a.parent.localeCompare(b.parent))]),
    ),
  };
}

/** One installed thing, as `tools/audit` needs it. `ecosystem` is the OSV ecosystem string. */
export type InventoryEntry = { ecosystem: "npm" | "crates.io"; name: string; version: string };

/**
 * Everything installed outside this checkout, as name+version pairs — the input `tools/audit`
 * scans for CVEs.
 *
 * Deliberately NOT `rows`. A row is the actionable view: one per package the operator can move,
 * and for npm that is the top level only. A CVE does not stop at the top level. On this host
 * `npm ls -g --all` yields 13 top-level packages and 1825 nodes in total, and the nested 1812
 * are exactly what a lockfile scan would have covered if npm kept a lockfile for globals.
 *
 * crates.io entries are top-level too, because that is all `cargo install --list` reports; an
 * installed crate's own transitive tree is its published `Cargo.lock`, which `tools/audit`
 * reads from the registry rather than reconstructing here.
 *
 * The two managers are re-read rather than threaded through `buildReport`, because the parsers
 * are the same ones and the flag is opt-in — `--inventory` is what `tools/audit` asks for, and
 * the dashboard's payload should not carry 1800 entries every four seconds.
 */
export function buildInventory(ctx: Ctx): InventoryEntry[] {
  const out: InventoryEntry[] = [];
  if (ctx.have("npm")) {
    const { tree } = parseNpmGlobalTree(ctx.run(["npm", "ls", "-g", "--json", "--all"]).stdout);
    for (const p of tree) out.push({ ecosystem: "npm", name: p.name, version: p.version });
  }
  if (ctx.have("cargo")) {
    for (const c of parseCargoInstallList(ctx.run(["cargo", "install", "--list"]).stdout)) {
      out.push({ ecosystem: "crates.io", name: c.name, version: c.version });
    }
  }
  return out;
}

/** `brew outdated --json=v2`. */
export function parseBrewOutdated(json: string): { name: string; installed: string; latest: string }[] {
  let parsed: { formulae?: any[]; casks?: any[] };
  try {
    parsed = JSON.parse(json);
  } catch {
    return [];
  }
  const one = (e: any, kind: string) => ({
    name: `${String(e?.name ?? "?")}${kind === "cask" ? " (cask)" : ""}`,
    installed: String(e?.installed_versions?.[0] ?? "?"),
    latest: String(e?.current_version ?? "?"),
  });
  return [
    ...(parsed?.formulae ?? []).map((e) => one(e, "formula")),
    ...(parsed?.casks ?? []).map((e) => one(e, "cask")),
  ];
}

/** `brew list --formula` — used only to spot a global npm package that shadows a brew formula. */
export function parseBrewFormulae(text: string): Set<string> {
  return new Set(text.split("\n").map((l) => l.trim()).filter(Boolean));
}

/** `rustup check`: "… - up to date: 1.99.0" or "… - Update available : 1.98.0 -> 1.99.0". */
export function parseRustupCheck(text: string): { component: string; installed: string; latest?: string }[] {
  const out: { component: string; installed: string; latest?: string }[] = [];
  for (const line of text.split("\n")) {
    const upd = line.match(/^(\S+)\s+-\s+update available\s*:\s*(\S+)\s*->\s*(\S+)/i);
    if (upd) {
      out.push({ component: upd[1], installed: upd[2], latest: upd[3] });
      continue;
    }
    const ok = line.match(/^(\S+)\s+-\s+up to date\s*:\s*(\S+)/i);
    if (ok) out.push({ component: ok[1], installed: ok[2] });
  }
  return out;
}

/** `<overlay>/data/host-patch/last.json` and its container-refresh sibling. */
export type Receipt = { at?: string; ran?: string; skipped?: string; failed?: string; audit?: string };

export function parseReceipt(json: string): Receipt | null {
  try {
    const r = JSON.parse(json);
    return r && typeof r === "object" ? (r as Receipt) : null;
  } catch {
    return null;
  }
}

export type ReceiptSummary = { ageH: number; audit: string; failed: string; ran: string } | null;

/**
 * Where `apply` records what it is doing, and what it did. The same shape as the two scheduled
 * jobs' receipts and for the same reason: a caller that cannot wait for the answer — the
 * dashboard's apply button, whose cargo step compiles for minutes — needs a file to read rather
 * than a request to hold open. A browser fetch that timed out while the install succeeded is the
 * outcome this file exists to prevent.
 */
export type ApplyReceipt = {
  at?: string;
  class?: string;
  steps?: number;
  failed?: number;
  stillStale?: number;
  state?: "running" | "done" | "failed";
  /** tools/audit's verdict, taken immediately after the steps above. Absent on a receipt
   *  written before this field existed, which is why it is optional rather than defaulted. */
  audit?: AuditVerdict;
};

/**
 * tools/audit's own exit contract, restated as a value: 0 clean · 1 a finding · 2 a scanner is
 * not installed. Anything else is the audit failing to run at all, which is a third thing and
 * must not be folded into either of the first two — an audit that did not run is not evidence
 * that the machine is clean.
 */
export type AuditVerdict = "clean" | "finding(s)" | "scanner-missing" | "could not run";

export function auditVerdict(code: number): AuditVerdict {
  if (code === 0) return "clean";
  if (code === 1) return "finding(s)";
  if (code === 2) return "scanner-missing";
  return "could not run";
}

/** What to do about a verdict, in the one line a receipt can carry. */
export const AUDIT_ADVICE: Record<AuditVerdict, string> = {
  clean: "",
  "finding(s)": " — run tools/audit for the detail",
  "scanner-missing": " — a scanner is not installed, so nothing was scanned",
  "could not run": " — tools/audit did not run",
};

/**
 * The audit that closes an apply.
 *
 * It runs on EVERY apply, including one whose plan already delegated to host-patch — and that
 * duplication is chosen rather than overlooked. host-patch audits too, but it writes its
 * verdict into its own receipt; reading that back here would make this field mean "whatever
 * ran last, wherever" and would need a third branch for a missing or stale receipt. One extra
 * read-only pass over a machine that is already minutes into an install is cheaper than a
 * field nobody can define, and it makes `audit` on this receipt mean exactly one thing: the
 * verdict taken immediately after these steps finished.
 *
 * Report-only. The caller's exit code stays a statement about the install, because an audit
 * finding is a fact about the machine and not about whether the upgrade worked. That is the
 * same shape Q77 gave the image scan in .github/workflows/security.yml.
 */
export function runAudit(ctx: Ctx): AuditVerdict {
  return auditVerdict(ctx.run([join(ctx.root, "tools", "audit")]).code);
}

export function applyReceiptPath(overlay: string): string {
  return join(overlay, "data", "updates", "last-apply.json");
}

export function readApplyReceipt(overlay: string): ApplyReceipt | null {
  const path = applyReceiptPath(overlay);
  if (!existsSync(path)) return null;
  try {
    return JSON.parse(readFileSync(path, "utf8")) as ApplyReceipt;
  } catch {
    return null;
  }
}

export function writeApplyReceipt(overlay: string, receipt: ApplyReceipt): void {
  const path = applyReceiptPath(overlay);
  try {
    mkdirSync(join(overlay, "data", "updates"), { recursive: true });
    writeFileSync(path, `${JSON.stringify(receipt)}\n`);
  } catch {
    // A receipt that cannot be written is not a reason to abandon an install that is already
    // running: the CLI prints its result either way, and only the panel loses the progress line.
  }
}

/**
 * The scheduled job's own receipt — the only honest answer to "did the daily job run". A launchd
 * StartInterval unit does not fire while the Mac sleeps, so "scheduled" and "ran" are different
 * questions, and doctor asks the same one.
 */
export function receiptSummary(path: string): ReceiptSummary {
  if (!existsSync(path)) return null;
  const r = parseReceipt(readFileSync(path, "utf8"));
  if (!r?.at || !Number.isFinite(Date.parse(r.at))) return null;
  return {
    ageH: (Date.now() - Date.parse(r.at)) / 3_600_000,
    audit: r.audit ?? "unknown",
    failed: (r.failed ?? "").trim(),
    ran: (r.ran ?? "").trim(),
  };
}

/**
 * The job's own verdict, as a note rather than a status. The audit's verdict describes the whole
 * machine — secrets, config, source scans — and is reported by tools/doctor; folding it into
 * "brew is stale" would name the wrong thing and send the reader to the wrong command.
 */
export function receiptNote(r: ReceiptSummary): string {
  const bits = [`job last ran ${r.ageH < 24 ? `${Math.round(r.ageH)}h` : `${Math.round(r.ageH / 24)}d`} ago`];
  if (r.failed) bits.push(`failed steps:${r.failed}`);
  if (r.audit !== "clean") bits.push(`audit ${r.audit} — run tools/audit`);
  return bits.join(" · ");
}

export function receiptIsStale(r: ReceiptSummary): boolean {
  return r === null || r.ageH > 48 || Boolean(r.failed);
}

// ── version arithmetic ────────────────────────────────────────────────────────
// Only cargo needs it: brew, npm and rustup are asked "what is outdated" and answer it
// themselves. Deliberately not a full semver implementation — it compares release segments
// numerically and treats a pre-release as older than its release, which is what "should this
// crate be rebuilt" needs and no more.

export function versionNewer(latest: string, installed: string): boolean {
  const seg = (v: string) => {
    const [core, pre] = v.replace(/^v/, "").split("-", 2);
    return { parts: core.split(".").map((p) => parseInt(p, 10) || 0), pre: pre ?? "" };
  };
  const a = seg(latest);
  const b = seg(installed);
  for (let i = 0; i < Math.max(a.parts.length, b.parts.length); i++) {
    const x = a.parts[i] ?? 0;
    const y = b.parts[i] ?? 0;
    if (x !== y) return x > y;
  }
  if (a.pre === b.pre) return false;
  return a.pre === "";
}

// ── gatherers ─────────────────────────────────────────────────────────────────
// Each never throws: a missing binary or an unreachable registry becomes a row that says so,
// because a report that dies on the first absent tool answers no question at all.

/** Homebrew: the owner answers this itself, via its own `outdated`. */
export function gatherBrew(ctx: Ctx, receipt: ReceiptSummary): Row[] {
  // The display form, not the argv form: a reader wants the command they would type. `planApply`
  // builds the absolute argv from `ctx.root`, so only one of the two has to be right.
  const action = "tools/host-patch.sh";
  if (!ctx.have("brew")) return [mk("brew", { name: "brew", status: "n/a", note: "brew not installed" })];
  if (ctx.offline) {
    return [
      mk("brew", {
        name: "not checked (--offline)",
        status: "unknown",
        note: receipt ? receiptNote(receipt) : "no host-patch receipt — the job has never run",
        action,
      }),
    ];
  }
  const res = ctx.run(["brew", "outdated", "--json=v2"]);
  if (res.code !== 0) {
    return [mk("brew", { name: "brew outdated failed", status: "unknown", note: res.stderr.trim().split("\n")[0] || "no output" })];
  }
  const outdated = parseBrewOutdated(res.stdout);
  if (outdated.length === 0) {
    return [
      mk("brew", {
        name: "all formulae and casks current",
        status: "current",
        note: receipt ? receiptNote(receipt) : "no host-patch receipt — the job has never run",
      }),
    ];
  }
  return outdated.map((o) =>
    mk("brew", {
      name: o.name,
      status: "stale",
      installed: o.installed,
      latest: o.latest,
      action,
      note: "moved by its owner, not by this tool",
    }),
  );
}

/** uv tools: no "what is new" verb exists, so the nightly job's receipt is the honest answer. */
export function gatherUv(ctx: Ctx, receipt: ReceiptSummary): Row[] {
  // The display form, not the argv form: a reader wants the command they would type. `planApply`
  // builds the absolute argv from `ctx.root`, so only one of the two has to be right.
  const action = "tools/host-patch.sh";
  if (!ctx.have("uv")) return [mk("uv", { name: "uv", status: "n/a", note: "uv not installed" })];
  return [
    mk("uv", {
      name: "tools",
      status: receiptIsStale(receipt) ? "stale" : "current",
      note: receipt ? receiptNote(receipt) : "no host-patch receipt — the job has never run",
      action,
    }),
  ];
}

/** rustup: it answers for itself with `check`. */
export function gatherRustup(ctx: Ctx, receipt: ReceiptSummary): Row[] {
  // The display form, not the argv form: a reader wants the command they would type. `planApply`
  // builds the absolute argv from `ctx.root`, so only one of the two has to be right.
  const action = "tools/host-patch.sh";
  if (!ctx.have("rustup")) return [mk("rustup", { name: "rustup", status: "n/a", note: "rustup not installed" })];
  if (ctx.offline) {
    return [
      mk("rustup", {
        name: "not checked (--offline)",
        status: "unknown",
        note: receipt ? receiptNote(receipt) : "no host-patch receipt",
        action,
      }),
    ];
  }
  const res = ctx.run(["rustup", "check"]);
  const components = parseRustupCheck(res.stdout);
  if (res.code !== 0 || components.length === 0) {
    return [mk("rustup", { name: "rustup check failed", status: "unknown", note: res.stderr.trim().split("\n")[0] || "no output" })];
  }
  const behind = components.filter((c) => c.latest);
  if (behind.length === 0) {
    const first = components[0];
    return [mk("rustup", { name: first.component, status: "current", installed: first.installed, note: "up to date" })];
  }
  return behind.map((c) =>
    mk("rustup", { name: c.component, status: "stale", installed: c.installed, latest: c.latest, action }),
  );
}

/** Containers: reported from container-refresh's receipt; never pulled here. */
export function gatherContainers(ctx: Ctx, overlay: string): Row[] {
  const path = join(overlay, "data", "container-refresh", "last.json");
  if (!existsSync(path)) return [mk("containers", { name: "last run", status: "n/a", note: "not enabled on this machine" })];
  const r = parseReceipt(readFileSync(path, "utf8"));
  const ageH = r?.at ? (Date.now() - Date.parse(r.at)) / 3_600_000 : NaN;
  if (!Number.isFinite(ageH)) return [mk("containers", { name: "last run", status: "unknown", note: "receipt unreadable" })];
  const failed = (r?.failed ?? "").trim();
  return [
    mk("containers", {
      name: "last run",
      status: ageH > 48 || failed ? "stale" : "current",
      note: `ran ${Math.round(ageH)}h ago${failed ? ` · failed:${failed}` : ""}`,
    }),
  ];
}

/**
 * graphify and interceptor integrations. Both markers record the DATE of the last re-derivation,
 * never a version (Q77 deleted every pin), so there is nothing to compare — the row states the
 * date it read and whether any harness reports itself stale. The harness config dirs come from
 * agent-integrations' own JSON rather than a table here: a second copy of that mapping is the
 * "second copy of the same logic" this repository forbids, and it would drift the day a harness
 * is added there.
 */
export function gatherIntegrations(ctx: Ctx): Row[] {
  const script = join(ctx.root, "tools", "agent-integrations.sh");
  if (!existsSync(script)) {
    return [mk("graphify", { name: "integration", status: "unknown", note: "tools/agent-integrations.sh is missing" })];
  }
  const res = ctx.run([script, "status", "--json"]);
  if (res.code !== 0) {
    return [mk("graphify", { name: "integration", status: "unknown", note: "agent-integrations status failed" })];
  }
  let payload: any;
  try {
    payload = JSON.parse(res.stdout);
  } catch {
    return [mk("graphify", { name: "integration", status: "unknown", note: "agent-integrations emitted no JSON" })];
  }

  const out: Row[] = [];
  for (const id of ["graphify", "interceptor"] as const) {
    const entry = (payload?.integrations ?? []).find((i: any) => i.upstream === id);
    const harnesses: any[] = entry?.harnesses ?? [];
    if (harnesses.length === 0) {
      out.push(mk(id, { name: "integration", status: "n/a", note: "no harness reported" }));
      continue;
    }
    const markerName = id === "graphify" ? ".graphify-upstream-installed" : ".interceptor-skills-axoned";
    let oldestDays: number | null = null;
    let oldestDate = "";
    for (const h of harnesses) {
      if (!h.config_dir) continue;
      const marker = join(String(h.config_dir), markerName);
      if (!existsSync(marker)) continue;
      const date = readFileSync(marker, "utf8").trim();
      const t = Date.parse(date);
      if (!Number.isFinite(t)) continue;
      const days = Math.floor((Date.now() - t) / 86_400_000);
      if (oldestDays === null || days > oldestDays) {
        oldestDays = days;
        oldestDate = date;
      }
    }
    const integrated = harnesses.filter((h) => h.state === "integrated").length;
    const named = harnesses.filter((h) => h.state !== "missing" && h.state !== "integrated").map((h) => String(h.name));
    const bits: string[] = [`${integrated}/${harnesses.length} harness(es) integrated`];
    if (oldestDays !== null) bits.push(`files re-derived from upstream ${oldestDate} (${oldestDays}d)`);
    else bits.push("no marker — never derived from upstream");
    if (named.length) bits.push(`not integrated: ${named.join(", ")}`);
    // 30 days, and it is a judgement rather than a measurement: with no version to compare
    // against, the row states the date so a reader can disagree with the threshold.
    const stale = oldestDays === null || oldestDays > 30;
    out.push(
      mk(id, {
        name: "harness integration",
        status: stale ? "stale" : "current",
        note: bits.join(" · "),
        action: `tools/agent-integrations.sh update ${id}`,
      }),
    );
  }
  return out;
}

/** The checkout itself — `tools/update.sh --check` semantics, without pulling. */
export function gatherCheckout(ctx: Ctx): Row[] {
  if (!existsSync(join(ctx.root, "tools", "update.sh"))) {
    return [mk("checkout", { name: "this repo", status: "unknown", note: "tools/update.sh missing" })];
  }
  const ahead = ctx.run(["git", "-C", ctx.root, "rev-list", "--count", "origin/main..HEAD"]);
  const behind = ctx.run(["git", "-C", ctx.root, "rev-list", "--count", "HEAD..origin/main"]);
  const a = parseInt(ahead.stdout.trim(), 10);
  const b = parseInt(behind.stdout.trim(), 10);
  if (!Number.isFinite(a) || !Number.isFinite(b)) {
    return [mk("checkout", { name: "this repo", status: "unknown", note: "no origin/main to compare against" })];
  }
  const dirty = ctx.run(["git", "-C", ctx.root, "status", "--porcelain"]);
  const dirtyCount = dirty.stdout.split("\n").filter((l) => l.trim()).length;
  const bits = [`${a} ahead / ${b} behind origin/main`];
  if (dirtyCount) bits.push(`${dirtyCount} uncommitted change(s)`);
  return [
    mk("checkout", {
      name: "this repo",
      status: b > 0 ? "stale" : "current",
      note: bits.join(" · "),
      action: "tools/update.sh",
    }),
  ];
}

/** cargo: enumerated locally, newest release looked up one crate at a time. */
export function gatherCargo(ctx: Ctx): Row[] {
  if (!ctx.have("cargo")) return [mk("cargo", { name: "cargo", status: "n/a", note: "cargo not installed" })];
  const installed = parseCargoInstallList(ctx.run(["cargo", "install", "--list"]).stdout);
  if (installed.length === 0) {
    return [mk("cargo", { name: "crates", status: "current", note: "nothing installed with `cargo install`" })];
  }
  return installed.map((c) => {
    if (ctx.offline) {
      return mk("cargo", {
        name: c.name,
        status: "unknown",
        installed: c.version,
        note: "not checked (--offline)",
        action: `cargo install ${c.name} --locked --force`,
      });
    }

    // crates.io first, because only its full version list can say what the newest STABLE is.
    // `curl` rather than `fetch` so this stays inside the injected runner and the tests can
    // drive it. The User-Agent is required: crates.io answers 403 without one (measured
    // 2026-10-01). A failure here is not an error, it is the fallback below.
    const api = ctx.run([
      "curl",
      "-sS",
      "-A",
      "sjel-updates",
      `https://crates.io/api/v1/crates/${c.name}/versions`,
    ]);
    const { stable, newest } = parseCratesIoVersions(api.stdout);

    let latest = stable;
    let preOnly: string | null = null;
    if (!latest) {
      if (newest) {
        // Only pre-releases are published, or the API answered but had no stable release.
        preOnly = newest;
      } else {
        // No API answer at all: `cargo search` is the fallback, and it can only report the max.
        const found = ctx.run(["cargo", "search", c.name, "--limit", "1"]);
        const max = found.code === 0 ? parseCargoSearch(c.name, found.stdout) : null;
        if (max) {
          if (max.includes("-")) preOnly = max;
          else latest = max;
        }
      }
    }

    if (!latest && !preOnly) {
      return mk("cargo", { name: c.name, status: "unknown", installed: c.version, note: "registry did not answer" });
    }
    if (!latest && preOnly) {
      // Nothing stable to move to. Named and left alone: adopting an alpha because it sorts
      // higher is exactly the decision a report must not make silently.
      return mk("cargo", {
        name: c.name,
        status: "current",
        installed: c.version,
        latest: preOnly,
        note: `newer pre-release ${preOnly} exists — not adopted`,
      });
    }

    const stale = versionNewer(latest!, c.version);
    // A pre-release above the newest stable is worth naming even when there IS a stable upgrade,
    // so a reader can see the whole picture rather than only the row this tool chose to act on.
    const preNote = newest && newest !== latest ? `newer pre-release ${newest} exists — not adopted` : undefined;
    return mk("cargo", {
      name: c.name,
      status: stale ? "stale" : "current",
      installed: c.version,
      latest,
      action: stale ? `cargo install ${c.name} --locked --force` : undefined,
      note: preNote,
    });
  });
}

/** npm global packages. npm exits 1 when it has something to report, which is not an error. */
export function gatherNpm(ctx: Ctx, brewFormulae: Set<string>): Row[] {
  if (!ctx.have("npm")) return [mk("npm", { name: "npm", status: "n/a", note: "npm not installed" })];
  // One call for both facts: `--all` carries the subtree, which is what says whether a package
  // is independently upgradable. A second `--depth=0` call would answer less for the same cost.
  const { installed, requiredBy } = parseNpmGlobalTree(ctx.run(["npm", "ls", "-g", "--json", "--all"]).stdout);
  if (installed.length === 0) return [mk("npm", { name: "packages", status: "current", note: "no global packages" })];

  if (ctx.offline) {
    return installed.map((p) =>
      mk("npm", {
        name: p.name,
        status: "unknown",
        installed: p.version,
        note: "not checked (--offline)",
        action: `npm install -g ${p.name}@latest`,
      }),
    );
  }
  const outdated = parseNpmOutdated(ctx.run(["npm", "outdated", "-g", "--json"]).stdout);
  const byName = new Map(outdated.map((o) => [o.name, o]));
  return installed.map((p) => {
    // The shadowing check: a global npm package whose name is also a brew formula can put two
    // binaries under one name on PATH — the shape this file's own header describes being paid for
    // once already (PRD §13). It is a hint to check, not a claim that they are the same project:
    // npm's `uv` and brew's `uv` are unrelated packages that happen to collide on the name.
    const shadowed = brewFormulae.has(p.name);
    const shadowNote = shadowed ? "name is also a brew formula — check which one PATH resolves" : undefined;
    const reqs = requiredBy.get(p.name) ?? [];
    // A parent pins this copy only when it resolved the SAME version — that is the hoisted case,
    // where the top-level package exists to satisfy someone. A parent that resolved a different
    // version bundled its own copy, which makes the top-level one a leftover nothing requires.
    // Matching by name alone conflated the two and reported an unused duplicate as "pinned".
    const pinnedBy = reqs.filter((r) => r.version === p.version).map((r) => r.parent);
    const bundled = reqs.filter((r) => r.version !== p.version);
    const o = byName.get(p.name);
    if (!o) return mk("npm", { name: p.name, status: "current", installed: p.version, note: shadowNote });

    if (pinnedBy.length > 0) {
      return mk("npm", {
        name: p.name,
        status: "stale",
        installed: o.current,
        latest: o.latest,
        note: [`pinned by ${pinnedBy.join(", ")} — upgrade those instead`, shadowNote].filter(Boolean).join(" · "),
      });
    }

    // Deprecated: the registry itself says not to use this. Checked only for rows that are
    // otherwise actionable, so it costs one call per genuinely-outdated package and never one
    // per installed package.
    const deprecation = parseNpmDeprecated(ctx.run(["npm", "view", p.name, "deprecated"]).stdout);
    if (deprecation) {
      return mk("npm", {
        name: p.name,
        status: "stale",
        installed: o.current,
        latest: o.latest,
        removable: true,
        note: [`deprecated — ${deprecation}`, shadowNote].filter(Boolean).join(" · "),
      });
    }

    // A leftover duplicate: nothing requires this copy at this version, and the parents that
    // mention the name bundle their own. Upgrading it would install a third copy, so there is
    // no action — the honest advice is removal, which is the reader's call and not this tool's.
    if (bundled.length > 0) {
      return mk("npm", {
        name: p.name,
        status: "stale",
        installed: o.current,
        latest: o.latest,
        removable: true,
        note: [
          `unused duplicate — ${bundled.map((b) => `${b.parent} bundles its own ${b.version}`).join(", ")}`,
          shadowNote,
        ]
          .filter(Boolean)
          .join(" · "),
      });
    }

    return mk("npm", {
      name: p.name,
      status: "stale",
      installed: o.current,
      latest: o.latest,
      action: `npm install -g ${p.name}@latest`,
      note: shadowNote,
    });
  });
}

/** Apps that update themselves. Named so the table is complete rather than silently partial. */
export function gatherVendor(ctx: Ctx): Row[] {
  const out: Row[] = [];
  for (const [bin, label] of [
    ["pi", "pi coding agent"],
    ["claude", "Claude Code"],
    ["codex", "Codex"],
    ["opencode", "opencode"],
  ] as const) {
    if (ctx.have(bin)) out.push(mk("vendor", { name: label, status: "n/a", note: "self-updating" }));
  }
  const ollama = ctx.have("ollama");
  if (ollama) {
    out.push(
      mk("vendor", {
        name: "ollama",
        // Ollama.app installs to /usr/local/bin and is neither a brew formula nor a cask on this
        // host (verified 2026-10-01), so the nightly sweep does not reach it.
        status: "n/a",
        note: ollama.includes(".app/")
          ? "app-managed (/Applications/Ollama.app)"
          : `at ${ollama} — not brew-managed; update through the app`,
      }),
    );
  }
  if (out.length === 0) out.push(mk("vendor", { name: "apps", status: "n/a", note: "none found" }));
  return out;
}

// ── report ────────────────────────────────────────────────────────────────────

export function buildReport(ctx: Ctx): { rows: Row[]; generatedAt: string; lastApply: ApplyReceipt | null } {
  const receipt = receiptSummary(join(ctx.overlay, "data", "host-patch", "last.json"));
  const brewFormulae = ctx.have("brew") && !ctx.offline ? parseBrewFormulae(ctx.run(["brew", "list", "--formula"]).stdout) : new Set<string>();
  const rows = [
    ...gatherBrew(ctx, receipt),
    ...gatherUv(ctx, receipt),
    ...gatherRustup(ctx, receipt),
    ...gatherContainers(ctx, ctx.overlay),
    ...gatherIntegrations(ctx),
    ...gatherCheckout(ctx),
    ...gatherCargo(ctx),
    ...gatherNpm(ctx, brewFormulae),
    ...gatherVendor(ctx),
  ];
  return { rows, generatedAt: new Date().toISOString(), lastApply: readApplyReceipt(ctx.overlay) };
}

export function grouped(rows: Row[]): { surface: Surface; rows: Row[] }[] {
  return SURFACES.map((s) => ({ surface: s, rows: rows.filter((r) => r.surface === s.id) })).filter((g) => g.rows.length > 0);
}

const MARK: Record<Status, string> = { current: "✓", stale: "✗", unknown: "?", "n/a": "·" };
const HEADING: Record<Owner, string> = {
  scheduled: "Managed by a scheduled job",
  manual: "Manual — a verb exists, nothing schedules it",
  unowned: "Unowned — nothing moves these",
  self: "Self-managed — the vendor updates these",
};

export function renderTable(rows: Row[], offline: boolean, lastApply: ApplyReceipt | null = null): string {
  const out: string[] = [`sjel update — software installed outside this checkout${offline ? " (offline)" : ""}`, ""];
  let lastOwner: Owner | null = null;
  for (const { surface: s, rows: rs } of grouped(rows)) {
    if (s.owner !== lastOwner) {
      out.push(HEADING[s.owner]);
      lastOwner = s.owner;
    }
    out.push(`  ${s.title}  [${s.ownerDetail}]`);
    for (const r of rs) {
      const name = r.name ? ` ${r.name}` : "";
      const vers = r.installed ? ` ${r.installed}${r.latest && r.latest !== r.installed ? ` → ${r.latest}` : ""}` : "";
      out.push(`    ${MARK[r.status]}${name}${vers}${r.note ? `  ${r.note}` : ""}`);
      if (r.status === "stale" && r.action) out.push(`      → ${r.action}`);
    }
  }
  const stale = rows.filter((r) => r.status === "stale");
  const unknown = rows.filter((r) => r.status === "unknown");
  out.push("");
  if (lastApply?.state === "running") {
    out.push(`an apply is running: ${lastApply.class} started ${Math.round((Date.now() - Date.parse(lastApply.at ?? "")) / 60_000)}m ago`);
  } else if (lastApply?.at) {
    out.push(
      `last apply: ${lastApply.class} ${lastApply.state ?? "done"} · ${lastApply.steps ?? 0} step(s)` +
        `${lastApply.failed ? `, ${lastApply.failed} failed` : ""}` +
        `${lastApply.audit ? ` · audit ${lastApply.audit}` : ""}`,
    );
  }
  if (stale.length === 0) {
    out.push(unknown.length ? `nothing stale · ${unknown.length} not checked` : "nothing stale");
  } else {
    const byOwner = [...new Set(stale.map((r) => r.owner))].sort();
    out.push(`${stale.length} stale (${byOwner.join(", ")}) · 'sjel update apply' moves what this tool owns`);
  }
  return out.join("\n");
}

export function renderJson(
  rows: Row[],
  generatedAt: string,
  offline: boolean,
  lastApply: ApplyReceipt | null = null,
  inventory?: InventoryEntry[],
): string {
  // `inventory` is present only when it was asked for: it is two orders of magnitude larger
  // than `rows`, and the dashboard reads this payload every four seconds while an apply runs.
  return JSON.stringify(
    { generatedAt, offline, lastApply, surfaces: SURFACES, rows, ...(inventory ? { inventory } : {}) },
    null,
    2,
  );
}

// ── apply ─────────────────────────────────────────────────────────────────────
// Split by owner on purpose. `delegated` execs the tools that already own a class; `direct` runs
// the steps for the two classes nothing owns. Keeping the lists apart is what stops this file
// growing a second `brew upgrade`.

export type Step = { surfaceId: string; label: string; argv: string[]; slow?: boolean; note?: string };

/**
 * Rows `--prune` removes: leftovers nothing requires, and only those.
 *
 * Narrow on purpose, and the narrowness is the safety property. A row marked `removable` is
 * either deprecated by the registry or a duplicate the parents that mention it replace with
 * their own copy. A row a parent PINS is never here, whatever its status — that copy exists to
 * satisfy someone, and deleting it breaks its parent. Anything this list gets wrong is a
 * package somebody wanted, deleted, so `apply` prints the whole list before running and still
 * needs `--yes` when it is not on a terminal.
 */
export function planPrune(rows: Row[]): Row[] {
  return rows.filter((r) => r.removable === true);
}

export function planApply(
  rows: Row[],
  only: string[],
  ctx: Ctx,
  opts: { prune?: boolean; reResolve?: string[] } = {},
): Step[] {
  const stale = rows.filter((r) => r.status === "stale");
  const wanted = (id: string) => only.length === 0 || only.includes(id);
  const steps: Step[] = [];

  // Removals first: a leftover going out frees the name before anything new arrives under it.
  if (opts.prune && wanted("npm")) {
    for (const r of planPrune(rows)) {
      steps.push({
        surfaceId: "npm",
        label: `prune: ${r.name}`,
        argv: ["npm", "uninstall", "-g", r.name],
        note: r.note,
      });
    }
  }

  // Delegated. The host-patch job is one job covering three managers, so one step runs it and
  // one step calls the owner, not three.
  const hostManaged = (["brew", "uv", "rustup"] as const).some(
    (id) => wanted(id) && stale.some((r) => r.surface === id),
  );
  if (hostManaged) {
    steps.push({
      surfaceId: "hostpatch",
      label: "brew, uv and rustup — via their owner (capabilities/host-patch)",
      argv: [join(ctx.root, "tools", "host-patch.sh")],
      slow: true,
    });
  }
  for (const id of ["graphify", "interceptor"] as const) {
    if (!wanted(id) || !stale.some((r) => r.surface === id)) continue;
    steps.push({
      surfaceId: id,
      label: `${id} harness integration`,
      argv: [join(ctx.root, "tools", "agent-integrations.sh"), "update", id],
    });
  }

  // Direct: the two classes nothing else owns.
  const reResolve = opts.reResolve ?? [];
  for (const r of stale) {
    if (r.surface === "cargo" && wanted("cargo") && r.action) {
      // `--locked` is the default because the published lockfile is what makes an install
      // reproducible. The one case where that costs more than it buys is a crate whose
      // published lockfile ALREADY pins a flagged dependency: reinstalling with --locked
      // reproduces the very tree the audit just named. So the exception is per-crate and the
      // operator's — they read the finding, they name the crate — rather than a policy that
      // silently re-resolves everything.
      const dropping = reResolve.includes(r.name);
      steps.push({
        surfaceId: "cargo",
        label: `cargo: ${r.name}`,
        argv: dropping ? ["cargo", "install", r.name, "--force"] : r.action.split(" "),
        note: dropping ? "re-resolving — --locked dropped, its published lockfile pins a flagged dependency" : undefined,
        slow: true,
      });
    }
    if (r.surface === "npm" && wanted("npm") && r.action) {
      steps.push({ surfaceId: "npm", label: `npm: ${r.name}`, argv: r.action.split(" ") });
    }
  }
  return steps;
}

// ── cli ───────────────────────────────────────────────────────────────────────

const HELP = `sjel update — every piece of software installed outside this checkout.

  sjel update                   report: who owns moving each class, and what is stale
  sjel update --json            the same, machine-readable (surfaces + rows)
  sjel update --offline         receipts and installed versions only; no registry is asked
  sjel update --json --inventory
                                add an 'inventory' array: every installed npm node and cargo
                                crate, not just the actionable rows. This is what tools/audit
                                scans, and it is why the flag exists — a CVE does not stop at
                                the top level, and 'rows' is the top level only
  sjel update apply [--only <class>...] [--yes]
                                move what this tool owns, and delegate the rest
  sjel update apply --prune     also REMOVE leftovers nothing requires: packages the registry
                                has deprecated and duplicates whose parents bundle their own
                                copy. The list is printed first; a package a parent pins is
                                never included. This is the only destructive mode
  sjel update apply --only cargo --re-resolve <crate,...>
                                reinstall the named crates WITHOUT --locked, for a crate whose
                                published lockfile pins a dependency the audit has flagged
  sjel update -h

Classes for --only:
  brew uv rustup containers graphify interceptor checkout cargo npm vendor

Report exits 1 when something is stale, 0 when nothing is. Apply exits 1 when there was
nothing to do (or you declined), 2 when a step failed.`;

export type Options = {
  verb: string;
  json: boolean;
  offline: boolean;
  yes: boolean;
  only: string[];
  inventory: boolean;
  prune: boolean;
  reResolve: string[];
};

export function parseArgs(argv: string[]): Options {
  // A leading flag means the default verb, not a verb named '--offline': `sjel update --offline`
  // is a report, and `apply` is the only verb that reads as a verb.
  const rest = argv.filter((a) => a !== "");
  const hasVerb = rest.length > 0 && !rest[0].startsWith("-");
  const verb = hasVerb ? rest[0] : "report";
  const tail = hasVerb ? rest.slice(1) : rest;
  const out: Options = { verb, json: false, offline: false, yes: false, only: [], inventory: false, prune: false, reResolve: [] };
  for (let i = 0; i < tail.length; i++) {
    const a = tail[i];
    if (a === "--json") out.json = true;
    else if (a === "--offline") out.offline = true;
    else if (a === "--inventory") out.inventory = true;
    else if (a === "--yes" || a === "-y") out.yes = true;
    else if (a === "--prune") out.prune = true;
    else if (a === "--re-resolve") {
      const next = tail[++i];
      if (!next) throw new Error("--re-resolve needs a crate name");
      out.reResolve.push(...next.split(",").filter(Boolean));
    } else if (a === "--only") {
      const next = tail[++i];
      if (!next) throw new Error("--only needs a class id");
      out.only.push(...next.split(",").filter(Boolean));
    } else throw new Error(`unknown argument '${a}'`);
  }
  return out;
}

export function makeCtx(env: NodeJS.ProcessEnv, run: Runner = defaultRunner, have = defaultHave, offline = false): Ctx {
  const root = env.SJEL_UPDATES_ROOT || env.SJEL_ROOT;
  const overlay = env.SJEL_UPDATES_OVERLAY || env.SJEL_OVERLAY_ROOT;
  if (!root || !overlay) {
    throw new Error("run this through the launcher (tools/updates) — SJEL_UPDATES_ROOT/SJEL_UPDATES_OVERLAY unset");
  }
  return { run, have, root, overlay, offline };
}

export async function main(argv: string[], env: NodeJS.ProcessEnv = process.env): Promise<number> {
  if (argv.includes("-h") || argv.includes("--help") || argv[0] === "help") {
    console.log(HELP);
    return 0;
  }

  let opts: Options;
  try {
    opts = parseArgs(argv);
  } catch (e) {
    console.error(`sjel update: ${(e as Error).message}`);
    console.error("run 'sjel update -h'");
    return 2;
  }
  if (opts.verb !== "report" && opts.verb !== "apply") {
    console.error(`sjel update: unknown verb '${opts.verb}' (report|apply)`);
    return 2;
  }
  for (const id of opts.only) {
    try {
      surface(id);
    } catch {
      console.error(`sjel update: unknown class '${id}'`);
      return 2;
    }
  }
  // A flag that silently does nothing is a lie about what the caller asked for.
  if (opts.inventory && !opts.json) {
    console.error("sjel update: --inventory only means something with --json, where it is the audit's input");
    return 2;
  }
  for (const [flag, on] of [
    ["--prune", opts.prune],
    ["--re-resolve", opts.reResolve.length > 0],
  ] as const) {
    if (on && opts.verb !== "apply") {
      console.error(`sjel update: ${flag} only means something with 'apply'`);
      return 2;
    }
  }

  let ctx: Ctx;
  try {
    ctx = makeCtx(env, defaultRunner, defaultHave, opts.offline);
  } catch (e) {
    console.error(`sjel update: ${(e as Error).message}`);
    return 2;
  }

  const { rows, generatedAt, lastApply } = buildReport(ctx);

  if (opts.verb === "report") {
    if (opts.json) {
      console.log(renderJson(rows, generatedAt, opts.offline, lastApply, opts.inventory ? buildInventory(ctx) : undefined));
    } else {
      console.log(renderTable(rows, opts.offline, lastApply));
    }
    return rows.some((r) => r.status === "stale") ? 1 : 0;
  }

  const steps = planApply(rows, opts.only, ctx, { prune: opts.prune, reResolve: opts.reResolve });
  if (steps.length === 0) {
    console.log("sjel update: nothing to do — everything this tool can move is current");
    return 1;
  }
  console.log(`sjel update apply — ${steps.length} step(s)`);
  // The removals are printed as their own block before anything runs. `--prune` deletes, and
  // this list plus the confirmation below is the whole of what stands between the flag and a
  // package somebody wanted.
  const removals = steps.filter((s) => s.argv[0] === "npm" && s.argv[1] === "uninstall");
  if (removals.length > 0) {
    console.log(`  REMOVING ${removals.length} package(s) nothing requires:`);
    for (const s of removals) {
      console.log(`    ✗ ${s.label.replace(/^prune: /, "")}${s.note ? ` — ${s.note}` : ""}`);
    }
  }
  for (const s of steps) {
    if (removals.includes(s)) continue;
    console.log(`  · ${s.label}${s.note ? ` — ${s.note}` : ""}${s.slow ? "  (slow — this one compiles or pulls)" : ""}`);
  }
  // A crate named on the command line that nothing plans to move is said out loud, so a typo
  // does not read as "it re-resolved and it was fine".
  for (const name of opts.reResolve) {
    if (!steps.some((s) => s.label === `cargo: ${name}`)) {
      console.log(`  · --re-resolve named ${name}, which this plan does not move`);
    }
  }
  // Deliberately unlike tools/update.sh, which pulls straight through when stdin is not a TTY.
  // That tool fast-forwards a git checkout; this one installs software, and on this host the
  // plan can include two npm major versions. An unattended run therefore has to say --yes out
  // loud rather than inherit the silence of a pipe.
  if (!opts.yes && !process.stdin.isTTY) {
    console.error("sjel update: refusing to install unattended — re-run with --yes, or --only <class> to scope it");
    return 1;
  }
  if (!opts.yes) {
    process.stdout.write("proceed? [y/N] ");
    const answer = (await new Promise<string>((r) => process.stdin.once("data", (d) => r(d.toString())))).trim();
    if (!/^y(es)?$/i.test(answer)) {
      console.log("aborted");
      return 1;
    }
  }

  const scope = opts.only.length ? opts.only.join(",") : "all";
  writeApplyReceipt(ctx.overlay, { at: new Date().toISOString(), class: scope, steps: steps.length, state: "running" });

  let failed = 0;
  for (const s of steps) {
    console.log(`\n▸ ${s.label}`);
    const res = defaultRunner(s.argv);
    if (res.stdout) process.stdout.write(res.stdout);
    if (res.stderr) process.stderr.write(res.stderr);
    if (res.code !== 0) {
      failed++;
      console.error(`  ✗ ${s.label} failed (exit ${res.code}) — continuing`);
    }
  }

  // Re-read rather than assume: an upgrade that reported success and moved nothing is a thing
  // package managers do, and the second read is what makes this a report and not a claim.
  const after = buildReport(ctx);
  const stillStale = after.rows.filter((r) => r.status === "stale");

  // Last, over the machine the steps above just changed. This is the seam that was missing:
  // `apply --only cargo` and `--only npm` install software that no scan had ever looked at,
  // and the scheduled job's audit only ran when brew, uv or rustup happened to be stale too.
  console.log("\n▸ audit (tools/audit)");
  const audit = runAudit(ctx);

  writeApplyReceipt(ctx.overlay, {
    at: new Date().toISOString(),
    class: scope,
    steps: steps.length,
    failed,
    stillStale: stillStale.length,
    state: failed > 0 ? "failed" : "done",
    audit,
  });
  console.log(`\n── applied ${steps.length - failed}/${steps.length}; ${stillStale.length} still stale ──`);
  console.log(`── audit: ${audit}${AUDIT_ADVICE[audit]} ──`);
  for (const r of stillStale) {
    console.log(`  still stale: ${r.surface}${r.name ? ` ${r.name}` : ""}${r.installed ? ` ${r.installed}` : ""}`);
  }
  return failed > 0 ? 2 : 0;
}

if (import.meta.main) {
  process.exit(await main(process.argv.slice(2)));
}
