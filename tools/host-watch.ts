// tools/host-watch.ts — one pass of the host watch, then exit.
//
// Two host conditions are invisible until they hurt, and both bit on 2026-08-15
// (Axon#177). A System Settings Storage pane got stuck at 08:35 and burned 3h29m of CPU
// before anyone noticed; the only symptom was "the Mac feels slow", nine hours later.
// And the disk half was ALREADY solved — tools/storage answered it in seconds — but
// nothing ran it, so it may as well not have existed. A tool nobody runs and a tool
// nobody built are the same tool.
//
// So: notice, then hand off. This writes a row per finding into its OWN table and
// NOTHING when the machine is healthy, because a watcher that cries wolf gets muted and
// a muted watcher is worse than none. No new notification machinery — core Axon has
// never had a notifier and does not grow one here (the precedent tools/sparpreis-watch.ts
// states in its own header).
//
// It filed a `tasks` record until PRD Q48 (2026-08-27) retired that capability: the
// Action kind went back to the vault, and a runaway process is not an action a human
// wrote — it is machine state. So the findings became host-watch's own rows in the
// shared store (capabilities/store), read by axon-status and ranked on the dashboard's
// decision ladder at band 900.
//
// Owning the table changed one thing beyond the transport, and it had to. Under `tasks`
// the only way a finding ever closed was the operator pressing Done, which is why the
// id carried a generation counter. That button is gone, so nothing would ever close a
// row again — a watcher whose findings only accumulate is a watcher that stops meaning
// anything. A run now RESOLVES every open finding whose condition it no longer sees.
//
//   tools/host-watch              check, record findings
//   tools/host-watch --dry-run    check and print; write nothing
//   tools/host-watch --json       machine-readable findings
//   tools/host-watch -h           this help
//
// Exit 0 = checked (findings or not). 1 = could not check. 2 = no policy.
//
// A third condition joined the two in 2026-09: an unexpected wildcard listener, delegated
// whole to `host-net check --json` the same way free space is delegated to `tools/storage`.
// This file owns no port number, no process name and no scope rule for it.
//
// Knows no fact about this machine. Every budget, threshold and allowlisted process
// comes from the overlay's config/host-watch-policy.toml, the same split
// tools/storage already uses (README.md#generic-in-axon-specific-in-the-overlay).
// The pure functions below are exported for tools/host-watch.test.ts.

