#!/usr/bin/env bun
// tools/harnesses — Packs across every agent harness at once.
//
// Each packs-<harness> adapter owns one destination and answers about it alone.
// Nothing answered the question an operator actually has: what is deployed
// WHERE, what has drifted, and what is sitting in a harness that Axon does not
// know about. This tool asks every harness in tools/lib/harness-registry.ts the
// same question and prints one answer.
//
// Direction: Axon is the source and `sync` is one-way, Axon -> harness. The one
// move in the other direction is `promote`, which is manual on purpose: a skill
// written inside a harness is brought into a Pack only when a human decides it
// is worth sharing system-wide, and promote then claims the live copy rather
// than replacing it.
//
//   tools/harnesses list                     which harnesses exist, and which are installed here
//   tools/harnesses status [<pack>]          one matrix: every Pack skill x every harness
//   tools/harnesses drift [<pack>] [--diff]  per-file detail for anything that drifted
//   tools/harnesses sync <pack>|--all        one-way Axon -> harness, installed harnesses only
//   tools/harnesses promote <skill> --pack <pack>   bring a harness-level skill into Axon
//   tools/harnesses accept <pack> <skill>           keep an edit made to a deployed copy
//
// Flags: --harness <id> restricts every verb to one harness. --all-harnesses
// includes harnesses that are not installed, which is otherwise refused.

import {
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  readlinkSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, relative } from "node:path";
import { createHash } from "node:crypto";
import {
  HARNESSES,
  UNSUPPORTED,
  harnessById,
  isInstalled,
  type Harness,
} from "./lib/harness-registry.ts";
import {
  adoptPack,
  activateProfile,
  availablePacks,
  desiredFiles,
  getStatuses,
  packUnits,
  readProfiles,
  readState,
  reconcileUnit,
  syncPack,
  type DeployConfig,
  type Profile,
  type SkillStatus,
  type StatusRow,
} from "./lib/pack-deploy.ts";
import { activateProfileOnPi } from "./packs-pi.ts";

const SJEL_ROOT = new URL("..", import.meta.url).pathname.replace(/\/$/, "");
const argv = process.argv.slice(2);
const flag = (name: string): string | undefined => {
  const i = argv.indexOf(`--${name}`);
  return i === -1 ? undefined : argv[i + 1];
};
const has = (name: string) => argv.includes(`--${name}`);
const positional = argv.filter((a, i) => !a.startsWith("--") && !(i > 0 && argv[i - 1] === "--harness") && !(i > 0 && argv[i - 1] === "--pack"));

function selectedHarnesses(): Harness[] {
  const one = flag("harness");
  const all = one ? [harnessById(one)] : HARNESSES;
  if (has("all-harnesses") || one) return all;
  return all.filter(isInstalled);
}

// ---------------------------------------------------------------- list

function list(): void {
  console.log("harness      state       delivery       marker");
  for (const h of HARNESSES) {
    const state = isInstalled(h) ? "installed" : "absent   ";
    console.log(`${h.id.padEnd(12)} ${state}   ${h.model.padEnd(14)} ${h.marker}`);
  }
  for (const u of UNSUPPORTED) {
    console.log(`${u.id.padEnd(12)} unsupported —              ${u.why}`);
  }
}

// ---------------------------------------------------------------- status

/** A registry harness has no copy, so it has no digest. Its states are its own. */
export type PiDiscovered = {
  name: string;
  /** Discovery root the entry was found in, as a human label — ~/.agents/skills, *.pi/skills, … */
  label: string;
  kind: "copy" | "external" | "symlink" | "md";
  detail?: string;
};

export type PiExtension = { path: string; source: "ledger" | "discovered" | "both" };

/**
 * The roots pi loads without being told (pi docs/skills.md, docs/extensions.md):
 * two global ones plus `.pi/skills` / `.agents/skills` in cwd and every ancestor up
 * to the git repo root (filesystem root when not in a repo). The registry model only
 * knew settings.json; the whole point of discovery is that it is not in the ledger.
 */
