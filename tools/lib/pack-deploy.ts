// tools/lib/pack-deploy.ts — the Pack deployment engine, shared by every harness adapter.
//
// Axon is the source of truth. A deployer materializes Packs into a harness'
// skill root by COPYING: each unit is assembled into a staging directory,
// validated, and atomically installed. A state ledger records ownership and the
// last installed digest, so sync and remove can refuse to erase destination-side
// edits or anything this deployment does not own.
//
// Extracted from tools/packs-codex.ts on 2026-08-09, when the Claude adapter
// stopped using symlinks (principal: "we should only deploy from axon overlays
// never using symlinks"). A symlink's target WAS its ownership proof — reading it
// told you whether a directory was ours to remove. Copies destroy that proof, so
// the ledger has to supply it, and the ledger already existed here. Two adapters
// hand-rolling the same digest-and-ownership logic is the duplication Axon's own
// "generic in Axon, specific in the overlay" rule exists to prevent.
//
// Nothing in this file may name a specific harness. Adapter differences arrive
// through DeployConfig: the overlay directory name, the state-file env var named
// in errors, an optional extra validator, and an optional whole-directory
// convention. A hardcoded "codex" or "claude" here is a bug.

import {
  chmodSync,
  closeSync,
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmdirSync,
  rmSync,
  statSync,
  writeFileSync,
  writeSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { basename, dirname, join, relative, resolve, sep } from "node:path";

export const DIGEST_POLICY = "exclude-python-generated-v1" as const;

type SkillRecord = {
  source: string;
  desiredDigest: string;
  installedDigest: string;
  digestPolicy?: typeof DIGEST_POLICY;
  deployedAt: string;
};

type PackRecord = {
  // Named `skills` on the wire because deployed ledgers already use that key and
  // a rename would strand every existing deployment. It holds units — a skill, or
  // a whole-directory convention like Claude's agents/.
  skills: Record<string, SkillRecord>;
};

export type DeploymentState = {
  version: 1;
  destination: string;
  packs: Record<string, PackRecord>;
};

export type DesiredFile = {
  absolutePath: string;
  relativePath: string;
  mode: number;
  /**
   * The bytes to install, when they are not the source file's own. Only a flat-file
   * unit sets this, because its `transform` rewrote the file on the way out. Every
   * reader — digestFiles, materializeStage — must prefer it over absolutePath, or a
   * transformed deployment reports permanent drift against its own source.
   */
  content?: Buffer;
};

export type DeployConfig = {
  axonRoot: string;
  /** Ordered Pack roots. The first root is the public Axon source of truth. */
  packRoots?: string[];
  destination: string;
  stateFile: string;
  /** Names the per-harness overlay directory in a Pack, and the temp-path prefixes. */
  adapter: string;
  /** Env var named in the "state belongs to another destination" error, so the fix is in the message. */
  stateEnvVar?: string;
  /** Extra per-adapter validation of an assembled skill. Codex checks agents/openai.yaml here. */
  validateAdapterFiles?: (files: Map<string, DesiredFile>, label: string) => void;
  /**
   * A whole directory a Pack may carry, deployed as ONE owned unit under its own
   * root — Claude Code's agents/, where a single directory exposes every agent
   * inside it. Deliberately not part of pack.toml: it is a per-harness
   * convention, and an adapter that knows nothing about it simply omits this.
   */
  treeConvention?: { sourceDir: string; destinationRoot: string };
  /**
   * A directory of independent files a Pack may carry, where EVERY FILE deploys as
   * its own owned unit into a SHARED destination directory.
   *
   * Distinct from `treeConvention`, and pi forced the distinction (2026-09-16): pi's
   * agent loader reads `*.md` directly in one flat directory and does NOT recurse, so
   * the per-Pack subdirectory a tree convention produces would be invisible to it.
   * Two Packs therefore share that one destination, which rules out a single
   * whole-directory unit — ownership has to be per file. `ownerOf` already compares
   * resolved destinations for exactly this class of problem, so it holds.
   *
   * `transform` runs on each file's bytes on the way out, because pi's agent files
   * are not byte-compatible with Claude Code's (tools/lib/pi-agent-file.ts). The
   * digest is taken over the TRANSFORMED bytes, so a transform is never drift.
   */
  flatFileConvention?: {
    sourceDir: string;
    destinationRoot: string;
    transform?: (content: string, label: string) => string;
  };
  /**
   * Deploy the flat-file convention and nothing else. For a harness that owns its
   * skill selection by some other means — pi registers paths in settings.json rather
   * than materializing them — so that reusing this engine for its agent files does
   * not also copy every skill to a destination nothing reads.
   */
  skipManifestSkills?: boolean;
};

export type SkillStatus =
  | "not-deployed"
  | "current"
  | "outdated"
  | "drifted"
  | "migration-required"
  | "missing"
  | "collision"
  | "invalid"
  | "discovered";

export type StatusRow = {
  pack: string;
  skill: string;
  status: SkillStatus;
  detail?: string;
};

/**
 * One deployable thing. A skill lives under the Pack's skills/ and validates as a
 * skill; a tree unit is the whole-directory convention and has no SKILL.md.
 *
 * `key` is what the ledger records. A tree's key keeps its trailing slash, which
 * `assertSimpleName` rejects — so a tree can never collide with a skill name, and
 * the impossibility is structural rather than a reserved-word list to maintain.
 */
export type Unit = {
  key: string;
  sourceRoot: string;
  destination: string;
  isSkill: boolean;
  /**
   * Set on a flat-file unit: the one file inside `sourceRoot` this unit owns. The
   * directory stays the source root because collectFiles owns the symlink and
   * entry-type checks, and running them per file beats a second implementation.
   */
  onlyFile?: string;
};

function emptyState(destination: string): DeploymentState {
  return { version: 1, destination, packs: {} };
}

export function readState(config: DeployConfig): DeploymentState {
  if (!existsSync(config.stateFile)) return emptyState(config.destination);
  let parsed: unknown;
  try {
    parsed = JSON.parse(readFileSync(config.stateFile, "utf8"));
  } catch (error) {
    throw new Error(`cannot read ${config.adapter} deployment state ${config.stateFile}: ${error}`);
  }
  if (
    typeof parsed !== "object" ||
    parsed === null ||
    (parsed as any).version !== 1 ||
    typeof (parsed as any).packs !== "object"
  ) {
    throw new Error(`unsupported or malformed ${config.adapter} deployment state: ${config.stateFile}`);
  }
  const state = parsed as DeploymentState;
  if (resolve(state.destination) !== resolve(config.destination)) {
    const hint = config.stateEnvVar ? `; set ${config.stateEnvVar} for this destination` : "";
    throw new Error(`state file belongs to ${state.destination}, not ${config.destination}${hint}`);
  }
  return state;
}

/**
 * Hold the ledger for the length of one mutating operation.
 *
 * writeState is atomic — a temp file and a rename, so no reader ever sees half a
 * ledger. That is not the race. Every mutator reads the whole state ONCE, mutates
 * the in-memory copy, and writes the whole file back one or more times
 * (deployPack: read at the top, write inside the per-unit loop). Two processes
 * that overlap therefore each hold a snapshot taken before the other's writes, and
 * the one that finishes last silently erases the other's entries. A skill would
 * still be on disk with no ledger row: reported as an unowned collision, and
 * un-syncable until somebody adopts it by hand.
 *
 * Nothing had overlapped yet, because every run was a human at a terminal. The
 * lock is the precondition for anything unattended — a scheduled check, a hook, a
 * second session — and this repository is already running two sessions at once.
 *
 * Re-entrant on purpose: activateProfile calls removePack and deployPack, which
 * are themselves mutators, and a lock that deadlocked on its own caller would be
 * a worse bug than the one it fixes.
 */
let lockDepth = 0;
let heldLockPath: string | null = null;

const LOCK_STALE_MS = 30_000;
/** How long to wait for another process to finish before refusing. Env-overridable
 *  so a test does not have to spend the real wait, and so an operator on a slow
 *  filesystem can raise it without a rebuild. */
const LOCK_WAIT_MS = Number(process.env.SJEL_PACK_LOCK_WAIT_MS ?? 4_000);

function lockPathFor(config: DeployConfig): string {
  return `${config.stateFile}.lock`;
}

/** Alive as far as this user can tell. A pid we cannot signal is treated as alive. */
function pidIsAlive(pid: number): boolean {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}

export function withStateLock<T>(config: DeployConfig, run: () => T): T {
  const path = lockPathFor(config);
  if (lockDepth > 0 && heldLockPath === path) {
    lockDepth += 1;
    try {
      return run();
    } finally {
      lockDepth -= 1;
    }
  }

  const deadline = Date.now() + LOCK_WAIT_MS;
  for (;;) {
    try {
      mkdirSync(dirname(path), { recursive: true });
      const handle = openSync(path, "wx", 0o600);
      writeSync(handle, `${JSON.stringify({ pid: process.pid, at: new Date().toISOString() })}\n`);
      closeSync(handle);
      break;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
      // Someone holds it. Steal only from a holder that is provably gone, or from a
      // lock old enough that a crashed holder is the only explanation — a stale lock
      // that never expires turns one killed process into a permanently broken tool.
      let holder = -1;
      let ageMs = Number.POSITIVE_INFINITY;
      try {
        holder = (JSON.parse(readFileSync(path, "utf8")) as { pid?: number }).pid ?? -1;
        ageMs = Date.now() - statSync(path).mtimeMs;
      } catch {
        ageMs = Number.POSITIVE_INFINITY; // unreadable lock: treat as stale
      }
      if (!pidIsAlive(holder) || ageMs > LOCK_STALE_MS) {
        rmSync(path, { force: true });
        continue;
      }
      if (Date.now() > deadline) {
        throw new Error(
          `${config.adapter} ledger is locked by pid ${holder} (${path}); it has been held for ${Math.round(ageMs / 1000)}s. Wait for that run to finish, or remove the lock file if that process is gone`,
        );
      }
      Bun.sleepSync(50);
    }
  }

  lockDepth = 1;
  heldLockPath = path;
  try {
    return run();
  } finally {
    lockDepth = 0;
    heldLockPath = null;
    rmSync(path, { force: true });
  }
}

function writeState(config: DeployConfig, state: DeploymentState): void {
  mkdirSync(dirname(config.stateFile), { recursive: true });
  const temp = `${config.stateFile}.tmp-${process.pid}`;
  writeFileSync(temp, `${JSON.stringify(state, null, 2)}\n`, { mode: 0o600 });
  renameSync(temp, config.stateFile);
}

function assertSimpleName(value: string, label: string): void {
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(value)) {
    throw new Error(`${label} '${value}' must be lowercase hyphen-case`);
  }
}

