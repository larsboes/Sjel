// The two scans over the dashboard's own source: `tools/dashboard-age.test.ts` (one module
// phrases an age) and `tools/dashboard-poll.test.ts` (one module polls). Both need the same
// three things — every component and module, path relative to the tree, and source with its
// comments blanked — and a second copy of a scan helper is how two gates start disagreeing
// about what they count.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { extname, join } from "node:path";

/** The dashboard's source root, resolved once from this file. */
export const SRC = join(import.meta.dir, "../../dashboard/src");

/** Every `.ts` and `.svelte` file under `dir`, recursively. */
export function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) return sources(full);
    const extension = extname(name);
    return extension === ".ts" || extension === ".svelte" ? [full] : [];
  });
}

/** A path as a reviewer names it: relative to the dashboard's source root. */
export const relative = (file: string) => file.slice(SRC.length + 1);

export const read = (file: string) => readFileSync(file, "utf8");

/**
 * Source with its comments blanked out, newlines kept.
 *
 * Blanked rather than deleted so every offset is still the offset in the file and a failure
 * names the line an editor opens at. The comments matter: they quote the very code a scan
 * looks for — this repository's doc comments name the functions and intervals they are
 * about — and a scan that reads them reports the defect it was written to prevent.
 */
export function code(text: string): string {
  const blank = (region: string) => region.replace(/[^\n]/g, " ");
  return text
    .replace(/<!--[\s\S]*?-->/g, blank)
    .replace(/\/\*[\s\S]*?\*\//g, blank)
    .replace(/(^|[^:])\/\/[^\n]*/g, (_match, before: string) => before);
}

/** The 1-based line an offset falls on. */
export const lineOf = (text: string, index: number) => text.slice(0, index).split("\n").length;