function discoveryRoots(): { root: string; label: string }[] {
  const home = process.env.HOME ?? "";
  const roots = [
    { root: join(home, ".pi", "agent", "skills"), label: "~/.pi/agent/skills" },
    { root: join(home, ".agents", "skills"), label: "~/.agents/skills" },
  ];
  let dir = process.cwd();
  while (true) {
    for (const name of [".pi", ".agents"]) {
      const root = join(dir, name, "skills");
      if (existsSync(root)) roots.push({ root, label: `project ${root}` });
    }
    if (existsSync(join(dir, ".git"))) break; // pi stops its walk at the repo root
    const parent = dirname(dir);
    if (parent === dir) break; // filesystem root
    dir = parent;
  }
  return roots;
}

function discoveredAt(root: string, label: string): PiDiscovered[] {
  const out: PiDiscovered[] = [];
  if (!existsSync(root)) return out;
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const full = join(root, entry.name);
    if (entry.isSymbolicLink()) {
      out.push({ name: entry.name, label, kind: "symlink", detail: `→ ${readlinkSync(full)}` });
    } else if (entry.isDirectory()) {
      if (!existsSync(join(full, "SKILL.md"))) continue;
      const external = existsSync(join(full, ".git"));
      out.push({
        name: entry.name,
        label,
        kind: external ? "external" : "copy",
        detail: external ? "carries .git; another installer owns it" : undefined,
      });
    } else if (entry.isFile() && entry.name.endsWith(".md") && entry.name !== "SKILL.md") {
      // Root .md files are skills when they carry valid skill frontmatter (pi docs/skills.md).
      const head = readFileSync(full, "utf8").slice(0, 1000);
      if (/^---\s*\n[\s\S]*?\bname:\s*\S+[\s\S]*?\bdescription:\s*\S+/.test(head)) {
        out.push({ name: entry.name.replace(/\.md$/, ""), label, kind: "md" });
      }
    }
  }
  return out;
}

/** Everything pi loads that the ledger does not name: discovered skills and extensions. */
export function piDiscovery(): { entries: PiDiscovered[]; extensions: PiExtension[] } {
  const home = process.env.HOME ?? "";
  const entries: PiDiscovered[] = [];
  for (const { root, label } of discoveryRoots()) entries.push(...discoveredAt(root, label));

  const settingsPath = join(home, ".pi", "agent", "settings.json");
  const parsed = existsSync(settingsPath)
    ? (JSON.parse(readFileSync(settingsPath, "utf8")) as Record<string, unknown>)
    : {};
  const ledger = Array.isArray(parsed.extensions) ? (parsed.extensions as string[]) : [];
  const found: string[] = [];
  const extsRoot = join(home, ".pi", "agent", "extensions");
  if (existsSync(extsRoot)) {
    for (const entry of readdirSync(extsRoot, { withFileTypes: true })) {
      const full = join(extsRoot, entry.name);
      if ((entry.isFile() || entry.isSymbolicLink()) && entry.name.endsWith(".ts")) found.push(full);
      else if (entry.isDirectory() && existsSync(join(full, "index.ts"))) found.push(join(full, "index.ts"));
    }
  }
  const byPath = new Map<string, PiExtension>();
  for (const p of ledger) byPath.set(p, { path: p, source: "ledger" });
  for (const p of found) {
    const existing = byPath.get(p);
    if (existing) existing.source = "both";
    else byPath.set(p, { path: p, source: "discovered" });
  }
  return { entries, extensions: [...byPath.values()] };
}