function packRoots(config: DeployConfig): string[] {
  return config.packRoots?.length ? config.packRoots : [join(config.axonRoot, "Packs")];
}

function packDir(config: DeployConfig, pack: string): string {
  assertSimpleName(pack, "pack");
  const matches = packRoots(config).filter((root) => existsSync(join(root, pack, "pack.toml")));
  if (matches.length === 0) return join(packRoots(config)[0], pack);
  if (matches.length > 1) {
    throw new Error(`pack '${pack}' is declared in more than one Pack root: ${matches.join(", ")}`);
  }
  return join(matches[0], pack);
}

export function readPackSkills(config: DeployConfig, pack: string): string[] {
  const dir = packDir(config, pack);
  const manifest = join(dir, "pack.toml");
  if (!existsSync(manifest)) throw new Error(`no such pack: ${pack}`);
  const parsed = Bun.TOML.parse(readFileSync(manifest, "utf8")) as Record<string, unknown>;
  if (parsed.name !== pack) throw new Error(`${manifest}: name must match directory '${pack}'`);
  if (!Array.isArray(parsed.skills) || parsed.skills.some((skill) => typeof skill !== "string")) {
    throw new Error(`${manifest}: skills must be an array of names`);
  }
  const skills = parsed.skills as string[];
  const seen = new Set<string>();
  for (const skill of skills) {
    assertSimpleName(skill, "skill");
    if (seen.has(skill)) throw new Error(`${manifest}: duplicate skill '${skill}'`);
    seen.add(skill);
  }
  return skills;
}

/** The tree unit's ledger key. Trailing slash on purpose — see Unit.key. */
export function treeKey(sourceDir: string): string {
  return `${sourceDir}/`;
}

/**
 * A flat-file unit's ledger key: the source directory plus the file's own name.
 * No trailing slash, so it can never collide with a tree key, and the dot keeps it
 * out of the lowercase-hyphen-case skill namespace.
 */
export function flatKey(sourceDir: string, file: string): string {
  return `${sourceDir}/${file}`;
}

/**
 * The files a Pack carries under a flat source directory, sorted for a stable
 * deploy order. A non-`.md` entry is an error rather than a silent skip: the harness
 * loads `.md` only, so anything else in there is either a stray or a rename that
 * would make an agent vanish from the harness with nothing reported.
 */
