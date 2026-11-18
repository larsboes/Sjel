import { lstatSync, readFileSync, statSync } from "node:fs";
import { isAbsolute, join } from "node:path";
import { pinnedWithin, protectedTarget, resolvePath, type Autonomy, type Scope } from "./classify.ts";
import { scopedWrite } from "./scoped.ts";

export interface MachinePolicy extends Autonomy {
  markdownRoots: string[];
}

/** Read machine-local grants only; malformed configuration grants no roots. */
export function loadMachinePolicy(scope: Scope): MachinePolicy {
  const empty = (): MachinePolicy => ({ markdownRoots: [], writeRoots: [], disposableRoots: [], protectedRoots: [] });
  const overlay = process.env.SJEL_OVERLAY_ROOT ?? process.env.SJEL_PERSONAL_ROOT;
  if (!overlay) return empty();
  if (!isAbsolute(overlay)) throw new Error("Command-policy overlay must be an absolute path.");
  const path = join(overlay, "config", "command-policy.json");
  let text: string;
  try {
    text = readFileSync(path, "utf8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT" && !lstatSync(path, { throwIfNoEntry: false })) return empty();
    throw new Error(`Cannot read command-policy config: ${path}`);
  }
  const config = JSON.parse(text);
  if (!config || typeof config !== "object" || Array.isArray(config) ||
    Object.keys(config).some((key) => !["markdown_write_roots", "write_roots", "disposable_roots", "protected_roots"].includes(key)) ||
    !Array.isArray(config.markdown_write_roots)) {
    throw new Error("Command-policy config requires markdown_write_roots and accepts write_roots, disposable_roots and protected_roots arrays.");
  }
  const roots = (values: unknown): string[] => {
    if (!Array.isArray(values)) throw new Error("Permission roots must be arrays.");
    return [...new Set<string>(values.map((raw: unknown) => {
      if (typeof raw !== "string" || !isAbsolute(raw)) throw new Error("Permission roots must be absolute directory paths.");
      const root = resolvePath(raw, scope);
      if (root === "/" || root === resolvePath(scope.home, scope) || !statSync(root).isDirectory()) {
        throw new Error("Permission roots must name existing directories other than home or filesystem root.");
      }
      return root;
    }))];
  };
  const policy = {
    markdownRoots: roots(config.markdown_write_roots),
    writeRoots: roots(config.write_roots ?? []),
    disposableRoots: roots(config.disposable_roots ?? []),
    protectedRoots: roots(config.protected_roots ?? []),
  };
  for (const root of policy.disposableRoots) {
    if (protectedTarget(root) || /(?:^|\/)target\/(?:release|debug)$|(?:^|\/)target\/release\//.test(root) ||
        policy.writeRoots.includes(root) || !pinnedWithin(root, scope, policy.writeRoots) ||
        policy.protectedRoots.some((p) => root === p || root.startsWith(`${p}/`) || p.startsWith(`${root}/`))) {
      throw new Error("Disposable roots must be unprotected output directories strictly below a write root, excluding live build trees.");
    }
  }
  return policy;
}

/** Compatibility reader for Markdown-only callers. */
export function loadMarkdownRoots(scope: Scope): string[] {
  return loadMachinePolicy(scope).markdownRoots;
}

function policyTarget(path: string): boolean {
  return /(?:^|\/)(?:AGENTS|CLAUDE|GEMINI|SKILL)\.md$/i.test(path) ||
    /(?:^|\/)copilot-instructions\.md$/i.test(path) ||
    /(?:^|\/)(?:\.obsidian|\.pi|\.claude|\.agents|\.codex|\.gemini|\.github|config|prompts)(?:\/|$)/i.test(path);
}

/** Check Markdown and canonical containment, including dangling links, without granting shell access. */
export function scopedMarkdownWrite(path: string, scope: Scope, roots: string[]): boolean {
  if (!path.endsWith(".md") || policyTarget(path)) return false;
  const resolved = resolvePath(path, scope);
  if (!resolved.endsWith(".md") || policyTarget(resolved) || !scopedWrite(path, scope, roots)) return false;
  return pinnedWithin(path, scope, roots);
}
