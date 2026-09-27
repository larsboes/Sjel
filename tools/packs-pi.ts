#!/usr/bin/env bun
// tools/packs-pi.ts — select Axon and overlay Packs for Pi's settings-managed registries.
// Pi scans each path in ~/.pi/agent/settings.json:skills and settings.json:extensions.
// Skills come from a pack's skills/ dir; a pack MAY also carry an extensions/ dir of
// pi-only .ts extensions (convention-based, no pack.toml field — the pi-exclusive
// counterpart of packs-claude's agents/ convention). Other adapters ignore it.
//
// A pack MAY also carry a pi-packages/ dir: one vendored pi package per
// subdirectory, registered as a PATH in settings.json `packages` so pi loads the
// source where it sits. That is the third artifact kind, and it exists because
// pi's own `pi install` has no way to keep a package inside a repo you edit: an
// npm source is opaque, and a git source is cloned elsewhere and reset on
// reconcile. A local path is neither — the checkout stays in the Pack, which is
// what makes a customization an edit instead of a re-vendor. Other adapters ignore
// this directory too.
//
// ── pi is a MIXED delivery model, and this file is where that shows ──────────
//
// Skills and extensions are REGISTERED: the pack source stays where it is and
// settings.json carries a path to it. Nothing is copied, so there is no drift.
//
// Agent files are MATERIALIZED, and they have to be. The pi-subagents extension
// reads agents from `$PI_CODING_AGENT_DIR/agents/*.md` on DISK and does not consult
// settings.json at all, and it does not recurse — so unlike Claude Code's
// `~/.claude/agents/<pack>/`, this destination is one FLAT directory shared by every
// pack. Two consequences, both load-bearing:
//
//   1. An agent file cannot be copied unchanged. Claude Code's `tools:` names are
//      TitleCase and pi's builtins are lowercase, and `Glob` does not exist in pi at
//      all (it searches with `find`). An unrecognised name is dropped from the
//      agent's allowlist with only a `tools-error` event, so a copied council member
//      would come up with no read tool. tools/lib/pi-agent-file.ts rewrites the file
//      and refuses rather than degrading.
//   2. Ownership is per FILE, not per pack directory, because every pack shares the
//      one destination root. That is what pack-deploy's flat-file convention is for.
//
// So this adapter drives two ledgers: its own settings ledger (below) for the paths
// it registered, and the shared deployment engine's ledger for the agent files it
// wrote. `status` reports both.
//
// This tool owns only paths recorded in its ledger, so remove never deletes a skill
// or extension selected outside Axon.

import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join, resolve, sep } from "node:path";
import { availablePacks, readPackSkills, type DeployConfig } from "./packs-codex.ts";
import {
  deployPack,
  getStatuses,
  packUnits,
  readProfiles,
  // Aliased deliberately. This file already defines a local readState() for its
  // settings ledger, and a bare `readState` import is shadowed by it — the local
  // function declaration wins, both look like they take the config, and the agent
  // checks then read the SETTINGS ledger and report every pack as having no agents.
  // That bug survived a direct call and only showed up in a deploy-then-remove run.
  readState as readDeploymentState,
  removePack,
  resolveProfilePacks,
  resolveProfileSkills,
  syncPack,
  type StatusRow,
} from "./lib/pack-deploy.ts";
import { translateAgentForPi } from "./lib/pi-agent-file.ts";

const SJEL_ROOT = resolve(import.meta.dir, "..");
const home = process.env.HOME ?? "";
const settingsPath = resolve(process.env.PI_SETTINGS_FILE ?? join(home, ".pi", "agent", "settings.json"));
const statePath = resolve(process.env.SJEL_PI_STATE_FILE ?? join(process.env.XDG_STATE_HOME ?? join(home, ".local", "state"), "axon", "pack-deployments", "pi.json"));
/**
 * The agent channel's ledger, kept apart from the settings ledger on purpose: the
 * two record different things (registered paths vs. materialized files), and merging
 * them would make one file speak two delivery models.
 */
const agentsStatePath = resolve(process.env.SJEL_PI_AGENTS_STATE_FILE ?? join(process.env.XDG_STATE_HOME ?? join(home, ".local", "state"), "axon", "pack-deployments", "pi-agents.json"));