/** A registry harness has no copy, so it has no digest. Its states are its own. */
function registryStatuses(harness: Harness, selected?: string): StatusRow[] {
  const config = harness.config();
  const settingsPath = (config as DeployConfig & { settingsPath?: string }).settingsPath;
  const piSettings = settingsPath ?? join(process.env.HOME ?? "", ".pi", "agent", "settings.json");
  let registered: string[] = [];
  if (existsSync(piSettings)) {
    const parsed = JSON.parse(readFileSync(piSettings, "utf8")) as Record<string, unknown>;
    registered = Array.isArray(parsed.skills) ? (parsed.skills as string[]) : [];
  }
  const discovered = new Map<string, PiDiscovered[]>();
  for (const { root, label } of discoveryRoots()) {
    for (const d of discoveredAt(root, label)) {
      if (!discovered.has(d.name)) discovered.set(d.name, []);
      discovered.get(d.name)!.push(d);
    }
  }
  const rows: StatusRow[] = [];
  for (const pack of availablePacks(config, true)) {
    if (selected && pack !== selected) continue;
    for (const unit of packUnits(config, pack)) {
      if (!unit.isSkill) continue;
      const isRegistered = registered.some((p) => p.replace(/\/$/, "") === unit.sourceRoot.replace(/\/$/, ""));
      const hits = discovered.get(unit.key) ?? [];
      let status: SkillStatus;
      let detail: string | undefined;
      if (isRegistered && hits.length) {
        status = "current";
        detail = `registered in settings; ALSO discovered from ${hits.map((h) => `${h.label}/${h.name}`).join(", ")} — pi keeps the first found, so the copy can shadow the registration`;
      } else if (isRegistered) {
        status = "current";
        detail = "registered in settings";
      } else if (hits.length) {
        status = "discovered";
        detail = hits.map((h) => `${h.label}/${h.name} (${h.kind}${h.detail ? `, ${h.detail}` : ""})`).join("; ");
      } else {
        status = "not-deployed";
      }
      rows.push({ pack, skill: unit.key, status, detail });
    }
  }
  // A registered path that no longer exists is this model's only other defect.
  for (const path of registered) {
    if (!existsSync(path)) {
      rows.push({ pack: "(registered)", skill: basename(path), status: "missing", detail: `${path} does not exist` });
    }
  }
  return rows;
}

/** A registry harness has no copy, so it has no digest. Its states are its own. */
export function statusesFor(harness: Harness, selected?: string): StatusRow[] {
  return harness.model === "registry" ? registryStatuses(harness, selected) : getStatuses(harness.config(), selected);
}

const MARK: Record<string, string> = {
  current: "·",
  "not-deployed": " ",
  outdated: "o",
  drifted: "D",
  missing: "M",
  collision: "C",
  invalid: "!",
  discovered: "~",
  "migration-required": "m",
};