function flatSourceFiles(config: DeployConfig, pack: string, sourceDir: string): string[] {
  const dir = join(packDir(config, pack), sourceDir);
  if (!existsSync(dir)) return [];
  const files: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name.startsWith(".")) continue;
    if (!entry.isFile()) throw new Error(`${pack}/${sourceDir}: ${entry.name} is not a file`);
    if (!entry.name.endsWith(".md")) {
      throw new Error(
        `${pack}/${sourceDir}/${entry.name}: only .md files deploy from a flat-file convention; `
          + `the harness loads nothing else, so this file would be silently inert`,
      );
    }
    files.push(entry.name);
  }
  return files.sort();
}

/**
 * Every unit a Pack deploys: its manifest skills, plus the tree convention when
 * the adapter declares one and the Pack actually carries that directory.
 */
export function packUnits(config: DeployConfig, pack: string): Unit[] {
  const dir = packDir(config, pack);
  // Read the manifest even when its skills are not deployed, so a mistyped Pack name
  // still fails here rather than deploying nothing and reporting success.
  const manifestSkills = readPackSkills(config, pack);
  const units: Unit[] = config.skipManifestSkills
    ? []
    : manifestSkills.map((skill) => ({
        key: skill,
        sourceRoot: join(dir, "skills", skill),
        destination: join(config.destination, skill),
        isSkill: true,
      }));
  const tree = config.treeConvention;
  if (tree && existsSync(join(dir, tree.sourceDir))) {
    units.push({
      key: treeKey(tree.sourceDir),
      sourceRoot: join(dir, tree.sourceDir),
      destination: join(tree.destinationRoot, pack),
      isSkill: false,
    });
  }
  const flat = config.flatFileConvention;
  if (flat) {
    for (const file of flatSourceFiles(config, pack, flat.sourceDir)) {
      units.push({
        key: flatKey(flat.sourceDir, file),
        sourceRoot: join(dir, flat.sourceDir),
        destination: join(flat.destinationRoot, file),
        isSkill: false,
        onlyFile: file,
      });
    }
  }
  return units;
}

function isGeneratedArtifactPath(relativePath: string): boolean {
  const parts = relativePath.split("/");
  return parts.includes("__pycache__") || /\.py[cod]$/.test(parts.at(-1) ?? "");
}

function collectFiles(
  config: DeployConfig,
  root: string,
  sourceLabel: string,
  includeGeneratedArtifacts = false,
): Map<string, DesiredFile> {
  const files = new Map<string, DesiredFile>();
  if (!existsSync(root)) return files;
  const rootStat = lstatSync(root);
  if (rootStat.isSymbolicLink()) {
    throw new Error(`${sourceLabel} is a symlink; ${config.adapter} deployment must be materialized`);
  }
  if (!rootStat.isDirectory()) throw new Error(`${sourceLabel} is not a directory: ${root}`);

  const visit = (dir: string): void => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (entry.name === ".DS_Store") continue;
      const absolutePath = join(dir, entry.name);
      const rel = relative(root, absolutePath).split(sep).join("/");
      if (!includeGeneratedArtifacts && isGeneratedArtifactPath(rel)) continue;
      const lst = lstatSync(absolutePath);
      if (lst.isSymbolicLink()) {
        throw new Error(`${sourceLabel} contains a symlink; ${config.adapter} deployment must be materialized: ${rel}`);
      }
      if (lst.isDirectory()) visit(absolutePath);
      else if (lst.isFile()) {
        files.set(rel, { absolutePath, relativePath: rel, mode: lst.mode & 0o777 });
      } else {
        throw new Error(`${sourceLabel} contains unsupported filesystem entry: ${rel}`);
      }
    }
  };
  visit(root);
  return files;
}

/**
 * The assembled file set for a unit: its shared source, with the adapter overlay
 * merged over it. A tree unit takes no overlay — the convention is already
 * per-harness, so there is nothing to specialize it against.
 *
 * SKILL.md may never be overridden by an overlay. That guard used to fire for the
 * codex adapter alone; making it universal is strictly stricter and matches the
 * stated rule that shared instructions stay canonical.
 */
export function desiredFiles(config: DeployConfig, pack: string, unit: Unit): Map<string, DesiredFile> {
  if (!existsSync(unit.sourceRoot)) throw new Error(`${pack}/${unit.key}: source missing at ${unit.sourceRoot}`);
  const collected = collectFiles(config, unit.sourceRoot, `${pack}/${unit.key}`);
  if (!unit.isSkill) {
    if (!unit.onlyFile) return collected;
    const file = collected.get(unit.onlyFile);
    if (!file) throw new Error(`${pack}/${unit.key}: source file '${unit.onlyFile}' missing from ${unit.sourceRoot}`);
    // The transform runs here, on the way out, so every consumer downstream —
    // digestFiles, materializeStage, getStatuses — sees the bytes the harness will
    // actually read. Hashing the source instead would report permanent drift.
    const transform = config.flatFileConvention?.transform;
    const content = transform
      ? Buffer.from(transform(readFileSync(file.absolutePath, "utf8"), `${pack}/${unit.key}`), "utf8")
      : undefined;
    return new Map([[file.relativePath, { ...file, content }]]);
  }
  const files = collected;

  // A Pack level `shared/` merge lived here briefly: built and removed on 2026-09-17, after
  // a review asked the obvious question — must a skill be self-contained? It materialized a
  // Pack's shared/ into EVERY skill unit at `<skill>/shared/...`, harness-neutrally, so a
  // script could open a file at a path that held on every surface.
  //
  // Two things killed it. It made the SOURCE skill incomplete: the deployed skill was
  // self-contained, but in the repo the file was absent, so the tree a human reads and
  // reviews did not show a dependency the skill actually had, and any copy not made by this
  // deployer broke at runtime. And it went to every skill of the Pack, which is why the one
  // thing big enough to want it — a 500-line tell catalog — could never use it.
  //
  // What it bought was that two lists were the same BYTES. The property that matters is that
  // they AGREE, and a gate proves that without any of the cost: see
  // tools/presentations-theme-contract.test.sh, which fails if a role one skill requires is
  // not one the other's themes define. A skill must be able to RUN from its own directory;
  // it may still POINT at a sibling by name for material it merely reads.
  const overlayRoot = join(packDir(config, pack), config.adapter, unit.key);
  const overlay = collectFiles(config, overlayRoot, `${pack}/${unit.key} ${config.adapter} overlay`);
  if (overlay.has("SKILL.md")) {
    throw new Error(`${pack}/${unit.key}: ${config.adapter} overlay may not override canonical SKILL.md`);
  }
  for (const [rel, file] of overlay) files.set(rel, file);
  return files;
}