/** Where pi-subagents looks for agent files: flat, no pack subdirectory, no recursion. */
function piAgentsRoot(): string {
  if (process.env.SJEL_PI_AGENTS_DIR) return expandHome(process.env.SJEL_PI_AGENTS_DIR);
  const agentDir = process.env.PI_CODING_AGENT_DIR ? expandHome(process.env.PI_CODING_AGENT_DIR) : join(home, ".pi", "agent");
  return join(agentDir, "agents");
}

type State = { version: 1; settingsPath: string; packs: Record<string, string[]>; extensions: Record<string, string[]>; packages: Record<string, string[]> };

function overlayRoot(): string | null {
  if (process.env.SJEL_OVERLAY_ROOT) return expandHome(process.env.SJEL_OVERLAY_ROOT);
  for (const file of [join(SJEL_ROOT, "axon.local.toml"), join(SJEL_ROOT, "axon.toml")]) {
    if (!existsSync(file)) continue;
    const overlay = (Bun.TOML.parse(readFileSync(file, "utf8")) as Record<string, unknown>).overlay;
    if (typeof overlay === "string" && overlay) return expandHome(overlay);
  }
  return null;
}

function expandHome(path: string): string {
  return path === "~" ? home : path.startsWith("~/") ? join(home, path.slice(2)) : path;
}

export function defaultPiDeployConfig(): DeployConfig {
  const roots = [join(SJEL_ROOT, "Packs")];
  const overlay = overlayRoot();
  if (overlay && existsSync(join(overlay, "Packs"))) roots.push(join(overlay, "Packs"));
  // destination is unused by Pi, but getStatuses needs a valid configuration.
  return { axonRoot: SJEL_ROOT, packRoots: roots, destination: join(home, ".pi", "agent", "skills"), stateFile: statePath, adapter: "pi" };
}

/**
 * The agent channel's config: the general engine pointed at a flat destination, with
 * manifest skills switched off because this adapter registers those rather than
 * copying them.
 *
 * Kept separate from defaultPiDeployConfig rather than folded into it because
 * tools/harnesses.ts asks that one for the SKILL status view, and a config whose
 * packUnits() returns no skills would quietly empty that view out.
 */
export function defaultPiAgentsDeployConfig(): DeployConfig {
  const cfg = defaultPiDeployConfig();
  return {
    ...cfg,
    destination: piAgentsRoot(),
    stateFile: agentsStatePath,
    stateEnvVar: "SJEL_PI_AGENTS_STATE_FILE",
    skipManifestSkills: true,
    flatFileConvention: {
      sourceDir: "agents",
      destinationRoot: piAgentsRoot(),
      transform: translateAgentForPi,
    },
  };
}

/** Whether this pack carries any flat-file unit to deploy — i.e. an agents/ dir. */
function agentUnits(pack: string) {
  return packUnits(defaultPiAgentsDeployConfig(), pack);
}

/** Whether this adapter's agent ledger owns anything for this pack. */
function agentsLedgerOwns(pack: string): boolean {
  const config = defaultPiAgentsDeployConfig();
  if (!existsSync(config.stateFile)) return false;
  return Boolean(readDeploymentState(config).packs[pack]);
}

function readSettings(): Record<string, unknown> {
  if (!existsSync(settingsPath)) return {};
  const value = JSON.parse(readFileSync(settingsPath, "utf8"));
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error(`${settingsPath}: expected a JSON object`);
  return value as Record<string, unknown>;
}

function settingSkills(settings: Record<string, unknown>): string[] {
  return stringArraySetting(settings, "skills");
}

function settingExtensions(settings: Record<string, unknown>): string[] {
  return stringArraySetting(settings, "extensions");
}

function settingPackages(settings: Record<string, unknown>): string[] {
  return stringArraySetting(settings, "packages");
}

function stringArraySetting(settings: Record<string, unknown>, key: string): string[] {
  const value = settings[key];
  if (value === undefined) return [];
  if (!Array.isArray(value) || value.some((item) => typeof item !== "string")) throw new Error(`${settingsPath}: ${key} must be an array of strings`);
  return value as string[];
}

function writeSettings(settings: Record<string, unknown>): void {
  mkdirSync(dirname(settingsPath), { recursive: true });
  const temporary = `${settingsPath}.tmp-${process.pid}`;
  writeFileSync(temporary, `${JSON.stringify(settings, null, 2)}\n`, { mode: 0o600 });
  renameSync(temporary, settingsPath);
}

