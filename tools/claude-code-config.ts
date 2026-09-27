// tools/claude-code-config.ts — deploy Axon's Claude Code harness config from
// version-controlled sources of truth. Two layers, two commands:
//
//   (default)   USER layer  — tools/templates/claude-code/settings.base.json
//               deep-merged into ~/.claude/settings.json, existing keys win.
//   --managed   MANAGED layer — tools/templates/claude-code/managed-settings.json
//               plus this machine's overlay fragment (see below), deployed to
//               /etc/claude-code/managed-settings.json (full replace).
//
// Why two layers with different merge rules: Claude Code reads user settings and an
// enterprise "managed" policy at /etc/claude-code/managed-settings.json, and the managed
// policy is the HIGHEST-precedence layer — it overrides user and project settings and
// cannot be relaxed by an untrusted repo's .claude/settings.json or by a prompt-injection.
// So the two layers carry different things:
//   - USER = personal defaults you may freely override → merge, existing-wins, idempotent.
//   - MANAGED = the security floor that must hold regardless of mode → Axon owns the whole
//     file, so it is a full replace, not a merge. Its deny/ask/sandbox rules stay in force
//     even in auto mode (auto-accept still obeys deny and still prompts on ask).
// Deliberately, the managed policy does NOT pin `permissions.defaultMode` and does NOT set
// `disableAutoMode` — permission MODE is a personal workflow choice that lives in the user
// layer (settings.base.json ships defaultMode:"auto"); pinning it in the managed layer only
// forbids the mode you actually want while adding no security (the deny/ask rules are the
// real boundary). `disableBypassPermissionsMode` is KEPT — full-bypass is a real hole.
//
// The managed policy is deliberately extensible from the overlay, because some deny rules
// describe one deployment rather than Axon. The public base contains only generic protection;
// real protected paths and deployment-specific credential identifiers come from
//
//   <overlay>/config/claude-code/managed-settings.fragment.json
//
// merged in by mergeFragment() below. The merge is deliberately ADDITIVE-ONLY: a fragment
// may append elements to arrays the base already declares, and nothing else. It cannot
// introduce a key, overwrite a scalar, or reorder anything — so no overlay, and no prompt
// injection that manages to write one, can relax `sandbox.enabled`, flip
// `allowManagedPermissionRulesOnly`, or delete a deny rule. An overlay can only ever make
// this policy stricter. Violations are a hard error naming the dotted path, never a
// silent skip: a security fragment that half-applied would be worse than one that failed.
//
// The managed target lives under /etc and needs root. This tool never silently clobbers it:
// it reports drift and, when it cannot write (the normal case — not root, or a sandbox that
// denies /etc writes), prints the exact privileged command for you to run yourself. Deploying
// a security policy to /etc is an explicit, user-run action by design.
//
// TS-via-bun, not bash: JSON deep-merge/compare in bash 3.2 (README.md#portable-shell) isn't worth
// hand-rolling; bun parses/emits JSON natively with no npm dependency (bun is already in
// upstreams.toml), the same call already made for tools/doctor — see that file's header.
// Invoke via the tools/claude-code-config launcher (exec bun run), not directly.
//
//   tools/claude-code-config              merge the user baseline into ~/.claude/settings.json
//   tools/claude-code-config --managed    deploy/check the managed policy at /etc/claude-code
//   tools/claude-code-config [...] --dry-run   report what would change, write nothing
//   tools/claude-code-config -h           this help
//
// Exit 0 = applied / already current / drift reported with a command to fix it,
// 1 = usage or missing-template error, 2 = existing user settings.json is unparseable
// (never clobbered), 3 = the overlay fragment is invalid (nothing deployed).