export function digestFiles(files: Map<string, DesiredFile>): string {
  const hash = createHash("sha256");
  for (const rel of [...files.keys()].sort()) {
    const file = files.get(rel)!;
    hash.update(`${rel}\0${file.mode.toString(8)}\0`);
    hash.update(file.content ?? readFileSync(file.absolutePath));
    hash.update("\0");
  }
  return hash.digest("hex");
}

export function digestTree(config: DeployConfig, root: string): string {
  return digestFiles(collectFiles(config, root, root));
}

function legacyDigestTree(config: DeployConfig, root: string): string {
  return digestFiles(collectFiles(config, root, root, true));
}

/**
 * The digest of whatever is installed at a destination, which is a directory for a
 * skill or a tree unit and a single FILE for a flat-file unit.
 *
 * A file is hashed through digestFiles under the same relative path its source
 * carries, which is its basename on both sides, so a desired digest and an installed
 * digest of the same bytes stay comparable. Every drift check here assumes that, and
 * the alternative — teaching each of the eight call sites to branch — would put the
 * assumption in eight places instead of one.
 */
function digestDestination(config: DeployConfig, destination: string): string {
  const stats = statSync(destination);
  if (stats.isDirectory()) return digestTree(config, destination);
  return digestFileAt(destination, stats.mode & 0o777);
}

/** The legacy-policy counterpart of digestDestination. Generated-artifact exclusion
 *  has no meaning for a single file, so both policies agree there. */
function legacyDigestDestination(config: DeployConfig, destination: string): string {
  const stats = statSync(destination);
  if (stats.isDirectory()) return legacyDigestTree(config, destination);
  return digestFileAt(destination, stats.mode & 0o777);
}

function digestFileAt(path: string, mode: number): string {
  const name = basename(path);
  return digestFiles(new Map([[name, { absolutePath: path, relativePath: name, mode }]]));
}

function adoptDigestPolicyIfSafe(
  config: DeployConfig,
  record: SkillRecord,
  destination: string,
  unitKey: string,
): void {
  if (record.digestPolicy === DIGEST_POLICY) {
    if (digestDestination(config, destination) !== record.installedDigest) {
      throw new Error(`${unitKey}: installed copy has local changes`);
    }
    return;
  }
  if (legacyDigestDestination(config, destination) !== record.installedDigest) {
    throw new Error(
      `${unitKey}: legacy digest is ambiguous; review the destination and run ` +
        `migrate-generated <pack> --accept-current`,
    );
  }
  record.installedDigest = digestDestination(config, destination);
  record.digestPolicy = DIGEST_POLICY;
}

type GeneratedArtifacts = { files: string[]; directories: string[] };

function knownGeneratedArtifacts(root: string, label: string): GeneratedArtifacts {
  const files: string[] = [];
  const directories: string[] = [];
  const visit = (dir: string, insideCache: boolean): void => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const absolutePath = join(dir, entry.name);
      const rel = relative(root, absolutePath).split(sep).join("/");
      const inCache = insideCache || entry.name === "__pycache__";
      const generatedFile = /\.py[cod]$/.test(entry.name);
      const lst = lstatSync(absolutePath);
      if (inCache) {
        if (lst.isSymbolicLink()) {
          throw new Error(`${label}: generated-artifact migration refuses symlink ${rel}`);
        }
        if (lst.isDirectory()) {
          directories.push(absolutePath);
          visit(absolutePath, true);
        } else if (lst.isFile() && (generatedFile || entry.name === ".DS_Store")) {
          files.push(absolutePath);
        } else {
          throw new Error(`${label}: unknown content inside __pycache__: ${rel}`);
        }
      } else if (generatedFile) {
        if (!lst.isFile()) {
          throw new Error(`${label}: generated-artifact migration refuses non-file ${rel}`);
        }
        files.push(absolutePath);
      } else if (lst.isDirectory() && !lst.isSymbolicLink()) {
        visit(absolutePath, false);
      }
    }
  };
  visit(root, false);
  return { files, directories };
}

function extractFrontmatter(skillMd: string, label: string): Record<string, unknown> {
  const match = skillMd.match(/^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/);
  if (!match) throw new Error(`${label}: SKILL.md has no valid YAML frontmatter`);
  let parsed: unknown;
  try {
    parsed = Bun.YAML.parse(match[1]);
  } catch (error) {
    throw new Error(`${label}: invalid SKILL.md YAML: ${error}`);
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error(`${label}: SKILL.md frontmatter must be a mapping`);
  }
  return parsed as Record<string, unknown>;
}

/** Skill units validate as skills; a tree unit has no SKILL.md and is checked only for the symlink and entry-type rules collectFiles already enforces. */
export function validateUnit(
  config: DeployConfig,
  files: Map<string, DesiredFile>,
  unit: Unit,
  label: string,
): void {
  if (!unit.isSkill) return;
  const skillMd = files.get("SKILL.md");
  if (!skillMd) throw new Error(`${label}: SKILL.md missing`);
  const frontmatter = extractFrontmatter(readFileSync(skillMd.absolutePath, "utf8"), label);
  if (frontmatter.name !== unit.key) {
    throw new Error(`${label}: SKILL.md name must be '${unit.key}'`);
  }
  if (typeof frontmatter.description !== "string" || !frontmatter.description.trim()) {
    throw new Error(`${label}: SKILL.md description must be a non-empty string`);
  }
  if (frontmatter.description.length > 1024) {
    throw new Error(`${label}: SKILL.md description exceeds 1024 characters`);
  }
  config.validateAdapterFiles?.(files, label);
}

function materializeStage(config: DeployConfig, pack: string, unit: Unit): { stage: string; digest: string } {
  const destinationRoot = dirname(unit.destination);
  mkdirSync(destinationRoot, { recursive: true });
  // Keep staging outside the discovery root. A harness scans its skill directory
  // recursively, so even a short-lived half-built tree must not appear there.
  const stage = mkdtempSync(join(dirname(destinationRoot), `.axon-${config.adapter}-stage-${basename(unit.destination)}-`));
  try {
    const files = desiredFiles(config, pack, unit);
    validateUnit(config, files, unit, `${pack}/${unit.key}`);
    for (const file of files.values()) {
      const dest = join(stage, ...file.relativePath.split("/"));
      mkdirSync(dirname(dest), { recursive: true });
      // A transformed file is written from its bytes; everything else is copied, so
      // the common path keeps mtime and hard-link behaviour it always had.
      if (file.content) writeFileSync(dest, file.content);
      else copyFileSync(file.absolutePath, dest);
      chmodSync(dest, file.mode);
    }
    return { stage, digest: digestTree(config, stage) };
  } catch (error) {
    rmSync(stage, { recursive: true, force: true });
    throw error;
  }
}