function readState(): State {
  if (!existsSync(statePath)) return { version: 1, settingsPath, packs: {}, extensions: {}, packages: {} };
  const state = JSON.parse(readFileSync(statePath, "utf8")) as State;
  if (state.version !== 1 || !state.packs || resolve(state.settingsPath) !== settingsPath) throw new Error(`${statePath}: malformed or belongs to another Pi settings file`);
  state.extensions ??= {};
  state.packages ??= {};
  return state;
}

function writeState(state: State): void {
  mkdirSync(dirname(statePath), { recursive: true });
  const temporary = `${statePath}.tmp-${process.pid}`;
  writeFileSync(temporary, `${JSON.stringify(state, null, 2)}\n`, { mode: 0o600 });
  renameSync(temporary, statePath);
}

function pathsForPack(pack: string, subset?: Set<string>): string[] {
  const cfg = defaultPiDeployConfig();
  return readPackSkills(cfg, pack)
    .filter((skill) => !subset || subset.has(skill))
    .map((skill) => {
      const matches = (cfg.packRoots ?? []).map((root) => join(root, pack, "skills", skill)).filter(existsSync);
      if (matches.length !== 1) throw new Error(`${pack}/${skill}: source is missing or ambiguous`);
      return matches[0];
    });
}

function extensionsForPack(pack: string): string[] {
  const cfg = defaultPiDeployConfig();
  const names = new Set<string>();
  for (const root of cfg.packRoots ?? []) {
    const dir = join(root, pack, "extensions");
    if (!existsSync(dir)) continue;
    for (const entry of readdirSync(dir)) if (entry.endsWith(".ts")) names.add(entry);
  }
  return [...names].sort().map((name) => {
    const matches = (cfg.packRoots ?? []).map((root) => join(root, pack, "extensions", name)).filter(existsSync);
    if (matches.length !== 1) throw new Error(`${pack}/extensions/${name}: source is missing or ambiguous`);
    return matches[0];
  });
}

/**
 * Vendored pi packages a Pack carries, one directory each under `pi-packages/`.
 *
 * A directory is a package only if its package.json declares a `pi` manifest,
 * because that manifest is what pi reads to find the entry points; without it pi
 * would accept the path and load nothing, which is a silent failure rather than a
 * loud one. So that case throws here instead.
 *
 * The directory name is a convention at the pack root, like skills/ and
 * extensions/, and deliberately not a pack.toml field: it is pi-only, and this is
 * the adapter that understands it.
 */
function packagesForPack(pack: string): string[] {
  const cfg = defaultPiDeployConfig();
  const found: string[] = [];
  for (const root of cfg.packRoots ?? []) {
    const dir = join(root, pack, "pi-packages");
    if (!existsSync(dir)) continue;
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (!entry.isDirectory()) continue;
      const manifestPath = join(dir, entry.name, "package.json");
      if (!existsSync(manifestPath)) continue;
      const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as { pi?: unknown };
      if (typeof manifest.pi !== "object" || manifest.pi === null) {
        throw new Error(
          `${pack}/pi-packages/${entry.name}/package.json declares no 'pi' manifest, so pi would accept the `
            + `path and load nothing from it`,
        );
      }
      found.push(join(dir, entry.name));
    }
  }
  return found.sort();
}

/**
 * Registry-model profile activation: the pi settings file IS the selection, so
 * activating a profile rewrites settings.json's skills AND extensions to exactly
 * the profile's set (per-Pack skill subsets honoured), and drops ledger entries
 * for Pack(s) the profile does not name.
 *
 * WHAT IS DEPLOYED IS THE PROFILE'S DECISION AND NOTHING ELSE'S. A Pack used to be
 * able to veto its own deployment from pack.toml with a `# pi: REFUSED` line
 * (written 2026-09-10, retired 2026-09-17). That put a harness-specific fact in a
 * manifest the schema requires to stay harness-neutral — the same rule that keeps
 * `agents/`, `extensions/` and `codex/` out of `pack.toml` — and it made `full`
 * silently mean something other than "all packs". The operator decides what is
 * deployed where; a profile is where that decision is written down, and
 * `tools/harnesses status` is where it is read back.
 */
