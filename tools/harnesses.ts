#!/usr/bin/env bun
// tools/harnesses — Packs across every agent harness at once: the WRITE verbs.
//
// The read verbs (`list`, `status`, `drift`) moved to tools/sjel-cli/src/harnesses/ on
// 2026-10-02, and this file's copies of them were deleted rather than left as a second reader
// of the same ledger. What remains is what writes: `sync`, `use`, `promote` and `accept`, on
// the engine that owns the mutation lock and the atomic install (tools/lib/pack-deploy.ts).
// They move when their parity is proven; until then `tools/harnesses` routes them here through
// the same binary that answers the read verbs.
//
// Run it through tools/harnesses, not directly: that launcher sets SJEL_ROOT and sends each
// verb to its implementation. Invoked here with a read verb, this file prints its usage.
//
// Direction: Sjel is the source and `sync` is one-way, Sjel -> harness. The one move in the
// other direction is `promote`, which is manual on purpose: a skill written inside a harness is
// brought into a Pack only when a human decides it is worth sharing system-wide, and promote
// then claims the live copy rather than replacing it.
//
//   tools/harnesses sync <pack>|--all        one-way Sjel -> harness, installed harnesses only
//   tools/harnesses use <profile>            activate a profile on every installed harness
//   tools/harnesses promote <skill> --pack <pack>   bring a harness-level skill into Sjel
//   tools/harnesses accept <pack> <skill>           keep an edit made to a deployed copy
//
// Flags: --harness <id> restricts every verb to one harness. --all-harnesses includes
// harnesses that are not installed, which is otherwise refused.

import {
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, relative } from "node:path";
import { HARNESSES, harnessById, isInstalled, type Harness } from "./lib/harness-registry.ts";
import {
  adoptPack,
  activateProfile,
  packUnits,
  readProfiles,
  readState,
  reconcileUnit,
  syncPack,
  type DeployConfig,
  type Profile,
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

// ---------------------------------------------------------------- use

/**
 * Activate a profile on every selected harness through the registry: pi rewrites
 * settings.json (skills AND extensions) via packs-pi's registry-model activation;
 * materialized harnesses go through the shared engine, honouring per-Pack skill subsets.
 * The direction rule is unchanged — this deploys Pack -> harness.
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

/** The one harness -> Sjel move. Manual by design; see the header. */
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
 * The second harness -> Sjel move: destination edits to a skill Sjel ALREADY
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
  sync <pack>|--all                     one-way Sjel -> harness (installed harnesses only)
  use <profile> [--harness <id>]        activate a profile on every installed harness (or one)
  promote <skill> --pack <p> [--from h] bring a harness-level skill Sjel does not own into a Pack
  accept <pack> <skill> [--from h]      keep a destination edit to a skill Sjel already owns

  --harness <id>     restrict to one harness (implies it, installed or not)
  --all-harnesses    include harnesses that are not installed
`;

// Guarded, so tools/harnesses.test.ts can import `skillsLineWith` without running a verb —
// console output and process.exit — as a side effect of the import.
if (import.meta.main) {
  try {
    switch (positional[0] ?? "list") {
      // list, status and drift are Rust now (tools/sjel-cli/src/harnesses/). Reaching them
      // here means this file was run directly instead of through tools/harnesses, so say so
      // rather than printing nothing.
      case "list": case "status": case "drift":
        throw new Error("the read verbs moved to Rust — run this through tools/harnesses");
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
