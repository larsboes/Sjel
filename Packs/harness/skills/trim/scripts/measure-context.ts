#!/usr/bin/env bun
// measure-context.ts — the deterministic half of trim.
//
// It answers three questions with numbers instead of impressions: what is
// always loaded, how big is each part, and which parts are provably dead —
// a pointer to a file that no longer exists, or a line repeated in two files
// that both load every session.
//
// It reads. It never edits. Every judgment call belongs to the human gate in
// SKILL.md, and this script exists so that gate gets facts.
//
// Usage:
//   bun measure-context.ts [--root DIR] [--home DIR] [--json]
//
// --root is the project whose CLAUDE.md counts as always-on (default: cwd).

import { existsSync, readFileSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";

type Entry = { path: string; bytes: number; lines: number; why: string };
type Section = { file: string; heading: string; bytes: number; lines: number };
type DeadPointer = { file: string; line: number; pointer: string };
type Duplicate = { text: string; files: string[] };

function arg(name: string, fallback: string): string {
  const i = process.argv.indexOf(`--${name}`);
  return i === -1 ? fallback : (process.argv[i + 1] ?? fallback);
}

const home = arg("home", process.env.HOME ?? "");
const root = resolve(arg("root", process.cwd()));
const warnings: string[] = [];
const entries: Entry[] = [];
const seen = new Set<string>();

/** True when the file is accounted for — already counted, or read and counted now.
 *  False only when it could not be read, which is what the caller reports.
 *
 *  The read is the existence check. An `existsSync` before it is a second answer to
 *  the same question taken from a different instant (CodeQL js/file-system-race), and
 *  it buys nothing here: a directory or an unreadable file throws, and the throw is
 *  the same "not a file I can count" the `isFile()` test used to give. */
function add(path: string, why: string): boolean {
  const full = resolve(path);
  if (seen.has(full)) return true;
  let body: string;
  try {
    body = readFileSync(full, "utf8");
  } catch {
    return false;
  }
  seen.add(full);
  entries.push({ path: full, bytes: Buffer.byteLength(body), lines: body.split("\n").length, why });
  // Claude Code's @-import: a line whose first token is @<path>. Relative paths
  // resolve against the importing file, ~ against home.
  for (const line of body.split("\n")) {
    const match = /^\s*@([^\s]+)\s*$/.exec(line);
    if (!match) continue;
    const target = match[1].startsWith("~/") ? join(home, match[1].slice(2)) : resolve(dirname(full), match[1]);
    if (!add(target, `imported by ${short(full)}`)) {
      // "missing or unreadable", not "does not exist": the read is now the only
      // probe, so a directory and a deleted file arrive here the same way.
      warnings.push(`${short(full)} imports ${match[1]}, which is missing or unreadable`);
    }
  }
  return true;
}

function short(path: string): string {
  return path.startsWith(home) ? `~${path.slice(home.length)}` : relative(root, path) || path;
}

// The always-on set, in the order the harness loads it.
add(join(home, ".claude", "CLAUDE.md"), "user-level, every session");
add(join(root, "CLAUDE.md"), "project-level, every session in this project");
add(join(root, "AGENTS.md"), "project-level, every session in this project");
add(join(root, ".claude", "CLAUDE.md"), "project-level, every session in this project");
const memoryDir = join(home, ".claude", "projects", root.replace(/\//g, "-"), "memory");
add(join(memoryDir, "MEMORY.md"), "memory index, injected every session");
if (!entries.length) warnings.push("nothing found: no CLAUDE.md or AGENTS.md at the user or project level");

// Sections, so a trim can target a part rather than a file.
const sections: Section[] = [];
for (const entry of entries) {
  const lines = readFileSync(entry.path, "utf8").split("\n");
  let heading = "(before the first heading)";
  let buffer: string[] = [];
  const flush = () => {
    if (!buffer.length) return;
    sections.push({
      file: short(entry.path),
      heading,
      bytes: Buffer.byteLength(buffer.join("\n")),
      lines: buffer.length,
    });
    buffer = [];
  };
  for (const line of lines) {
    if (/^#{1,3} /.test(line)) { flush(); heading = line.replace(/^#+\s*/, ""); }
    buffer.push(line);
  }
  flush();
}
sections.sort((a, b) => b.bytes - a.bytes);

// Dead pointers: a rule that points at a file which is not there any more is
// weight with no reader. Only unambiguous path shapes are checked, so a false
// positive here is a bug rather than a judgment call.
const dead: DeadPointer[] = [];
for (const entry of entries) {
  readFileSync(entry.path, "utf8").split("\n").forEach((line, i) => {
    // The lookbehind is load-bearing: without it, `Knowledge-Base/Projects/x.md`
    // matches from its inner slash as `/Projects/x.md` and every relative path in
    // the file is reported dead. Only rooted pointers are checked at all, because
    // a bare relative path has no reliable base directory from here.
    for (const raw of line.match(/(?<![A-Za-z0-9._~/-])(?:~\/|\.\/|\/)[A-Za-z0-9._~/-]*[A-Za-z0-9._-]/g) ?? []) {
      const pointer = raw.replace(/[.,;:)]+$/, "");
      // A file (extension), a directory (trailing slash), or a rooted path with at
      // least two segments. The extension rule alone missed every directory reference:
      // the first corpus this ran against named a tool by its directory, and that
      // directory had been deleted five days earlier.
      const segments = pointer.replace(/^~\//, "").split("/").filter(Boolean);
      if (!/\.[a-z0-9]{1,5}$|\/$/i.test(pointer) && segments.length < 2) continue;
      if (/^\/(usr|bin|etc|var|opt|tmp|dev|proc|Library|System|Applications)\b/.test(pointer)) continue;
      const target = pointer.startsWith("~/")
        ? join(home, pointer.slice(2))
        : isAbsolute(pointer)
          ? pointer
          : resolve(dirname(entry.path), pointer);
      if (!existsSync(target)) dead.push({ file: short(entry.path), line: i + 1, pointer });
    }
  });
}

// A line that carries a directive and appears in two always-on files is loaded
// twice every session. Short and structural lines are excluded.
const byLine = new Map<string, Set<string>>();
for (const entry of entries) {
  for (const raw of readFileSync(entry.path, "utf8").split("\n")) {
    const text = raw.trim().replace(/^[-*]\s+/, "");
    if (text.length < 40 || /^[#>|`]/.test(text)) continue;
    if (!byLine.has(text)) byLine.set(text, new Set());
    byLine.get(text)!.add(short(entry.path));
  }
}
const duplicates: Duplicate[] = [...byLine.entries()]
  .filter(([, files]) => files.size > 1)
  .map(([text, files]) => ({ text: text.length > 120 ? `${text.slice(0, 119)}…` : text, files: [...files] }));

const totalBytes = entries.reduce((sum, e) => sum + e.bytes, 0);
const report = {
  measuredAt: new Date().toISOString(),
  root,
  totals: { files: entries.length, bytes: totalBytes, approxTokens: Math.round(totalBytes / 4) },
  files: entries
    .map((e) => ({ ...e, path: short(e.path), approxTokens: Math.round(e.bytes / 4) }))
    .sort((a, b) => b.bytes - a.bytes),
  sections: sections.slice(0, 20),
  deadPointers: dead,
  duplicates,
  warnings,
};

if (process.argv.includes("--json") || !process.stdout.isTTY) {
  console.log(JSON.stringify(report, null, 2));
} else {
  console.log(`always-on: ${report.totals.files} files, ${report.totals.bytes} B, ~${report.totals.approxTokens} tokens\n`);
  for (const f of report.files) {
    console.log(`${String(f.bytes).padStart(7)} B  ~${String(f.approxTokens).padStart(5)} tok  ${f.path}  (${f.why})`);
  }
  console.log("\nlargest sections");
  for (const s of report.sections.slice(0, 10)) {
    console.log(`${String(s.bytes).padStart(7)} B  ${s.file} › ${s.heading}`);
  }
  if (dead.length) {
    console.log("\ndead pointers");
    for (const d of dead) console.log(`  ${d.file}:${d.line}  ${d.pointer}`);
  }
  if (duplicates.length) {
    console.log("\nduplicated directives");
    for (const d of duplicates) console.log(`  ${d.files.join(" + ")}: ${d.text}`);
  }
  for (const w of warnings) console.log(`warning: ${w}`);
}