import { Database } from "bun:sqlite";
import { existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";

import { axonRoot, overlayRoot } from "./lib/overlay.ts";

// One finding title needs a byte count in the same units storage prints. This used to be
// imported from tools/storage.ts; storage became a Rust crate on 2026-09-03, so the four
// lines are here rather than in a TypeScript shim kept alive to export them. The
// definition storage owns is `fmt_bytes` in tools/storage/src/measure.rs, and the unit
// switch at 1 GB is the part that has to agree.
const GB = 1024 ** 3;
const fmt = (b: number) =>
  b >= GB ? `${(b / GB).toFixed(1)} GB` : `${Math.round(b / 1024 ** 2)} MB`;

const HELP = `tools/host-watch — notice a runaway process or a filling disk, once per run.

  tools/host-watch              check, record findings
  tools/host-watch --dry-run    check and print; write nothing
  tools/host-watch --json       machine-readable findings
  tools/host-watch -h           this help

Policy: <overlay>/config/host-watch-policy.toml
`;

/// Prefixes this capability's tables in the one shared SQLite file (PRD Q45):
/// `host_watch` here means the table `host_watch_findings`. Underscored, not hyphenated
/// — the capability's directory name is `host-watch` and a hyphen is not a legal bare
/// identifier in SQL.
const PREFIX = "host_watch";

export type Proc = { pid: number; cpuSeconds: number; elapsedSeconds: number; comm: string };
export type ProcessBudget = { min_cpu_seconds?: number; min_cpu_ratio?: number };
export type AllowedProcess = { comm: string; reason: string };
export type WatchPolicy = { process?: ProcessBudget; allow_process?: AllowedProcess[] };

export type Finding = { key: string; title: string; note: string };
export type ProcFinding = Finding & { pid: number; comm: string; ratio: number; cpuSeconds: number };

/** One row of `host_watch_findings`, as the decision below needs to see it. */
export type FindingRow = { id: string; key: string; generation: number; status: string };
export type Emission =
  | { action: "refresh"; id: string }
  | { action: "create"; id: string; generation: number };

/**
 * ps prints three shapes and only three: `DD-HH:MM:SS`, `HH:MM:SS`, and `MM:SS[.ff]`,
 * where MM is NOT bounded at 60 (a process with 168 minutes of CPU prints "168:50.56").
 * Anything else is a row we do not understand, and the honest value for that is 0 —
 * NaN would propagate into a ratio and a garbage row would become a finding.
 */
function hmsToSeconds(raw: string): number {
  const t = raw.trim();
  const day = /^(\d+)-(\d+):(\d+):(\d+(?:\.\d+)?)$/.exec(t);
  if (day) return +day[1] * 86400 + +day[2] * 3600 + +day[3] * 60 + +day[4];
  const hms = /^(\d+):(\d+):(\d+(?:\.\d+)?)$/.exec(t);
  if (hms) return +hms[1] * 3600 + +hms[2] * 60 + +hms[3];
  const ms = /^(\d+):(\d+(?:\.\d+)?)$/.exec(t);
  if (ms) return +ms[1] * 60 + +ms[2];
  return 0;
}

/** Cumulative CPU time a process has consumed, in seconds. */
export const parseCpuTime = (raw: string): number => hmsToSeconds(raw);

/** Wall-clock time a process has existed, in seconds. */
export const parseElapsed = (raw: string): number => hmsToSeconds(raw);

/**
 * `ps -Aceo pid,time,etime,comm`. The command is LAST because it is the only field that
 * contains spaces ("Spotify Helper (Renderer)"), so it takes the rest of the line; the
 * header has no leading digits and falls out on its own.
 */
export function parsePsOutput(text: string): Proc[] {
  const procs: Proc[] = [];
  for (const line of text.split("\n")) {
    const m = /^\s*(\d+)\s+(\S+)\s+(\S+)\s+(.+?)\s*$/.exec(line);
    if (!m) continue;
    procs.push({
      pid: +m[1],
      cpuSeconds: parseCpuTime(m[2]),
      elapsedSeconds: parseElapsed(m[3]),
      comm: m[4],
    });
  }
  return procs;
}

/**
 * The runaway rule, and the reason it is two conditions rather than one.
 *
 * Ranking by cumulative CPU is the obvious implementation and it is wrong: on the day
 * this was written WindowServer had MORE CPU time than the stuck extension (168m vs
 * 148m) and was perfectly healthy — it had simply been alive four times longer. The
 * signal is the RATIO, how much of a core a process has held for its whole life.
 *
 * The absolute floor is the second condition, and it exists for the opposite error: a
 * compiler at 100% for five minutes has a ratio of 1.0 and is not a runaway, it is a
 * build. Something has to have been wrong for a while before it is worth an interrupt.
 *
 * The ratio is deliberately NOT capped at 1. A process pinning four cores for an hour
 * reads as 4.0, which is exactly how alarming it should look.
 *
 * ## One finding per command, the worst instance
 *
 * The key is the command, so a browser with four helper processes over the line is ONE
 * condition, not four. Collapsing them here rather than downstream was found by the first
 * end-to-end run against the new store, which failed on the unique index: `tasks` had
 * been absorbing the duplicates silently, returning the row it already owned and letting
 * this tool count a second "new task" that was never written. The worst instance wins
 * because it is the one worth looking at, and the note names how many others crossed the
 * line so the count is not lost with them.
 */
export function classifyProcesses(procs: Proc[], policy: WatchPolicy): ProcFinding[] {
  const floor = policy.process?.min_cpu_seconds ?? Infinity;
  const minRatio = policy.process?.min_cpu_ratio ?? Infinity;
  const allowed = new Set((policy.allow_process ?? []).map((a) => a.comm));

  const over: Array<Proc & { ratio: number }> = [];
  for (const p of procs) {
    if (allowed.has(p.comm)) continue;
    if (p.cpuSeconds < floor) continue;
    const ratio = p.elapsedSeconds > 0 ? p.cpuSeconds / p.elapsedSeconds : 0;
    if (ratio < minRatio) continue;
    over.push({ ...p, ratio });
  }
  over.sort((a, b) => b.ratio - a.ratio);

  const found: ProcFinding[] = [];
  const seen = new Set<string>();
  for (const p of over) {
    // Keyed on the command, never the pid: a pid is a different number every boot and
    // would make every restart look like a new problem.
    const key = `cpu:${p.comm}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const others = over.filter((q) => q.comm === p.comm).length - 1;
    found.push({
      key,
      pid: p.pid,
      comm: p.comm,
      ratio: p.ratio,
      cpuSeconds: p.cpuSeconds,
      title: `${p.comm} has used ${hours(p.cpuSeconds)} of CPU (${p.ratio.toFixed(2)} cores sustained)`,
      note:
        `pid ${p.pid} · ${hours(p.cpuSeconds)} CPU over ${hours(p.elapsedSeconds)} wall ` +
        `= ${p.ratio.toFixed(2)} cores held continuously.\n` +
        (others > 0
          ? `${others} other process(es) named ${p.comm} are also over the line; this is the worst.\n`
          : "") +
        `Inspect: ps -p ${p.pid} -o pid,lstart,time,pcpu,command\n` +
        `If it is stuck rather than working: kill ${p.pid}`,
    });
  }
  return found;
}

const hours = (s: number) => (s >= 3600 ? `${(s / 3600).toFixed(1)}h` : `${Math.round(s / 60)}m`);

export type StorageReport = {
  disk?: { free?: number; target?: string };
  state?: string;
  classes?: Array<{ name: string; bytes: number; flagged?: boolean }>;
};

/**
 * The volume state is the finding; a large class is not.
 *
 * Decided in Axon#177 against the tempting alternative. `class_flag_gb` legitimately
 * fires today — the cargo target dir is 28 GB against a 20 GB flag — on a machine with
 * 130 GB free and nothing wrong with it. Alerting on that would mean this watcher's
 * FIRST run produced a task nobody needed, which is precisely how a watcher gets muted.
 * The class breakdown stays what it already was: what `sysmon storage` tells you once
 * you are looking.
 */
export function storageFinding(report: StorageReport): Finding | null {
  const state = report.state ?? "ok";
  if (state === "ok") return null;
  const free = report.disk?.free ?? 0;
  const target = report.disk?.target ?? "the data volume";
  return {
    key: "storage:free-below-threshold",
    title: `Disk ${state}: ${fmt(free)} free on ${target}`,
    note:
      `Free space crossed the ${state} threshold in the overlay's storage-policy.toml.\n` +
      `What is filling it, and what is safe to reclaim: tools/sysmon storage\n` +
      `Reclaim the applicable classes: tools/sysmon storage --apply`,
  };
}

export type NetExposure = { process: string; port: string; protos: string; pid: number };
export type NetReport = { listeners?: number; wildcard?: number; policy?: string; unexpected?: NetExposure[] };

/**
 * One finding for the whole condition, never one per port.
 *
 * The same rule `cpu:<comm>` follows, for the same reason and one sharper case. The partial
 * unique index allows one open row per key, and a mesh VPN's wildcard ports are assigned per
 * start — `*:41641` today, a different number after the next restart — so a per-port key would
 * mint a fresh generation every hour and the ladder would fill with the same fact.
 *
 * host-net owns the scope rule, the policy file and the parsing. This reads its verdict and
 * adds nothing: the note lists what came back, in the order host-net sorted it.
 */
export function netFinding(report: NetReport | null): Finding | null {
  const unexpected = report?.unexpected ?? [];
  if (!report || unexpected.length === 0) return null;
  const names = [...new Set(unexpected.map((e) => e.process))];
  return {
    key: "net:unexpected-exposure",
    title:
      `${unexpected.length} wildcard listener(s) not in the host-net policy` +
      ` (${names.slice(0, 3).join(", ")}${names.length > 3 ? ", …" : ""})`,
    note:
      unexpected.map((e) => `${e.process} on *:${e.port} (${e.protos}) · pid ${e.pid}`).join("\n") +
      `\n\nEvery interface this host has, now or later, reaches these.\n` +
      `Inspect: host-net listen\n` +
      `Accept one by adding an [[expect_wildcard]] entry to ${report.policy ?? "the host-net policy"}`,
  };
}

/**
 * One row per RUN of a condition, not one per check and not one forever.
 *
 * The unique index on an open `key` collapses repeats, which is most of the job. What it
 * cannot express on its own is the case that makes the difference between a watcher that
 * works next year and one that silently stops: the condition clears, and six weeks later
 * it comes back. Re-using the same row would upsert onto the closed one and say nothing,
 * forever. So a row carries a generation, and a new one is minted only once every prior
 * row for this condition is closed.
 *
 * The generation used to be packed into the id as `{key}~{n}` because `tasks` gave this
 * watcher one string field to key on, and parsing that string back out is where the two
 * bugs of Axon#177 lived — a `#` separator truncated the PATCH path, and without any
 * separator `cpu:Storage` claimed `cpu:StorageManagementService`'s history. Owning the
 * table makes both unrepresentable: `key` and `generation` are columns, and the
 * comparison below is exact by construction rather than by choosing a lucky character.
 */
export function decideEmission(key: string, existing: FindingRow[]): Emission {
  const mine = existing.filter((row) => row.key === key);
  const open = mine.find((row) => row.status === "open");
  if (open) return { action: "refresh", id: open.id };
  const generation = mine.reduce((max, row) => Math.max(max, row.generation), 0) + 1;
  return { action: "create", id: `${key}~${generation}`, generation };
}

/**
 * Which open findings this run did NOT see, and must therefore close.
 *
 * The half that could not exist before. Under `tasks` a finding closed only when the
 * operator pressed Done; that button is gone with the capability, so without this a row
 * written once would stay open forever and the ladder would keep ranking a process that
 * exited months ago. A watcher whose findings only accumulate stops meaning anything.
 */
export function decideResolutions(present: Finding[], existing: FindingRow[]): string[] {
  const seen = new Set(present.map((finding) => finding.key));
  return existing.filter((row) => row.status === "open" && !seen.has(row.key)).map((row) => row.id);
}

// ── I/O ────────────────────────────────────────────────────────────────────────

function fail(message: string, code = 1): never {
  console.error(`host-watch: ${message}`);
  process.exit(code);
}

async function runPs(): Promise<Proc[]> {
  const proc = Bun.spawn(["ps", "-Aceo", "pid,time,etime,comm"], { stdout: "pipe", stderr: "pipe" });
  const text = await new Response(proc.stdout).text();
  await proc.exited;
  if (proc.exitCode !== 0) fail("ps failed");
  return parsePsOutput(text);
}

/**
 * storage is invoked rather than reimplemented: it owns the policy file, the du/df
 * arithmetic and the exit code, and re-deriving any of that here would be the second
 * source of truth its own header argues against. A non-zero exit is NOT a failure — it
 * is how storage reports free space below critical, which is the loudest thing it can say.
 *
 * The launcher, not the binary: it sources tools/lib/paths.sh, which is what resolves the
 * overlay, and it builds the release binary if this is the first run after a checkout.
 * The `--json` shape it returns is unchanged from the TypeScript tool this replaced.
 */
async function runStorage(): Promise<StorageReport | null> {
  const proc = Bun.spawn([join(axonRoot(), "tools", "storage", "storage"), "report", "--json"], {
    stdout: "pipe",
    stderr: "pipe",
  });
  const text = await new Response(proc.stdout).text();
  await proc.exited;
  try {
    return JSON.parse(text) as StorageReport;
  } catch {
    const err = (await new Response(proc.stderr).text()).trim().slice(0, 200);
    console.error(`host-watch: storage --json unreadable (exit ${proc.exitCode}) ${err}`);
    return null;
  }
}

/**
 * host-net is invoked rather than reimplemented, the same call storage.ts gets above: it owns
 * the netstat parsing, the scope rule and the overlay policy, and re-deriving any of that here
 * would be the second source of truth its own README argues against.
 *
 * A non-zero exit is NOT a failure — exit 1 is how host-net reports an unexpected listener,
 * which is the loudest thing it can say. Exit 2 means it could not check at all (no policy,
 * a missing command), and that is data too: one stderr line, no finding, no throw.
 *
 * The built BINARY, never `capabilities/host-net/host-net`. That launcher builds on first use,
 * and an hourly scheduled job with the ability to start a cargo build is a surprise nobody
 * asked for. A host that has never built it files nothing and says so once.
 */
async function runHostNet(): Promise<NetReport | null> {
  const bin = join(axonRoot(), "target", "release", "host-net-cli");
  if (!existsSync(bin)) {
    console.error(`host-watch: ${bin} not built — network exposure not checked`);
    return null;
  }
  const proc = Bun.spawn([bin, "check", "--json"], { stdout: "pipe", stderr: "pipe" });
  // Both pipes are drained together, before awaiting exit. Reading one and leaving the other
  // buffered deadlocks the moment the child writes more than a pipe buffer to the one nobody
  // is reading, and the child that talks most on stderr is exactly the failing one.
  const [text, err] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
  ]);
  await proc.exited;
  try {
    return JSON.parse(text) as NetReport;
  } catch {
    const tail = err.trim().slice(0, 200);
    console.error(`host-watch: host-net check --json unreadable (exit ${proc.exitCode}) ${tail}`);
    return null;
  }
}

