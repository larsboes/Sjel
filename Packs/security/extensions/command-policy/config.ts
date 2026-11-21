import { lstatSync, readFileSync, statSync } from "node:fs";
import { isAbsolute, join } from "node:path";
import { resolvePath, type Scope } from "./classify.ts";

/** Read only additional restrictions. Legacy grants cannot widen a project session. */
export function loadProtectedRoots(scope: Scope): string[] {
  const overlay = process.env.SJEL_OVERLAY_ROOT ?? process.env.SJEL_PERSONAL_ROOT;
  if (!overlay) return [];
  if (!isAbsolute(overlay)) throw new Error("Command-policy overlay must be an absolute path.");
  const root = resolvePath(overlay, scope);
  const path = join(root, "config", "command-policy.json");
  let text: string;
  try {
    text = readFileSync(path, "utf8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT" && !lstatSync(path, { throwIfNoEntry: false })) return [root];
    throw error;
  }
  const config = JSON.parse(text);
  if (!config || typeof config !== "object" || Array.isArray(config)) throw new Error("Invalid command-policy restrictions.");
  const additional: unknown = Object.hasOwn(config, "protected_roots") ? config.protected_roots : [];
  if (!Array.isArray(additional)) throw new Error("protected_roots must be an array.");
  return [...new Set([root, ...additional.map((raw: unknown) => {
    if (typeof raw !== "string" || !isAbsolute(raw)) throw new Error("Protected roots must be absolute directory paths.");
    const path = resolvePath(raw, scope);
    if (!statSync(path).isDirectory()) throw new Error("Protected roots must be existing directories.");
    return path;
  })])];
}