export function activateProfileOnPi(profileName: string): void {
  const config = defaultPiDeployConfig();
  const profiles = readProfiles(config);
  const profile = profiles.find((p) => p.name === profileName);
  if (!profile) throw new Error(`no such profile: '${profileName}'`);
  const target = new Set(resolveProfilePacks(config, profile));
  const subsets = resolveProfileSkills(config, profile);
  const state = readState();
  const settings = readSettings();
  const messages: string[] = [];
  messages.push(`Activating profile '${profile.name}' — ${profile.description}`);
  const active: string[] = [];
  const skills: string[] = [];
  const extensions: string[] = [];
  const packages: string[] = [];
  for (const pack of [...target].sort()) {
    const paths = pathsForPack(pack, subsets.get(pack) ?? undefined);
    const extPaths = extensionsForPack(pack);
    const pkgPaths = packagesForPack(pack);
    skills.push(...paths);
    extensions.push(...extPaths);
    packages.push(...pkgPaths);
    active.push(pack);
    messages.push(`  → ${pack}: ${paths.length} skill(s), ${extPaths.length} extension(s), ${pkgPaths.length} package(s)`);
  }
  const removed = Object.keys(state.packs).filter((p) => !active.includes(p)).sort().join(", ");
  if (removed) messages.push(`Removing Pack(s) not in profile: ${removed}`);
  // The settings registry is written above; agent files are materialized here. A
  // profile activation is the one place both channels have to move together, or a
  // pack would be registered with its agent types missing.
  messages.push(...activateAgentsForProfile([...target].sort()));
  settings.skills = dedupeCanonical(skills);
  settings.extensions = dedupeCanonical(extensions);
  settings.packages = dedupeCanonical(packages);
  state.packs = Object.fromEntries(active.map((p) => [p, pathsForPack(p, subsets.get(p) ?? undefined)]));
  state.extensions = Object.fromEntries(active.map((p) => [p, extensionsForPack(p)]));
  state.packages = Object.fromEntries(active.map((p) => [p, packagesForPack(p)]));
  writeSettings(settings);
  writeState(state);
  for (const m of messages) console.log(m);
  console.log(
    `✓ profile '${profile.name}' active for pi: ${active.length} pack(s), ${settings.skills.length} skill(s), ${settings.extensions.length} extension(s), ${settings.packages.length} package(s)${removed ? `; removed ${removed}` : ""}`,
  );
}

function status(packs: string[]): void {
  const selected = packs.length ? packs : availablePacks(defaultPiDeployConfig(), true);
  const settings = readSettings();
  const skills = settingSkills(settings);
  const extensions = settingExtensions(settings);
  const packages = settingPackages(settings);
  const state = readState();
  for (const pack of selected) {
    for (const path of pathsForPack(pack)) {
      const selected = skills.includes(path);
      const owned = state.packs[pack]?.includes(path) ?? false;
      console.log(`${pack}/${path.split("/").slice(-1)[0]}: ${selected ? (owned ? "current" : "selected-unmanaged") : "not-deployed"}`);
    }
    for (const path of extensionsForPack(pack)) {
      const selected = extensions.includes(path);
      const owned = state.extensions[pack]?.includes(path) ?? false;
      console.log(`${pack}/extensions/${path.split("/").slice(-1)[0]}: ${selected ? (owned ? "current" : "selected-unmanaged") : "not-deployed"}`);
    }
    for (const path of packagesForPack(pack)) {
      const selected = packages.includes(path);
      const owned = state.packages[pack]?.includes(path) ?? false;
      console.log(`${pack}/pi-packages/${path.split("/").slice(-1)[0]}: ${selected ? (owned ? "current" : "selected-unmanaged") : "not-deployed"}`);
    }
  }
  printAgentStatuses(packs);
}

/**
 * Agent rows, in the same shape as the settings rows so one status listing reads as
 * one table. Separate because the two channels answer different questions: the
 * settings rows ask "is this path registered", these ask "are the bytes at that flat
 * destination the ones this Pack would write".
 */
function printAgentStatuses(packs: string[]): void {
  const config = defaultPiAgentsDeployConfig();
  const selected = packs.length ? packs : availablePacks(config, true);
  for (const pack of selected) {
    if (agentUnits(pack).length === 0 && !agentsLedgerOwns(pack)) continue;
    for (const row of getStatuses(config, pack) as StatusRow[]) {
      const state = row.status === "current" ? "current" : row.status;
      console.log(`${pack}/${row.skill}: ${state}${row.detail ? ` (${row.detail})` : ""}`);
    }
  }
}

function canonicalPath(path: string): string {
  return resolve(expandHome(path));
}