/**
 * Where the shared SQLite file is, resolved exactly the way `axon_config::database_path()`
 * resolves it — `SJEL_DB_PATH` first, then the overlay. A tool that opened a different
 * file than the capabilities do would write findings nothing reads.
 */
function databasePath(): string {
  const fromEnv = (process.env.SJEL_DB_PATH ?? "").trim();
  if (fromEnv) return fromEnv.startsWith("~/") ? join(process.env.HOME ?? "", fromEnv.slice(2)) : fromEnv;
  const overlay = overlayRoot();
  if (!overlay) fail("no overlay to resolve the database path from; set SJEL_DB_PATH");
  return join(overlay, "data", "axon", "axon.db");
}

/** The canonical stamp, spelled the way `axon_store::NOW` spells it. */
const NOW = "strftime('%Y-%m-%d %H:%M:%f+00:00','now')";

/**
 * Open the shared store and make sure this capability's one table is there.
 *
 * `CREATE TABLE IF NOT EXISTS` on every run, which is what every Rust capability's
 * migration does too (libs/axon-store) — a job that runs hourly and might be the first
 * thing to touch a fresh database cannot assume someone else went first.
 *
 * The partial unique index is the contract: at most one OPEN finding per condition. A
 * plain unique index on `key` would refuse the second generation, which is precisely the
 * history this watcher needs to keep.
 */
