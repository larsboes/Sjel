// check-doc-links.ts — relative-link gate for Markdown (CI: repo gates).
// Every relative link in a *.md file under the working directory must name an existing
// path, and a `#fragment` on a Markdown target must name one of that file's headings.
// Pure file-based, like the sibling gates: no git, no network.
//
// Usage: bun tools/check-doc-links.ts   (run from the tree to check)
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, normalize, relative } from "node:path";

const SKIP_DIRS = new Set([".git", "node_modules", "target", "vendor", "dist", ".build"]);
/** Trees whose links resolve somewhere else: third-party notices and the overlay template. */
const SKIP_PREFIXES = [".claude/worktrees/", "tools/distribution/notices/", "tools/templates/"];

export type Broken = { file: string; target: string; reason: "missing" | "anchor" };

/** Walks `root` for *.md files, skipping build output, dependencies and SKIP_PREFIXES. */
export function markdownFiles(root: string): string[] {
  const out: string[] = [];
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      const rel = relative(root, path);
      if (SKIP_PREFIXES.some(prefix => `${rel}/`.startsWith(prefix))) continue;
      if (entry.isDirectory()) {
        if (!SKIP_DIRS.has(entry.name)) walk(path);
      } else if (entry.name.endsWith(".md")) {
        out.push(rel);
      }
    }
  };
  walk(root);
  return out.sort();
}

/** Removes fenced blocks and inline code, where a link is an example and not a pointer. */
export function stripCode(text: string): string {
  return text.replace(/^(```|~~~)[^\n]*\n[\s\S]*?^\1[^\n]*$/gm, "").replace(/`[^`\n]*`/g, "");
}

function* linkTargets(text: string): Iterable<string> {
  for (const link of text.matchAll(/\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)/g)) yield link[1];
  for (const [tag] of text.matchAll(/<a(?=\s|>)(?:[^'"<>]|"[^"]*"|'[^']*')*>/gi)) {
    for (const [, name, doubleQuoted, singleQuoted, unquoted] of tag.slice(2, -1).matchAll(/([^\s=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s=<>`]+)))?/g)) {
      const href = doubleQuoted ?? singleQuoted ?? unquoted;
      if (name.toLowerCase() === "href" && href?.trim()) yield href.trim();
    }
  }
}

/** GitHub's heading slug: lowercase, drop punctuation, spaces become hyphens, repeats get -n. */
export function headingAnchors(text: string): Set<string> {
  const seen = new Map<string, number>();
  const out = new Set<string>();
  for (const line of stripFences(text).split("\n")) {
    const m = /^#{1,6}\s+(.*?)\s*#*\s*$/.exec(line);
    if (!m) continue;
    const base = m[1]
      .replace(/<[^>]+>/g, "")
      .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
      .toLowerCase()
      .replace(/[^\p{L}\p{N}\s_-]/gu, "")
      .replace(/ /g, "-");
    const n = seen.get(base) ?? 0;
    seen.set(base, n + 1);
    out.add(n === 0 ? base : `${base}-${n}`);
  }
  for (const m of text.matchAll(/<a\s+(?:id|name)="([^"]+)"/g)) out.add(m[1]);
  return out;
}

function stripFences(text: string): string {
  return text.replace(/^(```|~~~)[^\n]*\n[\s\S]*?^\1[^\n]*$/gm, "");
}

const anchorCache = new Map<string, Set<string>>();
function anchorsOf(path: string): Set<string> {
  let a = anchorCache.get(path);
  if (!a) {
    a = headingAnchors(readFileSync(path, "utf8"));
    anchorCache.set(path, a);
  }
  return a;
}

/** Returns every broken relative link in the Markdown files under `root`. */
export function check(root: string): { files: number; broken: Broken[] } {
  const files = markdownFiles(root);
  const broken: Broken[] = [];
  for (const file of files) {
    const abs = join(root, file);
    const text = stripCode(readFileSync(abs, "utf8")).replace(/<!--[\s\S]*?-->/g, "");
    for (const target of linkTargets(text)) {
      if (/^[a-z][a-z0-9+.-]*:/i.test(target) || target.startsWith("//")) continue;
      const [path, fragment] = splitFragment(target);
      const resolved = path ? normalize(join(dirname(abs), decodeURIComponent(path))) : abs;
      if (path && !existsSync(resolved)) {
        broken.push({ file, target, reason: "missing" });
        continue;
      }
      if (fragment && resolved.endsWith(".md") && statSync(resolved).isFile()) {
        if (!anchorsOf(resolved).has(decodeURIComponent(fragment).toLowerCase())) {
          broken.push({ file, target, reason: "anchor" });
        }
      }
    }
  }
  return { files: files.length, broken };
}

function splitFragment(target: string): [string, string] {
  const i = target.indexOf("#");
  return i < 0 ? [target, ""] : [target.slice(0, i), target.slice(i + 1)];
}

if (import.meta.main) {
  const { files, broken } = check(process.cwd());
  if (files === 0) {
    console.error("FAIL: no *.md files found under the working directory");
    process.exit(1);
  }
  for (const b of broken) {
    const what = b.reason === "missing" ? "names a path that does not exist" : "names a heading the target does not have";
    console.error(`FAIL [${b.file}]: (${b.target}) ${what}`);
  }
  if (broken.length > 0) process.exit(1);
  console.log(`doc link check passed (${files} docs, every relative link and anchor resolves).`);
}