/**
 * Add each path to a settings array, replacing an equivalent entry in place rather
 * than appending a second spelling of it. One helper for all three artifact kinds,
 * because the loop was otherwise copied per kind and the third copy is where a
 * dedupe that only two kinds got would have gone unnoticed.
 *
 * Non-path sources (`npm:pi-web-access`) are left exactly as they are: they are
 * already in the array, nothing here removes, and resolve() on one produces a
 * harmless synthetic key that no real path collides with.
 */
function registerInto(target: string[], paths: string[]): void {
  for (const path of paths) {
    const duplicate = target.findIndex((existing) => canonicalPath(existing) === canonicalPath(path));
    if (duplicate === -1) target.push(path);
    else if (target[duplicate] !== path) target[duplicate] = path;
  }
}

/**
 * Drop registration entries that point where this deployment owns but nothing exists.
 *
 * `deploy` only ever ADDED, so pi's registry accumulated dead paths. Deleting a skill
 * from its Pack, or moving a Pack between roots, left the old path in settings.json
 * forever — three of twenty-seven entries on 2026-09-11 pointed at directories that no
 * longer existed, including a retired skill and two that had moved out of the private
 * overlay. Pi was being asked to load them on every start. `activateProfile` rebuilds
 * the whole array and so never had the bug; this makes the incremental verb as clean as
 * the wholesale one.
 *
 * Two conditions, and BOTH are required. The path must be GONE from disk, and it must
 * sit under a Pack root this deployment reads from. An npm source (`npm:foo`) is not a
 * path at all, and a skill the operator keeps outside our roots is theirs to manage —
 * neither is ever touched. That is the same property the vendored-package tests already
 * pin, and the reason this prunes rather than replacing the array with the desired set.
 */
function pruneDeadOwnedPaths(paths: string[], config: DeployConfig): string[] {
  const roots = (config.packRoots ?? []).map((root) => resolve(root) + sep);
  return paths.filter((path) => {
    const absolute = resolve(expandHome(path));
    const ours = roots.some((root) => absolute.startsWith(root));
    return !ours || existsSync(absolute);
  });
}

function deploy(packs: string[]): void {
  if (!packs.length) throw new Error("deploy needs one or more Pack names");
  const config = defaultPiDeployConfig();
  const settings = readSettings();
  const skills = settingSkills(settings);
  const extensions = settingExtensions(settings);
  const packages = settingPackages(settings);
  const state = readState();
  for (const pack of packs) {
    const paths = pathsForPack(pack);
    const extPaths = extensionsForPack(pack);
    const pkgPaths = packagesForPack(pack);
    registerInto(skills, paths);
    registerInto(extensions, extPaths);
    registerInto(packages, pkgPaths);
    state.packs[pack] = paths;
    state.extensions[pack] = extPaths;
    state.packages[pack] = pkgPaths;
    console.log(
      `✓ ${pack}: ${paths.length} skill(s), ${extPaths.length} extension(s), ${pkgPaths.length} package(s) selected for Pi`,
    );
  }
  // Pi treats equivalent absolute and ~/ paths as the same source. Keep one path
  // so its startup discovery does not report a duplicate skill name.
  settings.skills = dedupeCanonical(pruneDeadOwnedPaths(skills, config));
  settings.extensions = dedupeCanonical(pruneDeadOwnedPaths(extensions, config));
  settings.packages = dedupeCanonical(pruneDeadOwnedPaths(packages, config));
  // The settings ledger is trimmed on the same terms, so `remove` cannot later be asked
  // to remove a path that is already gone, and `status` does not report a phantom row.
  for (const channel of ["packs", "extensions", "packages"] as const) {
    for (const pack of Object.keys(state[channel])) {
      state[channel][pack] = pruneDeadOwnedPaths(state[channel][pack], config);
    }
  }
  writeSettings(settings);
  writeState(state);
  deployAgents(packs);
}

function dedupeCanonical(paths: string[]): string[] {
  return paths.filter((path, index) =>
    paths.findIndex((candidate) => canonicalPath(candidate) === canonicalPath(path)) === index,
  );
}