function openStore(): Database {
  const path = databasePath();
  mkdirSync(dirname(path), { recursive: true });
  const db = new Database(path, { create: true });
  db.exec("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;");
  db.exec(`
    CREATE TABLE IF NOT EXISTS ${PREFIX}_findings (
        id TEXT PRIMARY KEY,
        key TEXT NOT NULL,
        generation INTEGER NOT NULL,
        status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','resolved')),
        title TEXT NOT NULL,
        note TEXT NOT NULL,
        first_seen TEXT NOT NULL,
        last_seen TEXT NOT NULL,
        resolved_at TEXT
    );
    CREATE UNIQUE INDEX IF NOT EXISTS ${PREFIX}_findings_one_open_per_key
        ON ${PREFIX}_findings (key) WHERE status = 'open';
  `);
  return db;
}

/** What this run changed, for the one line it prints. */
type Emitted = { created: number; refreshed: number; resolved: number };

/**
 * Write this run's verdict: open what is new, refresh what persists, close what cleared.
 *
 * One transaction, because a half-applied run is a lie about the machine — a finding
 * resolved with its replacement not yet written reads as "nothing wrong" for exactly as
 * long as it takes the next hour to arrive.
 */
function emit(findings: Finding[]): Emitted {
  const db = openStore();
  try {
    const existing = db
      .query(`SELECT id, key, generation, status FROM ${PREFIX}_findings`)
      .all() as FindingRow[];

    const refresh = db.prepare(
      `UPDATE ${PREFIX}_findings SET title = ?2, note = ?3, last_seen = ${NOW} WHERE id = ?1`,
    );
    const create = db.prepare(
      `INSERT INTO ${PREFIX}_findings (id, key, generation, status, title, note, first_seen, last_seen)
       VALUES (?1, ?2, ?3, 'open', ?4, ?5, ${NOW}, ${NOW})`,
    );
    const resolve = db.prepare(
      `UPDATE ${PREFIX}_findings SET status = 'resolved', resolved_at = ${NOW} WHERE id = ?1`,
    );

    const counts: Emitted = { created: 0, refreshed: 0, resolved: 0 };
    db.transaction(() => {
      for (const finding of findings) {
        const decision = decideEmission(finding.key, existing);
        if (decision.action === "refresh") {
          refresh.run(decision.id, finding.title, finding.note);
          counts.refreshed += 1;
          console.log(`host-watch: still open — ${finding.title}`);
          continue;
        }
        create.run(decision.id, finding.key, decision.generation, finding.title, finding.note);
        counts.created += 1;
        console.log(`host-watch: NEW — ${finding.title}`);
      }
      for (const id of decideResolutions(findings, existing)) {
        resolve.run(id);
        counts.resolved += 1;
        console.log(`host-watch: cleared — ${id}`);
      }
    })();
    return counts;
  } finally {
    db.close();
  }
}