import {
  chmodSync,
  closeSync,
  existsSync,
  fsyncSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  renameSync,
  statSync,
  unlinkSync,
  writeFileSync,
  writeSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { overlayRoot } from "./lib/overlay.ts";

// Both layers this tool writes are policy files a half-write destroys rather than degrades:
// a truncated managed-settings.json is unparseable, and an unparseable security floor is an
// ABSENT security floor, not a weaker one. So neither layer gets a bare writeFileSync.
//
// Same directory as the target on purpose: rename() is only atomic within one filesystem,
// and $TMPDIR is routinely on a different one. Mode is set at open() rather than chmod'd
// after the rename, because the post-rename version publishes a world-readable file first
// and tightens it a moment later. On failure the temp file is removed and the previous
// target is left exactly as it was — the caller sees the error, the reader sees the old
// policy, and nobody ever sees a fragment.
export function writeFileAtomic(target: string, contents: string, mode: number): void {
  const dir = dirname(target);
  mkdirSync(dir, { recursive: true });
  const tmp = join(dir, `.${basename(target)}.axon-${process.pid}.tmp`);
  let fd: number | null = null;
  try {
    // "wx" — never follow a symlink into, or reuse, an existing path at this name.
    fd = openSync(tmp, "wx", mode);
    const buf = Buffer.from(contents, "utf8");
    for (let off = 0; off < buf.length; ) off += writeSync(fd, buf, off, buf.length - off);
    fsyncSync(fd); // content durable BEFORE the rename publishes it
    closeSync(fd);
    fd = null;
    renameSync(tmp, target);
  } catch (e) {
    if (fd !== null) try { closeSync(fd); } catch {}
    try { unlinkSync(tmp); } catch {}
    throw e;
  }
  // Best-effort: makes the rename itself survive a power loss. Not every platform lets you
  // open a directory for fsync, and failing here would undo a write that already succeeded.
  try {
    const dfd = openSync(dir, "r");
    try { fsyncSync(dfd); } finally { closeSync(dfd); }
  } catch {}
}

const HELP = `tools/claude-code-config — deploy Axon's Claude Code harness config.

  tools/claude-code-config              merge the user baseline into ~/.claude/settings.json
  tools/claude-code-config --managed    deploy/check the managed policy at /etc/claude-code/managed-settings.json
  tools/claude-code-config --dry-run    report what would change, write nothing (combine with --managed)
  tools/claude-code-config -h           this help

USER layer merges, existing keys win — never clobbers a personal override, safe to re-run.
MANAGED layer is a full replace of an Axon-owned security policy; it needs root, so when this
tool can't write /etc it prints the exact sudo command for you to run. This machine's own
additions to that policy come from <overlay>/config/claude-code/managed-settings.fragment.json
and may only APPEND to arrays the base template already declares — never relax it.
`;

// Arg reads are pure; every exit lives in main(), which only runs as a program. That
// split is what lets claude-code-config.test.ts import mergeFragment — the additive-only
// rule is a security boundary, so it gets a test, and a module that dispatched on import
// could not have one. Same import.meta.main shape as tools/doctor.ts.
const args = process.argv.slice(2);
const DRY_RUN = args.includes("--dry-run") || args.includes("-n");
const MANAGED = args.includes("--managed");

const SJEL_ROOT = resolve(import.meta.dir, "..");
const HOME = process.env.HOME ?? "";
const TEMPLATES = join(SJEL_ROOT, "tools", "templates", "claude-code");

function expandHome(p: string): string {
  return p.startsWith("~") ? HOME + p.slice(1) : p;
}

type Json = Record<string, unknown>;

function managedOut(message: string): void {
  writeSync(1, message + "\n");
}

function managedErr(message: string): void {
  writeSync(2, message + "\n");
}

function isPlainObject(v: unknown): v is Json {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function readJson(path: string, label: string): Json {
  try {
    return JSON.parse(readFileSync(path, "utf8")) as Json;
  } catch (e) {
    console.error(`claude-code-config: ${label} is not valid JSON: ${path}\n  ${e}`);
    process.exit(1);
  }
}

// ---- USER layer: deep-merge with existing-wins ----------------------------------------

// Merge `baseline` into `existing` recording the dotted path of every key the baseline
// actually contributes. Arrays/scalars are leaves: an existing key is left untouched.
function mergeDefaults(existing: Json, baseline: Json, prefix: string, added: string[]): Json {
  const out: Json = { ...existing };
  for (const key of Object.keys(baseline)) {
    const path = prefix ? `${prefix}.${key}` : key;
    const bv = baseline[key];
    if (!(key in out)) {
      out[key] = bv;
      added.push(path);
    } else if (isPlainObject(out[key]) && isPlainObject(bv)) {
      out[key] = mergeDefaults(out[key] as Json, bv, path, added);
    }
  }
  return out;
}

function deployUserLayer(): never {
  const baselinePath = join(TEMPLATES, "settings.base.json");
  if (!existsSync(baselinePath)) {
    console.error(`claude-code-config: baseline not found: ${baselinePath}`);
    process.exit(1);
  }
  const baseline = readJson(baselinePath, "baseline");

  // ~/.claude is the Claude Code config dir; CLAUDE_CONFIG_DIR overrides it (the harness's
  // own env var). Mirrors packs.sh honouring CLAUDE_SKILLS_DIR — README.md#dynamic-paths-and-current-facts, derived & overridable.
  const configDir = process.env.CLAUDE_CONFIG_DIR
    ? expandHome(process.env.CLAUDE_CONFIG_DIR)
    : join(HOME, ".claude");
  const target = join(configDir, "settings.json");

  let existing: Json = {};
  if (existsSync(target)) {
    try {
      existing = JSON.parse(readFileSync(target, "utf8")) as Json;
    } catch (e) {
      console.error(`claude-code-config: ${target} exists but is not valid JSON — refusing to overwrite.`);
      console.error(`  ${e}`);
      process.exit(2);
    }
  }

  const added: string[] = [];
  const merged = mergeDefaults(existing, baseline, "", added);
  if (added.length === 0) {
    console.log(`claude-code-config: ${target} already applies Axon's baseline — no changes.`);
    process.exit(0);
  }
  if (DRY_RUN) {
    console.log(`claude-code-config: [dry-run] would add to ${target}:`);
    for (const p of added) console.log(`  + ${p}`);
    process.exit(0);
  }
  // Keep whatever mode the user's own settings.json already carries — this command merges
  // into a file Claude Code created, and silently tightening or loosening it is not what
  // "add Axon's baseline keys" means. A file we create ourselves starts at 0600.
  const userMode = existsSync(target) ? statSync(target).mode & 0o777 : 0o600;
  writeFileAtomic(target, JSON.stringify(merged, null, 2) + "\n", userMode);
  console.log(`claude-code-config: updated ${target} — added:`);
  for (const p of added) console.log(`  + ${p}`);
  process.exit(0);
}

// ---- MANAGED layer: full replace of the Axon-owned /etc policy -------------------------

const RESTRICTIVE_ARRAY_PATHS = new Set([
  "permissions.deny",
  "sandbox.filesystem.denyRead",
  "sandbox.filesystem.denyWrite",
  "sandbox.credentials.files",
  "sandbox.credentials.envVars",
]);

function validateRestrictiveAddition(path: string, value: unknown): void {
  if (["permissions.deny", "sandbox.filesystem.denyRead", "sandbox.filesystem.denyWrite"].includes(path)) {
    if (typeof value !== "string" || !value.trim()) {
      throw new Error(`${path}: additions must be non-empty strings`);
    }
    return;
  }

  if (!isPlainObject(value) || value.mode !== "deny") {
    throw new Error(`${path}: credential additions must be objects with mode \"deny\"`);
  }
  const identityKey = path.endsWith(".envVars") ? "name" : "path";
  if (typeof value[identityKey] !== "string" || !(value[identityKey] as string).trim()) {
    throw new Error(`${path}: credential additions require a non-empty ${identityKey}`);
  }
  const allowedKeys = new Set([identityKey, "mode"]);
  const unexpected = Object.keys(value).filter((key) => !allowedKeys.has(key));
  if (unexpected.length > 0) {
    throw new Error(`${path}: unsupported credential field ${unexpected[0]}`);
  }
}

// Fold `fragment` into `base`, RESTRICTIVE-ONLY. The only legal contribution is "append these
// elements to an array the base already declares"; recursion exists solely to reach nested
// arrays. Every other shape is a hard error, because the alternative — quietly ignoring the
// part of a security fragment that doesn't typecheck — deploys a policy the author believes
// is stricter than it is. Appends dedupe (a fragment re-stating a base rule is a no-op, so
// the two files can be edited independently without drifting into duplicates).
export function mergeFragment(base: Json, fragment: Json, prefix: string, appended: string[]): Json {
  const out: Json = { ...base };
  for (const key of Object.keys(fragment)) {
    // `_`-prefixed keys are documentation, not policy — the same convention
    // scripts/gcp/sbx-catalog.json uses. Dropped before the merge so a fragment can carry
    // its own why (Rule 27) without that prose reaching /etc.
    if (key.startsWith("_")) continue;
    const path = prefix ? `${prefix}.${key}` : key;
    const fv = fragment[key];
    if (!(key in out)) {
      throw new Error(`${path}: fragment may only extend keys the base template declares`);
    }
    const bv = out[key];
    if (Array.isArray(bv) && Array.isArray(fv)) {
      if (!RESTRICTIVE_ARRAY_PATHS.has(path)) {
        throw new Error(`${path}: fragment may not extend this array`);
      }
      const seen = new Set(bv.map((el) => JSON.stringify(el)));
      const additions = fv.filter((el) => !seen.has(JSON.stringify(el)));
      for (const el of additions) validateRestrictiveAddition(path, el);
      out[key] = [...bv, ...additions];
      for (const _ of additions) appended.push(path);
    } else if (isPlainObject(bv) && isPlainObject(fv)) {
      out[key] = mergeFragment(bv, fv, path, appended);
    } else {
      const shape = (v: unknown) => (Array.isArray(v) ? "an array" : isPlainObject(v) ? "an object" : typeof v);
      throw new Error(
        `${path}: fragment may only append to arrays — the base declares ${shape(bv)} here, the fragment gives ${shape(fv)}`,
      );
    }
  }
  return out;
}

// The base policy plus this machine's overlay fragment, if it has one.
function renderManagedPolicy(templatePath: string): { policy: Json; fragmentPath: string | null; appended: string[] } {
  const base = readJson(templatePath, "managed template");
  const overlay = overlayRoot(SJEL_ROOT);
  if (!overlay) return { policy: base, fragmentPath: null, appended: [] };

  if (!existsSync(overlay)) {
    managedErr("claude-code-config: the configured private overlay is not reachable; refusing managed-policy deployment.");
    process.exit(3);
  }

  const fragmentPath = join(overlay, "config", "claude-code", "managed-settings.fragment.json");
  if (!existsSync(fragmentPath)) return { policy: base, fragmentPath: null, appended: [] };

  let fragment: Json;
  try {
    fragment = JSON.parse(readFileSync(fragmentPath, "utf8")) as Json;
  } catch {
    managedErr("claude-code-config: the overlay managed-policy fragment is not valid JSON; refusing deployment.");
    process.exit(3);
  }
  const appended: string[] = [];
  try {
    return { policy: mergeFragment(base, fragment, "", appended), fragmentPath, appended };
  } catch (e: any) {
    managedErr("claude-code-config: the overlay managed-policy fragment is invalid.");
    managedErr(`  ${e?.message ?? e}`);
    managedErr("  A fragment may only append validated entries to approved deny arrays.");
    process.exit(3);
  }
}

export function stageManagedPolicy(desired: string): { stageDir: string; source: string } {
  const stageDir = mkdtempSync(join(tmpdir(), "axon-claude-managed-"));
  chmodSync(stageDir, 0o700);
  const source = join(stageDir, "managed-settings.json");
  writeFileSync(source, desired, { encoding: "utf8", mode: 0o600, flag: "wx" });
  return { stageDir, source };
}

export function managedHandoffInstructions(
  target: string,
  source: string,
  stageDir: string | null,
): string[] {
  const lines = [
    `  Review:  diff "${target}" "${source}"`,
    `  Deploy:  sudo install -m 0644 "${source}" "${target}"`,
  ];
  if (stageDir) lines.push(`  Cleanup: rm -rf "${stageDir}"`);
  return lines;
}

function deployManagedLayer(): never {
  const templatePath = join(TEMPLATES, "managed-settings.json");
  if (!existsSync(templatePath)) {
    console.error(`claude-code-config: managed template not found: ${templatePath}`);
    process.exit(1);
  }
  const { policy, fragmentPath, appended } = renderManagedPolicy(templatePath);
  const desired = JSON.stringify(policy, null, 2) + "\n";

  // Overridable for testing; the real Claude Code enterprise-policy path otherwise.
  const target = process.env.MANAGED_SETTINGS_PATH
    ? expandHome(process.env.MANAGED_SETTINGS_PATH)
    : "/etc/claude-code/managed-settings.json";

  // Compare parsed JSON (formatting-insensitive) so a whitespace-only difference isn't drift.
  let current: Json | null = null;
  if (existsSync(target)) current = readJson(target, "deployed managed policy");
  const inSync = current !== null && JSON.stringify(policy) === JSON.stringify(current);

  if (fragmentPath) {
    const paths = [...new Set(appended)].sort().join(", ");
    managedOut(`claude-code-config: overlay fragment adds ${appended.length} restrictive rule(s)${paths ? ` at ${paths}` : ""}.`);
  } else {
    managedOut("claude-code-config: no overlay managed-policy fragment is configured; using the public base policy.");
  }
  if (inSync) {
    managedOut(`claude-code-config: ${target} already matches Axon's managed policy — no changes.`);
    process.exit(0);
  }

  const verb = current === null ? "install" : "replace";
  if (DRY_RUN) {
    managedOut(`claude-code-config: [dry-run] would ${verb} ${target} from ${templatePath}.`);
    process.exit(0);
  }

  // Try to write directly (works when run as root / when target is user-writable, e.g. tests).
  try {
    // 0644 matches the `sudo install -m 0644` handoff below: every user's harness has to be
    // able to READ the security floor; only root may write it.
    writeFileAtomic(target, desired, 0o644);
    managedOut(`claude-code-config: ${verb === "install" ? "installed" : "replaced"} ${target} from Axon's managed policy.`);
    process.exit(0);
  } catch (e: any) {
    if (e && (e.code === "EACCES" || e.code === "EPERM")) {
      // Expected when not root (or a sandbox denies /etc writes): hand off the privileged step.
      // With a fragment in play the template alone is no longer what should land in /etc, so
      // stage the rendered policy and point the privileged command at THAT. Staging in the
      // temp dir on purpose: it is a build artifact of two tracked inputs, so giving it a
      // permanent home would just create a third file that can go stale.
      let source = templatePath;
      let stageDir: string | null = null;
      if (fragmentPath) {
        const staged = stageManagedPolicy(desired);
        stageDir = staged.stageDir;
        source = staged.source;
      }
      managedOut(`claude-code-config: ${target} is out of sync with Axon's managed policy and needs root to ${verb}.`);
      if (fragmentPath) managedOut(`  Staged rendered policy (base + fragment) at ${source}`);
      for (const line of managedHandoffInstructions(target, source, stageDir)) managedOut(line);
      process.exit(0);
    }
    managedErr(`claude-code-config: failed to write ${target}\n  ${e}`);
    process.exit(1);
  }
}

function main(): never {
  if (args.includes("-h") || args.includes("--help")) {
    console.log(HELP);
    process.exit(0);
  }
  const known = ["--dry-run", "-n", "--managed"];
  const unknown = args.filter((a) => !known.includes(a));
  if (unknown.length > 0) {
    console.error(`claude-code-config: unknown argument(s): ${unknown.join(", ")}`);
    console.error("Try: tools/claude-code-config -h");
    process.exit(1);
  }
  if (MANAGED) deployManagedLayer();
  else deployUserLayer();
}

if (import.meta.main) main();