/**
 * The Pack that already occupies this unit's destination, if any.
 *
 * Ownership is a claim on a PATH, not on a name. For a skill the two coincide —
 * its ledger key IS its destination basename — but a tree unit's key is the
 * source directory (`agents/`) while its destination carries the pack name. Two
 * Packs each carrying an agents/ tree therefore share a key and collide in no
 * other way, and comparing keys made the second one permanently unclaimable:
 * `agents/: already owned by Pack '<the first one>'`, with nothing wrong at the
 * destination. Compare the resolved destinations instead.
 */
function ownerOf(config: DeployConfig, state: DeploymentState, unit: Unit): string | null {
  for (const [pack, record] of Object.entries(state.packs)) {
    for (const unitKey of Object.keys(record.skills)) {
      if (recordedDestination(config, pack, unitKey) === unit.destination) return pack;
    }
  }
  return null;
}

function replaceAtomically(config: DeployConfig, stage: string, destination: string, singleFile = false): void {
  if (singleFile) {
    // The stage is a directory holding this one file, and the destination is a file,
    // so renaming the stage over it would fail outright (ENOTDIR). Rename the file
    // itself: on POSIX that replaces the destination in one step, which is why no
    // rollback copy is needed here and one is needed for a directory.
    renameSync(join(stage, basename(destination)), destination);
    rmSync(stage, { recursive: true, force: true });
    return;
  }
  if (!existsSync(destination)) {
    renameSync(stage, destination);
    return;
  }
  // The rollback copy also stays outside the discovery root so the harness never
  // sees a duplicate during the short rename window.
  const backup = join(
    dirname(dirname(destination)),
    `.axon-${config.adapter}-backup-${basename(destination)}-${process.pid}`,
  );
  renameSync(destination, backup);
  try {
    renameSync(stage, destination);
  } catch (error) {
    renameSync(backup, destination);
    throw error;
  }
  rmSync(backup, { recursive: true, force: true });
}

function recordUnit(config: DeployConfig, state: DeploymentState, pack: string, unit: Unit, digest: string): void {
  state.packs[pack] ??= { skills: {} };
  state.packs[pack].skills[unit.key] = {
    source: relative(config.axonRoot, unit.sourceRoot),
    desiredDigest: digest,
    installedDigest: digest,
    digestPolicy: DIGEST_POLICY,
    deployedAt: new Date().toISOString(),
  };
}

function installOne(
  config: DeployConfig,
  state: DeploymentState,
  pack: string,
  unit: Unit,
  mode: "deploy" | "sync",
): string {
  const destination = unit.destination;
  const owner = ownerOf(config, state, unit);
  const existingRecord = state.packs[pack]?.skills[unit.key];
  if (owner && owner !== pack) throw new Error(`${unit.key}: already owned by Pack '${owner}'`);
  if (existsSync(destination) && !existingRecord) {
    throw new Error(`${unit.key}: ${destination} exists and is not owned by this Axon deployment`);
  }
  if (mode === "deploy" && existingRecord && existsSync(destination)) {
    adoptDigestPolicyIfSafe(config, existingRecord, destination, unit.key);
    const actual = digestDestination(config, destination);
    const wanted = digestFiles(desiredFiles(config, pack, unit));
    return wanted === actual ? `= ${unit.key} (already current)` : `= ${unit.key} (deployed; run sync to update)`;
  }

  if (existingRecord && existsSync(destination)) {
    try {
      adoptDigestPolicyIfSafe(config, existingRecord, destination, unit.key);
    } catch (error) {
      throw new Error(`${(error as Error).message}; refusing to overwrite`);
    }
  }

  const { stage, digest } = materializeStage(config, pack, unit);
  try {
    if (existsSync(destination) && digestDestination(config, destination) === digest) {
      rmSync(stage, { recursive: true, force: true });
      recordUnit(config, state, pack, unit, digest);
      return `= ${unit.key} (already current)`;
    }
    replaceAtomically(config, stage, destination, Boolean(unit.onlyFile));
  } catch (error) {
    if (existsSync(stage)) rmSync(stage, { recursive: true, force: true });
    throw error;
  }
  recordUnit(config, state, pack, unit, digest);
  return `✓ ${unit.key} ${mode === "deploy" ? "deployed" : "synced"}`;
}

export function deployPack(config: DeployConfig, pack: string, skillSubset?: Set<string>): string[] {
  return withStateLock(config, () => {
    // A profile subset deploys ONLY the named skills; tree units (agents/ and
    // similar) are excluded with it, so a subset is an exact load statement.
    const units = skillSubset
      ? packUnits(config, pack).filter((u) => !u.isSkill || skillSubset.has(u.key))
      : packUnits(config, pack);
    const state = readState(config);
    // Validate every source and collision before the first write so a bad unit
    // cannot leave a normally-failing Pack only partially deployed.
    for (const unit of units) {
      const files = desiredFiles(config, pack, unit);
      validateUnit(config, files, unit, `${pack}/${unit.key}`);
      const record = state.packs[pack]?.skills[unit.key];
      const owner = ownerOf(config, state, unit);
      if (owner && owner !== pack) throw new Error(`${unit.key}: already owned by Pack '${owner}'`);
      if (existsSync(unit.destination) && !record) {
        throw new Error(`${unit.key}: ${unit.destination} exists and is not owned by this Axon deployment`);
      }
      if (record && existsSync(unit.destination)) {
        try {
          adoptDigestPolicyIfSafe(config, record, unit.destination, unit.key);
        } catch (error) {
          throw new Error(`${(error as Error).message}; refusing to redeploy`);
        }
      }
    }
    const messages: string[] = [];
    const failures: string[] = [];
    for (const unit of units) {
      try {
        messages.push(installOne(config, state, pack, unit, "deploy"));
        writeState(config, state);
      } catch (error) {
        failures.push((error as Error).message);
      }
    }
    if (failures.length) throw new Error(failures.join("\n"));
    return messages;
  });
}