function remove(packs: string[]): void {
  if (!packs.length) throw new Error("remove needs one or more Pack names");
  const settings = readSettings();
  const state = readState();
  const removePaths = new Set<string>();
  const removeExtensions = new Set<string>();
  const removePackages = new Set<string>();
  for (const pack of packs) {
    const paths = state.packs[pack];
    if (!paths) throw new Error(`${pack}: not owned by Axon Pi deployment`);
    paths.forEach((path) => removePaths.add(path));
    (state.extensions[pack] ?? []).forEach((path) => removeExtensions.add(path));
    (state.packages[pack] ?? []).forEach((path) => removePackages.add(path));
    delete state.packs[pack];
    delete state.extensions[pack];
    delete state.packages[pack];
    console.log(`✓ ${pack}: removed from Pi selection`);
  }
  settings.skills = settingSkills(settings).filter((path) => !removePaths.has(path));
  settings.extensions = settingExtensions(settings).filter((path) => !removeExtensions.has(path));
  settings.packages = settingPackages(settings).filter((path) => !removePackages.has(path));
  writeSettings(settings);
  writeState(state);
  removeAgents(packs);
}

/**
 * Materialize the agent files of the named Packs.
 *
 * A Pack with no agents/ directory is skipped rather than reported, because that is
 * the common case and not a fault. The engine writes each file individually and
 * records its digest, so a later sync can tell a hand edit from a source change.
 */
function deployAgents(packs: string[]): void {
  const config = defaultPiAgentsDeployConfig();
  for (const pack of packs) {
    if (agentUnits(pack).length === 0) {
      if (!agentsLedgerOwns(pack)) continue;
      // The Pack dropped its agents/ directory; take the deployed copies with it.
      console.log(`  ${pack}: agents/ is gone from the Pack, removing ${removeAgentsFor(config, pack)} file(s)`);
      continue;
    }
    const deployed = agentsLedgerOwns(pack);
    const messages = deployed ? syncPack(config, pack) : deployPack(config, pack);
    console.log(`  ${pack}: ${messages.length} agent file(s) at ${config.flatFileConvention!.destinationRoot}`);
  }
}

/** Remove every agent file this adapter's ledger owns for a Pack. Returns the count. */
function removeAgentsFor(config: DeployConfig, pack: string): number {
  if (!agentsLedgerOwns(pack)) return 0;
  const count = Object.keys(readDeploymentState(config).packs[pack]?.skills ?? {}).length;
  removePack(config, pack);
  return count;
}

function removeAgents(packs: string[]): void {
  const config = defaultPiAgentsDeployConfig();
  for (const pack of packs) {
    const removed = removeAgentsFor(config, pack);
    if (removed > 0) console.log(`  ${pack}: ${removed} agent file(s) removed`);
  }
}

/**
 * Move the agent channel with a profile activation. Called by activateProfileOnPi
 * after it rewrites settings.json.
 *
 * Deliberately NOT the engine's own activateProfile: that one resolves per-Pack skill
 * subsets against packUnits(), and this config deploys no skills, so every subset in
 * profiles.toml would fail validation as an unknown skill.
 */
function activateAgentsForProfile(activePacks: string[]): string[] {
  const config = defaultPiAgentsDeployConfig();
  const messages: string[] = [];
  const ledgerPacks = existsSync(config.stateFile) ? Object.keys(readDeploymentState(config).packs) : [];
  for (const pack of ledgerPacks.filter((p) => !activePacks.includes(p)).sort()) {
    const removed = removeAgentsFor(config, pack);
    if (removed > 0) messages.push(`  → agents: ${pack} removed (${removed} file(s))`);
  }
  for (const pack of activePacks) {
    if (agentUnits(pack).length === 0) continue;
    const messages2 = agentsLedgerOwns(pack) ? syncPack(config, pack) : deployPack(config, pack);
    messages.push(`  → agents: ${pack} (${messages2.length} file(s))`);
  }
  return messages;
}

function usage(): never {
  throw new Error("usage: tools/packs-pi list | status [pack ...] | deploy <pack ...> | sync <pack ...> | remove <pack ...> | use <profile>");
}
// Guarded so this module can be imported for its exported DeployConfig without
// running the CLI — tools/lib/harness-registry.ts does exactly that, and an
// unguarded top-level block printed a full status listing on import.
if (import.meta.main) {

  try {
    const [command = "list", ...args] = process.argv.slice(2);
    if (command === "list") {
      for (const pack of availablePacks(defaultPiDeployConfig(), true)) console.log(pack);
    } else if (command === "status") status(args);
    else if (command === "deploy" || command === "sync") deploy(args);
    else if (command === "remove") remove(args);
    else if (command === "use" || command === "profile") {
      const profileName = args[0];
      if (!profileName) throw new Error("usage: tools/packs-pi use <profile>");
      activateProfileOnPi(profileName);
    }
    else usage();
  } catch (error) {
    console.error(`packs-pi: ${(error as Error).message}`);
    process.exit(1);
  }
}