function status(): void {
  const selected = positional[1];
  const harnesses = selectedHarnesses();
  if (has("json")) {
    // The machine-readable shape any surface reads — a dashboard panel, a doctor
    // section, a hook. Emitted by the same code path as the table so the two can
    // never disagree about what is deployed.
    console.log(
      JSON.stringify(
        {
          measuredAt: new Date().toISOString(),
          harnesses: HARNESSES.map((h) => ({
            id: h.id,
            label: h.label,
            installed: isInstalled(h),
            model: h.model,
            marker: h.marker,
            destination: h.model === "materialized" ? h.config().destination : null,
            cli: h.cli,
            // Reported for EVERY harness, installed or not. Reading is free and an
            // absent harness holding deployed copies is the exact condition this
            // tool exists to surface — three Packs sat in ~/.agents/skills for an
            // uninstalled Codex, and a report that skipped absent harnesses would
            // have hidden them the same way the adapters did.
            units: statusesFor(h, selected).map((row) => ({ pack: row.pack, skill: row.skill, status: row.status, detail: row.detail })),
            discovered: h.model === "registry" ? piDiscovery() : undefined,
            unowned: foreignAt(h),
          })),
          unsupported: UNSUPPORTED,
        },
        null,
        2,
      ),
    );
    return;
  }
  if (!harnesses.length) {
    console.log("no harness is installed; nothing to compare");
    return;
  }
  const perHarness = new Map<string, Map<string, StatusRow>>();
  const keys: string[] = [];
  for (const h of harnesses) {
    const map = new Map<string, StatusRow>();
    for (const row of statusesFor(h, selected)) {
      const key = `${row.pack}/${row.skill}`;
      map.set(key, row);
      if (!keys.includes(key)) keys.push(key);
    }
    perHarness.set(h.id, map);
  }

  const width = Math.max(28, ...keys.map((k) => k.length + 2));
  console.log("".padEnd(width) + harnesses.map((h) => h.id.padEnd(10)).join(""));
  let previousPack = "";
  for (const key of keys) {
    const pack = key.split("/")[0];
    if (pack !== previousPack) {
      console.log(pack);
      previousPack = pack;
    }
    const cells = harnesses.map((h) => {
      const row = perHarness.get(h.id)?.get(key);
      return (row ? (MARK[row.status] ?? row.status) : " ").padEnd(10);
    });
    console.log(`  ${key.split("/").slice(1).join("/").padEnd(width - 2)}${cells.join("")}`);
  }
  console.log("\n· current   o outdated   D drifted   M missing   C collision   ! invalid   ~ discovered (loaded outside the ledger)   (blank) not deployed");

  // An absent harness holding deployed copies is invisible in the matrix above,
  // because the matrix only shows harnesses that are installed. It is also the
  // condition this tool was written for, so it gets its own line.
  for (const harness of HARNESSES) {
    if (isInstalled(harness) || harness.model !== "materialized") continue;
    const deployed = statusesFor(harness).filter((row) => row.status !== "not-deployed");
    if (!deployed.length) continue;
    const packs = [...new Set(deployed.map((row) => row.pack))].sort();
    console.log(
      `\n${harness.label} is NOT installed (no ${harness.marker}), and ${deployed.length} units from ${packs.length} Pack(s) are deployed at ${harness.config().destination}:`,
    );
    console.log(`  ${packs.join(", ")}`);
    // ~/.agents/skills doubles as pi's global discovery root. Saying "nothing reads
    // this" there with pi installed was 2026-09-09's motivating lie: pi was loading
    // all four units while the tool told the operator the opposite. Name the reader.
    const pi = harnessById("pi");
    const piReadsDestination =
      isInstalled(pi) && harness.config().destination === join(process.env.HOME ?? "", ".agents", "skills");
    if (piReadsDestination) {
      console.log(`  pi IS installed and discovers ${harness.config().destination} — these units are loaded by pi right now.`);
      console.log(`  If pi should keep them, deploy them there FIRST: tools/packs-pi deploy ${packs.join(" ")}.`);
      console.log(`  Removing without that step strips them from pi without replacement: ${harness.cli} remove ${packs.join(" ")}`);
    } else {
      console.log(`  Nothing on this machine reads them. Remove: ${harness.cli} remove ${packs.join(" ")}`);
    }
  }

  // Pi's discovery roots are live surfaces with no ledger entry, and a registry
  // harness has no destination section above — so discovered skills (interceptor,
  // a foreign skills dir) and the discovered extensions get their own lines here.
  if (harnesses.some((h) => h.id === "pi")) {
    const { entries, extensions } = piDiscovery();
    const seen = new Set<string>();
    if (entries.length || extensions.some((e) => e.source !== "ledger")) {
      console.log("\npi loads these outside settings.json (discovered, not ledger-owned):");
      for (const d of entries) {
        const key = `${d.label}/${d.name}`;
        if (seen.has(key)) continue;
        seen.add(key);
        console.log(`  ~ ${key}  (${d.kind}${d.detail ? `, ${d.detail}` : ""})`);
      }
      for (const e of extensions) {
        if (e.source === "ledger") continue;
        console.log(`  ~ ${e.path}  (extension; ${e.source === "both" ? "ledger + discovered" : "discovered, not in the ledger"})`);
      }
    }
  }

  for (const h of harnesses) {
    const strays = foreignAt(h);
    if (!strays.length) continue;
    console.log(`\n${h.label}: at the destination, not owned by any Pack`);
    for (const s of strays) console.log(`  ${s.kind.padEnd(8)} ${s.name}${s.detail ? `  ${s.detail}` : ""}`);
    if (strays.some((s) => s.kind === "copy")) {
      console.log(`  a copy is a promote candidate: tools/harnesses promote <name> --pack <pack> --from ${h.id}`);
    }
  }
}