/**
 * Take ownership of destinations that already hold exactly what this Pack would
 * deploy, without writing anything.
 *
 * The migration case it exists for: an adapter that used to deploy by symlink has
 * real directories sitting in place that no ledger knows about, and a plain
 * deploy correctly refuses them as collisions. Adoption is safe ONLY because it
 * demands a digest match — an adopted unit is byte-identical to its source, so
 * recording it asserts nothing that is not already true on disk. Anything that
 * differs is left alone and reported, because a difference is either a hand edit
 * or a stale deployment and both need a human, not a ledger entry.
 */
export function adoptPack(config: DeployConfig, pack: string): string[] {
  return withStateLock(config, () => {
    const units = packUnits(config, pack);
    const state = readState(config);
    const messages: string[] = [];
    const failures: string[] = [];
    for (const unit of units) {
      const owner = ownerOf(config, state, unit);
      if (owner === pack) { messages.push(`= ${unit.key} (already owned)`); continue; }
      if (owner) { failures.push(`${unit.key}: already owned by Pack '${owner}'`); continue; }
      if (!existsSync(unit.destination)) { messages.push(`= ${unit.key} (not deployed; nothing to adopt)`); continue; }
      const files = desiredFiles(config, pack, unit);
      validateUnit(config, files, unit, `${pack}/${unit.key}`);
      const wanted = digestFiles(files);
      const actual = digestDestination(config, unit.destination);
      if (wanted !== actual) {
        failures.push(`${unit.key}: ${unit.destination} differs from the Pack source; refusing to adopt`);
        continue;
      }
      recordUnit(config, state, pack, unit, wanted);
      writeState(config, state);
      messages.push(`✓ ${unit.key} adopted`);
    }
    if (failures.length) throw new Error(failures.join("\n"));
    return messages;
  });
}

/**
 * Re-record a unit whose destination is byte-identical to its source.
 *
 * The case: an operator edited a deployed copy, the edit was worth keeping, and
 * the source has just been updated FROM the destination. Source and destination
 * now agree, but the ledger still holds the old digest and reports drift that no
 * longer exists. `deploy` would copy (pointlessly) and `sync` refuses outright,
 * because it sees a destination that differs from the digest it recorded.
 *
 * Refuses unless the two really are identical, so this can never be used to make
 * a ledger claim something the disk does not support.
 */
export function reconcileUnit(config: DeployConfig, pack: string, unit: Unit): string {
  return withStateLock(config, () => {
    const state = readState(config);
    const record = state.packs[pack]?.skills[unit.key];
    if (!record) throw new Error(`${unit.key}: not owned by Pack '${pack}'`);
    if (!existsSync(unit.destination)) throw new Error(`${unit.key}: ${unit.destination} does not exist`);
    const files = desiredFiles(config, pack, unit);
    // Validate before recording. An edit accepted from a destination can have
    // broken the frontmatter, and a ledger that records a broken skill as current
    // is worse than one that reports drift.
    validateUnit(config, files, unit, `${pack}/${unit.key}`);
    const wanted = digestFiles(files);
    const installed = digestDestination(config, unit.destination);
    if (wanted !== installed) {
      throw new Error(`${unit.key}: destination still differs from the Pack source; refusing to re-record`);
    }
    if (record.installedDigest === wanted && record.desiredDigest === wanted) return `= ${unit.key} (already recorded)`;
    recordUnit(config, state, pack, unit, wanted);
    writeState(config, state);
    return `✓ ${unit.key} re-recorded`;
  });
}

/**
 * The destination a recorded unit occupies. Rebuilt from the key rather than
 * stored, so a ledger written before the tree convention existed still resolves.
 */
function recordedDestination(config: DeployConfig, pack: string, unitKey: string): string {
  const tree = config.treeConvention;
  if (tree && unitKey === treeKey(tree.sourceDir)) return join(tree.destinationRoot, pack);
  const flat = config.flatFileConvention;
  if (flat && unitKey.startsWith(`${flat.sourceDir}/`)) {
    return join(flat.destinationRoot, unitKey.slice(flat.sourceDir.length + 1));
  }
  return join(config.destination, unitKey);
}

function removeOwnedUnit(
  config: DeployConfig,
  state: DeploymentState,
  pack: string,
  unitKey: string,
): string {
  const record = state.packs[pack]?.skills[unitKey];
  if (!record) throw new Error(`${unitKey}: not owned by Pack '${pack}'`);
  const destination = recordedDestination(config, pack, unitKey);
  if (existsSync(destination)) {
    try {
      adoptDigestPolicyIfSafe(config, record, destination, unitKey);
    } catch (error) {
      throw new Error(`${(error as Error).message}; refusing to remove`);
    }
    rmSync(destination, { recursive: true });
  }
  delete state.packs[pack].skills[unitKey];
  if (Object.keys(state.packs[pack].skills).length === 0) delete state.packs[pack];
  return `✓ ${unitKey} removed`;
}

export function syncPack(config: DeployConfig, pack: string): string[] {
  return withStateLock(config, () => {
    const units = packUnits(config, pack);
    const state = readState(config);
    if (!state.packs[pack]) throw new Error(`${pack}: not deployed; deploy it first`);
    const desired = new Set(units.map((unit) => unit.key));
    const messages: string[] = [];
    const failures: string[] = [];

    // Preflight both stale removals and desired updates before mutating either.
    for (const [unitKey, record] of Object.entries(state.packs[pack].skills)) {
      const destination = recordedDestination(config, pack, unitKey);
      if (existsSync(destination)) {
        try {
          adoptDigestPolicyIfSafe(config, record, destination, unitKey);
        } catch (error) {
          throw new Error(`${(error as Error).message}; refusing to sync`);
        }
      }
    }
    for (const unit of units) {
      const files = desiredFiles(config, pack, unit);
      validateUnit(config, files, unit, `${pack}/${unit.key}`);
      const owner = ownerOf(config, state, unit);
      if (owner && owner !== pack) throw new Error(`${unit.key}: already owned by Pack '${owner}'`);
      if (existsSync(unit.destination) && !state.packs[pack].skills[unit.key]) {
        throw new Error(`${unit.key}: ${unit.destination} exists and is not owned by this Axon deployment`);
      }
    }

    for (const stale of Object.keys(state.packs[pack].skills).filter((key) => !desired.has(key))) {
      try {
        messages.push(removeOwnedUnit(config, state, pack, stale));
        writeState(config, state);
      } catch (error) {
        failures.push((error as Error).message);
      }
    }
    for (const unit of units) {
      try {
        messages.push(installOne(config, state, pack, unit, "sync"));
        writeState(config, state);
      } catch (error) {
        failures.push((error as Error).message);
      }
    }
    if (failures.length) throw new Error(failures.join("\n"));
    return messages;
  });
}

