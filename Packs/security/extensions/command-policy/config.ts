import { lstatSync, readFileSync, realpathSync, statSync } from "node:fs";
import { dirname, isAbsolute, join } from "node:path";
import { resolvePath, type Scope } from "./classify.ts";
import { scopedWrite } from "./scoped.ts";

/** Read machine-local grants only; malformed configuration throws without granting any root. */
export function loadMarkdownRoots(scope: Scope): string[] {
  const overlay = process.env.SJEL_OVERLAY_ROOT ?? process.env.SJEL_PERSONAL_ROOT;
  if (!overlay) return [];
  if (!isAbsolute(overlay)) throw new Error("Command-policy overlay must be an absolute path.");
  const path = join(overlay, "config", "command-policy.json");
  let text: string;
  try {
    text = readFileSync(path, "utf8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT" && !lstatSync(path, { throwIfNoEntry: false })) return [];
    throw new Error(`Cannot read command-policy config: ${path}`);
  }
  const config = JSON.parse(text);
  if (!config || typeof config !== "object" || Array.isArray(config) ||
    Object.keys(config).some((key) => key !== "markdown_write_roots") || !Array.isArray(config.markdown_write_roots)) {
    throw new Error("Command-policy config must contain only a markdown_write_roots array.");
  }
  const roots = config.markdown_write_roots.map((raw: unknown) => {
    if (typeof raw !== "string" || !isAbsolute(raw)) throw new Error("Markdown roots must be absolute directory paths.");
    const root = resolvePath(raw, scope);
    if (root === "/" || root === resolvePath(scope.home, scope) || !statSync(root).isDirectory()) {
      throw new Error("Markdown roots must name existing directories other than home or filesystem root.");
    }
    return root;
  });
  return [...new Set<string>(roots)];
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
  // resolvePath falls back for broken links; a grant must not use that fallback.
  let ancestor = isAbsolute(path) ? path : join(scope.cwd, path);
  try {
    while (!lstatSync(ancestor, { throwIfNoEntry: false })) {
      const parent = dirname(ancestor);
      if (parent === ancestor) return false;
      ancestor = parent;
    }
    realpathSync(ancestor);
    return roots.some((root) => realpathSync(root) === root && statSync(root).isDirectory() && scopedWrite(path, scope, [root]));
  } catch {
    return false;
  }
}