type Stray = { name: string; kind: "copy" | "symlink" | "external"; detail?: string };

/**
 * A marker that says some other installer owns this directory. Promoting one of
 * these would fork it: the clone re-syncs from its own remote and the tool
 * install rewrites the directory on upgrade. Both were mistaken for unmanaged
 * copies on 2026-09-07 before anyone looked inside them.
 */
const EXTERNAL_MARKERS = [".git", ".graphify_version"];

/** Skill directories at a harness destination that no Pack ledger claims. */
function foreignAt(harness: Harness): Stray[] {
  if (harness.model !== "materialized") return [];
  const config = harness.config();
  if (!existsSync(config.destination)) return [];
  const owned = new Set<string>();
  const state = readState(config);
  for (const record of Object.values(state.packs)) for (const key of Object.keys(record.skills)) owned.add(key);
  const strays: Stray[] = [];
  for (const entry of readdirSync(config.destination)) {
    if (entry.startsWith(".") || owned.has(entry)) continue;
    const full = join(config.destination, entry);
    const link = lstatSync(full).isSymbolicLink();
    if (!link && !statSync(full).isDirectory()) continue;
    const marker = link ? null : EXTERNAL_MARKERS.find((m) => existsSync(join(full, m)));
    strays.push({
      name: entry,
      kind: link ? "symlink" : marker ? "external" : "copy",
      detail: link ? `→ ${readlinkSync(full)}` : marker ? `carries ${marker}; another installer owns it` : undefined,
    });
  }
  return strays.sort((a, b) => a.name.localeCompare(b.name));
}

// ---------------------------------------------------------------- drift

function fileDigest(path: string): string {
  return createHash("sha256").update(readFileSync(path)).digest("hex").slice(0, 12);
}

function walk(root: string, prefix = ""): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    if (entry.name === "__pycache__" || entry.name.startsWith(".DS_Store")) continue;
    const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
    const full = join(root, entry.name);
    if (entry.isDirectory()) out.push(...walk(full, rel));
    else out.push(rel);
  }
  return out;
}

function drift(): void {
  const selected = positional[1];
  let found = false;
  for (const h of selectedHarnesses()) {
    if (h.model !== "materialized") continue;
    const config = h.config();
    for (const row of statusesFor(h, selected)) {
      if (row.status !== "drifted" && row.status !== "outdated" && row.status !== "missing") continue;
      found = true;
      console.log(`\n${h.label} · ${row.pack}/${row.skill} — ${row.status}`);
      const unit = packUnits(config, row.pack).find((u) => u.key === row.skill);
      if (!unit) continue;
      if (!existsSync(unit.destination)) {
        console.log(`  the destination is gone: ${unit.destination}`);
        continue;
      }
      const wanted = desiredFiles(config, row.pack, unit);
      const actual = new Set(walk(unit.destination));
      for (const [rel, file] of wanted) {
        const there = join(unit.destination, rel);
        if (!actual.has(rel)) { console.log(`  missing at destination  ${rel}`); continue; }
        actual.delete(rel);
        if (fileDigest(file.absolutePath) !== fileDigest(there)) {
          console.log(`  differs                 ${rel}`);
          if (has("diff")) {
            const proc = Bun.spawnSync(["diff", "-u", file.absolutePath, there]);
            process.stdout.write(new TextDecoder().decode(proc.stdout));
          }
        }
      }
      for (const rel of actual) console.log(`  only at destination     ${rel}`);
      console.log(`  source:      ${relative(SJEL_ROOT, unit.sourceRoot)}`);
      console.log(`  destination: ${unit.destination}`);
      console.log(`  discard it:  ${h.cli} sync ${row.pack}   (overwrites the destination)`);
      console.log(`  keep it:     tools/harnesses accept ${row.pack} ${row.skill} --from ${h.id}`);
    }
  }
  if (!found) console.log("no drift: every deployed unit matches its Pack source");
}