export function removePack(config: DeployConfig, pack: string): string[] {
  return withStateLock(config, () => {
    const state = readState(config);
    if (!state.packs[pack]) throw new Error(`${pack}: not deployed`);
    for (const [unitKey, record] of Object.entries(state.packs[pack].skills)) {
      const destination = recordedDestination(config, pack, unitKey);
      if (existsSync(destination)) {
        try {
          adoptDigestPolicyIfSafe(config, record, destination, unitKey);
        } catch (error) {
          throw new Error(`${(error as Error).message}; refusing to remove`);
        }
      }
    }
    const messages: string[] = [];
    const failures: string[] = [];
    for (const unitKey of Object.keys(state.packs[pack].skills)) {
      try {
        messages.push(removeOwnedUnit(config, state, pack, unitKey));
        writeState(config, state);
      } catch (error) {
        failures.push((error as Error).message);
      }
    }
    if (failures.length) throw new Error(failures.join("\n"));
    return messages;
  });
}

export function migrateGeneratedArtifacts(
  config: DeployConfig,
  pack: string,
  acceptCurrent: boolean,
): string[] {
  if (!acceptCurrent) {
    throw new Error(
      "migration requires --accept-current after reviewing non-generated destination files",
    );
  }
  const state = readState(config);
  if (!state.packs[pack]) throw new Error(`${pack}: not deployed`);
  const plans = new Map<string, GeneratedArtifacts>();
  for (const [unitKey, record] of Object.entries(state.packs[pack].skills)) {
    if (record.digestPolicy === DIGEST_POLICY) continue;
    const destination = recordedDestination(config, pack, unitKey);
    if (!existsSync(destination)) throw new Error(`${unitKey}: owned destination is missing`);
    plans.set(unitKey, knownGeneratedArtifacts(destination, `${pack}/${unitKey}`));
  }

  const messages: string[] = [];
  for (const [unitKey, artifacts] of plans) {
    const destination = recordedDestination(config, pack, unitKey);
    for (const file of artifacts.files) rmSync(file);
    for (const directory of artifacts.directories.sort((a, b) => b.length - a.length)) {
      if (readdirSync(directory).length === 0) rmdirSync(directory);
    }
    const record = state.packs[pack].skills[unitKey];
    record.installedDigest = digestDestination(config, destination);
    record.digestPolicy = DIGEST_POLICY;
    messages.push(`✓ ${unitKey} migrated (${artifacts.files.length} generated artifact(s) removed)`);
  }
  writeState(config, state);
  if (messages.length === 0) messages.push(`= ${pack} (digest policy already current)`);
  return messages;
}

// ── Profiles ────────────────────────────────────────────────────────

export type Profile = {
  name: string;
  description: string;
  packs: string[];
  /**
   * Packs a `*` profile must NOT deploy. Only meaningful with `packs = ["*"]`.
   *
   * This is where "what to deploy, and what not" belongs — the operator's decision,
   * in the file the operator owns, naming only Packs that actually exist. It replaces
   * the `# pi: REFUSED` line a Pack used to carry in its own `pack.toml` (retired
   * 2026-09-17): that was a harness-specific veto inside a manifest the schema keeps
   * harness-neutral, and it made `full` silently mean "all packs except …".
   *
   * An `except` list also keeps `full` portable. Writing the same set as an explicit
   * `packs = ["…"]` would have to name the private overlay's Packs, which do not exist
   * on another machine, and would silently drop every Pack added later.
   */
  except?: string[];
  /**
   * Optional per-Pack skill subset. A Pack with no entry loads all its skills;
   * a Pack with an entry loads ONLY the named skills (tree units like agents/
   * are excluded with a subset). Enables a profile like `home` to stop dragging
   * ten ha-* skills into every session when the measurements say most are never
   * used (2026-09-11).
   */
  skills?: Record<string, string[]>;
};

export function readProfiles(config: DeployConfig): Profile[] {
  const path = join(config.axonRoot, "profiles.toml");
  if (!existsSync(path)) return [];
  const parsed = Bun.TOML.parse(readFileSync(path, "utf8")) as Record<string, unknown>;
  const profiles = parsed.profile;
  if (!Array.isArray(profiles)) return [];
  return profiles as Profile[];
}

export function resolveProfilePacks(config: DeployConfig, profile: Profile): string[] {
  const except = profile.except ?? [];
  if (profile.packs.length === 1 && profile.packs[0] === "*") {
    const available = new Set(availablePacks(config));
    for (const pack of except) {
      // Named-but-absent is an error, not a no-op: an exclusion list that silently
      // does nothing because of a typo reads as "deployed" for a Pack that is not.
      if (!available.has(pack)) {
        throw new Error(`profile '${profile.name}': except names unknown pack '${pack}'`);
      }
    }
    const excluded = new Set(except);
    return availablePacks(config).filter((pack) => !excluded.has(pack));
  }
  if (except.length) {
    throw new Error(
      `profile '${profile.name}': except is only meaningful with packs = ["*"]; ` +
        `list the Packs this profile wants instead of excluding from a list it does not have`,
    );
  }
  const allPacks = new Set(availablePacks(config, true));
  for (const pack of profile.packs) {
    if (!allPacks.has(pack)) {
      throw new Error(`profile '${profile.name}': unknown pack '${pack}'`);
    }
    const owner = packDeployer(config, pack);
    if (owner) {
      throw new Error(
        `profile '${profile.name}': pack '${pack}' is deployed by ${owner}; remove it from the profile`,
      );
    }
  }
  return profile.packs;
}

/**
 * The per-Pack skill subset a profile asks for: `null` means "all skills of that
 * Pack". Validates every named skill against its Pack before anything moves.
 */
export function resolveProfileSkills(config: DeployConfig, profile: Profile): Map<string, Set<string> | null> {
  const out = new Map<string, Set<string> | null>();
  if (!profile.skills) return out;
  const profilePacks = new Set(resolveProfilePacks(config, profile));
  for (const [pack, skills] of Object.entries(profile.skills)) {
    if (!profilePacks.has(pack)) {
      throw new Error(`profile '${profile.name}': skills names pack '${pack}', which is not in packs`);
    }
    const unitNames = new Set(packUnits(config, pack).filter((u) => u.isSkill).map((u) => u.key));
    for (const skill of skills) {
      if (!unitNames.has(skill)) throw new Error(`profile '${profile.name}': pack '${pack}' has no skill '${skill}'`);
    }
    out.set(pack, new Set(skills));
  }
  return out;
}

