import { statSync } from "node:fs";
import { classify, lex, resolvePath, type Scope } from "./classify.ts";

/** Targets that must not gain approval from a routine-file scope. */
function protectedTarget(path: string): boolean {
  return /(?:^|\/)(?:\.ssh|\.aws|\.gnupg|secrets|credentials|command-policy)(?:\/|$)/i.test(path) ||
    /(?:^|\/)(?:\.env(?:\..*)?|auth\.json|credentials(?:\..*)?|settings(?:\.local)?\.json|config\.toml|secrets-guard\.[jt]s)$/i.test(path) ||
    /(?:^|\/)(?:extensions|agents|hooks)(?:\/|$)/i.test(path) || /\.(?:pem|key)$/i.test(path);
}

/** Whether a write stays under a pinned task root without targeting a guarded file. */
export function scopedWrite(path: string, scope: Scope, roots: string[]): boolean {
  const resolved = resolvePath(path, scope);
  return !protectedTarget(resolved) && roots.some((root) => resolved === root || resolved.startsWith(`${root}/`));
}

/** Only simple, known file operations qualify; never reclassify a compound shell command. */
export function scopedCommand(command: string, scope: Scope, roots: string[]): boolean {
  const tokens = lex(command, scope.home);
  if (tokens.some((t) => t.kind !== "word" || t.dynamic || t.substitutions.length > 0)) return false;
  const words = tokens.filter((t) => t.kind === "word").map((t) => t.text);
  const [program, ...args] = words;
  const options = args.filter((a) => a.startsWith("-"));
  const paths = args.filter((a) => !a.startsWith("-"));
  if (paths.some((p) => /[*?\[\]{}]/.test(p))) return false;
  let targets: string[];
  if (program === "cp") {
    // No recursion, overwrite, option arguments, or directory destinations.
    if (paths.length !== 2 || !options.length || options.some((o) => !/^-[np]+$/.test(o)) || !options.join("").includes("n")) return false;
    const destination = resolvePath(paths[1], scope);
    try {
      if (statSync(destination, { throwIfNoEntry: false })?.isDirectory()) return false;
    } catch {
      return false;
    }
    targets = [paths[1]];
  } else if (program === "mkdir") {
    if (options.some((o) => o !== "-p")) return false;
    targets = paths;
  } else if (program === "touch") {
    if (options.length) return false;
    targets = paths;
  } else return false;
  if (!targets.length || targets.some((p) => !scopedWrite(p, scope, roots))) return false;
  // Classification still catches all non-path rules after the narrowly permitted expansion.
  return classify(words.map((w) => `'${w.replace(/'/g, `'\\''`)}'`).join(" "), { ...scope, writable: [...scope.writable, ...roots] }).class === "allow";
}