// ---------------------------------------------------------------- use

/**
 * Activate a profile on every selected harness through the registry: pi rewrites
 * settings.json (skills AND extensions) via packs-pi's registry-model activation;
 * materialized harnesses go through the shared engine, honouring per-Pack skill
 * subsets. The direction rule is unchanged — this deploys Pack -> harness.
 */
function useProfile(): void {
  const profileName = positional[1];
  if (!profileName) throw new Error("usage: tools/harnesses use <profile> [--harness <id>]");
  const harnesses = selectedHarnesses();
  const profile = readProfiles({ axonRoot: SJEL_ROOT } as DeployConfig).find((p: Profile) => p.name === profileName);
  if (!profile) throw new Error(`no such profile: '${profileName}'`);
  for (const h of harnesses) {
    console.log(`── ${h.label}`);
    if (h.model === "registry") {
      activateProfileOnPi(profileName);
    } else {
      for (const line of activateProfile(h.config(), profile)) console.log(`  ${line}`);
    }
  }
}

// ---------------------------------------------------------------- sync

/**
 * Which packs `sync` should touch, given what the harness already knows about.
 *
 * `--all` is a flag here, not a positional value. The argv filter that builds `positional`
 * strips every `--`-prefixed argument, so the documented `sync --all` form could never reach
 * `positional[1]` — and the `target === "--all"` checks it used to feed were therefore dead
 * code, because the command always threw its own usage error first. Measured 2026-09-30,
 * which is why this is a named function with a test rather than two inline ternaries.
 */
export function syncTargets(pack: string | undefined, all: boolean, known: string[]): string[] {
  if (all) return [...known].sort();
  return pack ? [pack] : [];
}

function sync(): void {
  const pack = positional[1];
  const all = has("all");
  if (pack && all) throw new Error("tools/harnesses sync: give a pack or --all, not both");
  if (!pack && !all) throw new Error("usage: tools/harnesses sync <pack>|--all [--harness <id>]");
  for (const h of selectedHarnesses()) {
    console.log(`${h.label}:`);
    if (h.model === "registry") {
      // A registry harness deploys one pack per invocation, so this line is a hint to run
      // per pack rather than a loop this command could perform.
      console.log(`  registry harness — run: ${h.cli} deploy ${all ? "<pack>" : pack}`);
      continue;
    }
    const config = h.config();
    for (const target of syncTargets(pack, all, Object.keys(readState(config).packs))) {
      try {
        for (const line of syncPack(config, target)) console.log(`  ${line}`);
      } catch (error) {
        console.log(`  ✗ ${target}: ${(error as Error).message}`);
      }
    }
  }
}

// ---------------------------------------------------------------- promote

/**
 * A pack.toml `skills = [...]` line with one more skill in it.
 *
 * Its own function so it can be tested: `promote` around it copies a directory tree
 * and adopts a Pack, and the string edit is the part that a skill name can steer.
 * A skill name is a directory name off `config.destination`, so every character a
 * macOS filename allows can reach here — and three of them used to matter:
 *
 * - `.` or `|` made `new RegExp(`"${skill}"`)` match a name that is not there, so
 *   `a.b` reported `axb` as already present and refused a legal promote. CodeQL
 *   js/regex-injection, alert 71. A substring test asks the question that was meant.
 * - `$&` or `$'` in a `String.replace` REPLACEMENT expands to the match and the
 *   text after it, so the name went into the manifest rewritten. The splice below
 *   never builds a replacement pattern.
 * - a `skills` array written across lines matched no `]` at the end, so the old
 *   `replace` returned the body unchanged and `promote` still printed "✓ added to".
 *   It now refuses, which is what "single-line TOML only" was always worth.
 */