export async function main() {
  const argv = Bun.argv.slice(2);
  if (argv.includes("-h") || argv.includes("--help")) return void process.stdout.write(HELP);
  const dryRun = argv.includes("--dry-run");
  const asJson = argv.includes("--json");

  const policyPath = `${overlayRoot()}/config/host-watch-policy.toml`;
  if (!existsSync(policyPath)) {
    console.error(
      `host-watch: no policy at ${policyPath}\nSee schemas/host-watch-policy.toml.example for the expected shape.`,
    );
    process.exit(2);
  }
  const policy = Bun.TOML.parse(await Bun.file(policyPath).text()) as WatchPolicy;

  const [procs, storage, net] = await Promise.all([runPs(), runStorage(), runHostNet()]);
  const findings: Finding[] = [
    ...classifyProcesses(procs, policy),
    ...(storage ? [storageFinding(storage)].filter((f): f is Finding => f !== null) : []),
    ...[netFinding(net)].filter((f): f is Finding => f !== null),
  ];

  if (asJson) {
    console.log(JSON.stringify({ checked: procs.length, findings }, null, 2));
    return;
  }

  if (dryRun) {
    console.log(`host-watch: ${findings.length} finding(s), writing nothing (--dry-run)`);
    for (const f of findings) console.log(`  ${f.title}`);
    return;
  }

  // A healthy run still writes, and that is the point of the closing half: it is the
  // run with NO findings that clears the rows the last one left open. Returning early
  // here — which this did while `tasks` owned the lifecycle — would mean a condition
  // that cleared stayed on the ladder until the operator noticed it themselves.
  const { created, refreshed, resolved } = emit(findings);
  if (findings.length === 0 && resolved === 0) {
    console.log(
      `host-watch: ${procs.length} processes, disk ${storage?.state ?? "unknown"} — nothing to report`,
    );
    return;
  }
  console.log(
    `host-watch: ${findings.length} finding(s) — ${created} new, ${refreshed} still open, ${resolved} cleared`,
  );
}

// Guarded so the test file can import the pure functions without running a scan.
if (import.meta.main) await main();