export function activateProfile(config: DeployConfig, profile: Profile): string[] {
  return withStateLock(config, () => {
    const targetPackNames = new Set(resolveProfilePacks(config, profile));
    const skillSubsets = resolveProfileSkills(config, profile);
    const state = readState(config);
    const messages: string[] = [];

    messages.push(`Activating profile '${profile.name}' — ${profile.description}`);

    const currentPacks = Object.keys(state.packs).sort();
    const targetPacks = [...targetPackNames].sort();

    const toRemove = currentPacks.filter((p) => !targetPackNames.has(p));
    const toDeploy = targetPacks.filter((p) => {
      if (!state.packs[p]) return true;
      // Re-deploy if any owned destination is missing from disk
      return Object.keys(state.packs[p].skills).some(
        (unitKey) => !existsSync(recordedDestination(config, p, unitKey)),
      );
    });

    if (toRemove.length === 0 && toDeploy.length === 0) {
      messages.push("  → already current");
      return messages;
    }

    if (toRemove.length > 0) {
      messages.push("", `Removing ${toRemove.length} pack(s) not in profile:`);
      for (const pack of toRemove) {
        try {
          messages.push(...removePack(config, pack).map((l) => `  ${l}`));
        } catch (error) {
          messages.push(`  ✗ ${pack}: ${(error as Error).message}`);
        }
      }
    }

    if (toDeploy.length > 0) {
      messages.push("", `Deploying ${toDeploy.length} pack(s):`);
      for (const pack of toDeploy) {
        try {
          messages.push(...deployPack(config, pack, skillSubsets.get(pack) ?? undefined).map((l) => `  ${l}`));
        } catch (error) {
          messages.push(`  ✗ ${pack}: ${(error as Error).message}`);
        }
      }
    }

    return messages;
  });
}

export function profileActivePacks(config: DeployConfig, profile: Profile): string[] {
  const target = new Set(resolveProfilePacks(config, profile));
  const state = readState(config);
  return Object.keys(state.packs).filter((p) => target.has(p)).sort();
}

// ── Discovery ───────────────────────────────────────────────────────

// A pack whose manifest names a `deployer` is owned by that tool alone, so generic
// harness deployment never competes with its dedicated lifecycle.
export function packDeployer(config: DeployConfig, pack: string): string | null {
  const manifest = join(config.axonRoot, "Packs", pack, "pack.toml");
  if (!existsSync(manifest)) return null;
  const parsed = Bun.TOML.parse(readFileSync(manifest, "utf8")) as Record<string, unknown>;
  const deployer = parsed.deployer;
  return typeof deployer === "string" && deployer.length > 0 ? deployer : null;
}

// `includeDedicated` exists so name validation can still see a dedicated pack and
// say who owns it, instead of reporting a pack that plainly exists as unknown.
export function availablePacks(config: DeployConfig, includeDedicated = false): string[] {
  const matches = new Map<string, string[]>();
  for (const root of packRoots(config)) {
    if (!existsSync(root)) continue;
    for (const entry of readdirSync(root, { withFileTypes: true })) {
      if (!entry.isDirectory() || !existsSync(join(root, entry.name, "pack.toml"))) continue;
      matches.set(entry.name, [...(matches.get(entry.name) ?? []), root]);
    }
  }
  for (const [pack, roots] of matches) {
    if (roots.length > 1) throw new Error(`pack '${pack}' is declared in more than one Pack root: ${roots.join(", ")}`);
  }
  return [...matches.keys()]
    .filter((pack) => includeDedicated || packDeployer(config, pack) === null)
    .sort();
}

export function getStatuses(config: DeployConfig, selectedPack?: string): StatusRow[] {
  const state = readState(config);
  const packs = selectedPack
    ? [selectedPack]
    : [...new Set([...availablePacks(config), ...Object.keys(state.packs)])].sort();
  const rows: StatusRow[] = [];
  for (const pack of packs) {
    let units: Unit[];
    try {
      units = packUnits(config, pack);
    } catch (error) {
      rows.push({ pack, skill: "(manifest)", status: "invalid", detail: (error as Error).message });
      continue;
    }
    const seen = new Set(units.map((unit) => unit.key));
    for (const unit of units) {
      const destination = unit.destination;
      const record = state.packs[pack]?.skills[unit.key];
      let wanted: string;
      try {
        const files = desiredFiles(config, pack, unit);
        validateUnit(config, files, unit, `${pack}/${unit.key}`);
        wanted = digestFiles(files);
      } catch (error) {
        rows.push({ pack, skill: unit.key, status: "invalid", detail: (error as Error).message });
        continue;
      }
      if (!record) {
        rows.push({
          pack,
          skill: unit.key,
          status: existsSync(destination) ? "collision" : "not-deployed",
        });
        continue;
      }
      if (!existsSync(destination)) {
        rows.push({ pack, skill: unit.key, status: "missing" });
        continue;
      }
      try {
        const actual = digestDestination(config, destination);
        if (record.digestPolicy !== DIGEST_POLICY) {
          if (legacyDigestDestination(config, destination) !== record.installedDigest) {
            rows.push({
              pack,
              skill: unit.key,
              status: "migration-required",
              detail: "legacy digest differs; review before adopting generated-artifact exclusions",
            });
            continue;
          }
        } else if (actual !== record.installedDigest) {
          rows.push({ pack, skill: unit.key, status: "drifted" });
          continue;
        }
        rows.push({ pack, skill: unit.key, status: wanted === actual ? "current" : "outdated" });
      } catch (error) {
        rows.push({ pack, skill: unit.key, status: "invalid", detail: (error as Error).message });
      }
    }
    for (const stale of Object.keys(state.packs[pack]?.skills ?? {}).filter((key) => !seen.has(key))) {
      rows.push({ pack, skill: stale, status: "outdated", detail: "removed from pack manifest" });
    }
  }
  return rows;
}

export function printStatuses(rows: StatusRow[]): void {
  let lastPack = "";
  for (const row of rows) {
    if (row.pack !== lastPack) {
      if (lastPack) console.log();
      console.log(row.pack);
      lastPack = row.pack;
    }
    console.log(`  ${row.skill.padEnd(24)} [${row.status}]${row.detail ? ` ${row.detail}` : ""}`);
  }
}