export function skillsLineWith(line: string, skill: string): string {
  if (line.includes(`"${skill}"`)) throw new Error(`${skill} is already in the skills line`);
  if (!/\]\s*$/.test(line)) {
    throw new Error("the skills line does not end in `]`; tools/lib/toml.sh cannot read a multi-line array");
  }
  // An empty array has nothing to separate the new name from. `skills = []` spliced
  // with a comma gives `skills = [, "x"]`, which no TOML parser reads — and an empty
  // array is exactly the state a Pack is in when `promote` puts the first skill in it.
  const head = line.replace(/\]\s*$/, "").replace(/\s+$/, "");
  const separator = head.endsWith("[") ? "" : ", ";
  return `${head}${separator}"${skill}"]`;
}

/** The one harness -> Axon move. Manual by design; see the header. */
function promote(): void {
  const skill = positional[1];
  const pack = flag("pack");
  const from = flag("from") ?? "claude";
  if (!skill || !pack) throw new Error("usage: tools/harnesses promote <skill> --pack <pack> [--from <harness>]");
  const harness = harnessById(from);
  if (harness.model !== "materialized") throw new Error(`${from} registers Pack paths in place; there is nothing to promote from it`);
  const config = harness.config();
  const source = join(config.destination, skill);
  if (!existsSync(source)) throw new Error(`${source} does not exist`);
  if (lstatSync(source).isSymbolicLink()) {
    throw new Error(`${source} is a symlink: another installer owns that skill and a copy here would silently pin it`);
  }
  const owner = Object.entries(readState(config).packs).find(([, record]) => record.skills[skill]);
  if (owner) throw new Error(`${skill} is already owned by Pack '${owner[0]}'; nothing to promote`);

  const packDir = join(SJEL_ROOT, "Packs", pack);
  const manifest = join(packDir, "pack.toml");
  // Read the manifest here rather than asking `existsSync` here and reading it after
  // the copy. Two answers to the same question, taken from two instants, and the
  // second one is what gets written back: CodeQL js/file-system-race, alert 72. The
  // read is the existence check, and every refusal below now happens before a single
  // file is copied.
  let body: string;
  try {
    body = readFileSync(manifest, "utf8");
  } catch {
    throw new Error(`no Pack at ${relative(SJEL_ROOT, packDir)}`);
  }
  // Single-line TOML only: tools/lib/toml.sh cannot read an array across lines.
  const line = body.split("\n").find((l) => /^\s*skills\s*=/.test(l));
  if (!line) throw new Error(`${relative(SJEL_ROOT, manifest)} has no skills = [...] line`);
  let updated: string;
  try {
    updated = skillsLineWith(line, skill);
  } catch (error) {
    throw new Error(`${relative(SJEL_ROOT, manifest)}: ${(error as Error).message}`);
  }

  const target = join(packDir, "skills", skill);
  if (existsSync(target)) throw new Error(`${relative(SJEL_ROOT, target)} already exists`);

  for (const rel of walk(source)) {
    const to = join(target, rel);
    mkdirSync(dirname(to), { recursive: true });
    copyFileSync(join(source, rel), to);
  }

  // A function replacement, not a string one: a `$&` in the skill name would expand
  // inside a replacement pattern and rewrite the line it was inserted into.
  writeFileSync(manifest, body.replace(line, () => updated));

  console.log(`✓ copied ${skill} → ${relative(SJEL_ROOT, target)}`);
  console.log(`✓ added to ${relative(SJEL_ROOT, manifest)}`);
  for (const message of adoptPack(config, pack)) console.log(`  ${message}`);
  console.log(`\nThe live copy is now claimed, not replaced. Next: review the files, then`);
  console.log(`deploy the Pack to the other harnesses that should carry it.`);
}

/**
 * The second harness -> Axon move: destination edits to a skill Axon ALREADY
 * owns. `promote` is for a skill that has no Pack; this is for one that has a
 * Pack and was edited in place. Both directions exist because a good edit made
 * inside a harness is a normal thing to happen and `sync` would silently
 * destroy it.
 */
function accept(): void {
  const pack = positional[1];
  const skill = positional[2];
  const from = flag("from") ?? "claude";
  if (!pack || !skill) throw new Error("usage: tools/harnesses accept <pack> <skill> [--from <harness>]");
  const harness = harnessById(from);
  if (harness.model !== "materialized") throw new Error(`${from} reads the Pack source in place; it has no copy to accept`);
  const config = harness.config();
  const unit = packUnits(config, pack).find((u) => u.key === skill);
  if (!unit) throw new Error(`${pack} does not carry ${skill}`);
  if (!readState(config).packs[pack]?.skills[skill]) throw new Error(`${pack}/${skill} is not deployed to ${from}; nothing to accept`);
  if (!existsSync(unit.destination)) throw new Error(`${unit.destination} does not exist`);

  // Refuse to bury uncommitted work in the Pack source. The destination copy is
  // about to overwrite it, and git is the only undo this move has.
  const dirty = Bun.spawnSync(["git", "-C", SJEL_ROOT, "status", "--porcelain", "--", relative(SJEL_ROOT, unit.sourceRoot)]);
  const pending = new TextDecoder().decode(dirty.stdout).trim();
  if (pending && !has("force")) {
    throw new Error(`${relative(SJEL_ROOT, unit.sourceRoot)} has uncommitted changes:\n${pending}\ncommit or stash them first, or pass --force to overwrite`);
  }

  const incoming = walk(unit.destination);
  const existing = walk(unit.sourceRoot);
  for (const rel of incoming) {
    const to = join(unit.sourceRoot, rel);
    mkdirSync(dirname(to), { recursive: true });
    copyFileSync(join(unit.destination, rel), to);
  }
  const removed = existing.filter((rel) => !incoming.includes(rel));
  for (const rel of removed) rmSync(join(unit.sourceRoot, rel));

  console.log(`✓ ${incoming.length} files copied into ${relative(SJEL_ROOT, unit.sourceRoot)}`);
  for (const rel of removed) console.log(`  removed (absent at the destination): ${rel}`);
  // The ledger still holds the pre-edit digest and would keep reporting drift
  // that no longer exists, so re-record it now that the two agree.
  console.log(`  ${reconcileUnit(config, pack, unit)}`);
  console.log(`\nReview before committing:  git -C ${SJEL_ROOT} diff -- ${relative(SJEL_ROOT, unit.sourceRoot)}`);
  console.log(`Then deploy the Pack to the other harnesses that carry it.`);
}

// ---------------------------------------------------------------- main

const HELP = `tools/harnesses — Packs across every agent harness at once.

  list                                  which harnesses exist, and which are installed here
  status [<pack>] [--json]              one matrix: every Pack skill x every harness
  drift [<pack>] [--diff]               per-file detail for anything that drifted
  sync <pack>|--all                     one-way Axon -> harness (installed harnesses only)
  use <profile> [--harness <id>]        activate a profile on every installed harness (or one)
  promote <skill> --pack <p> [--from h] bring a harness-level skill Axon does not own into a Pack
  accept <pack> <skill> [--from h]      keep a destination edit to a skill Axon already owns

  --harness <id>     restrict to one harness (implies it, installed or not)
  --all-harnesses    include harnesses that are not installed
`;

// Guarded, so tools/harnesses.test.ts can import `skillsLineWith` without running a
// verb — console output and process.exit — as a side effect of the import. The
// precedent is tools/doctor.ts, whose own test does the same thing.
if (import.meta.main) {
  try {
    switch (positional[0] ?? "list") {
      case "list": list(); break;
      case "status": status(); break;
      case "drift": drift(); break;
      case "sync": sync(); break;
      case "use": useProfile(); break;
      case "promote": promote(); break;
      case "accept": accept(); break;
      case "help": case "-h": case "--help": console.log(HELP); break;
      default: console.error(HELP); process.exit(1);
    }
  } catch (error) {
    console.error(`harnesses: ${(error as Error).message}`);
    process.exit(1);
  }
}
