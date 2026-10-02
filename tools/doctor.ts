// tools/doctor.ts — health checks for an already-set-up Axon machine:
// overlay reachability, machine.toml validity, declared state mounts,
// systems.toml coverage + undeclared-connection sweep, Pack deployment state. Real
// TOML parsing via Bun's built-in Bun.TOML — machine.toml uses array-of-tables
// ([[state_mount]]) that tools/lib/toml.sh's grep/sed single-line contract
// can't parse, which is why this is TS, not bash. It stays an interpreted command with no
// build step of its own (CONTRIBUTING.md#cargo-and-bun-are-the-build-path).
//
// Delegates each rule to the script that owns it rather than reimplementing it: the host
// toolchain to tools/toolchain-check, boot persistence to tools/service-runner.sh. Upstream
// freshness used to be delegated here too, to tools/upstream-checker; PRD Q41 retired that
// script on 2026-08-28 and Dependabot answers the question now, on GitHub rather than to a
// local health command. The same reasoning applies to the systems.toml/connection checks
// below: this extends the existing state-mount reality-check idiom (systems.toml stays the one
// hand-authored registry, CONTRIBUTING.md#one-manifest-per-concern) rather than adding a second manifest or a
// separate tool — see CONTRIBUTING.md#documentation-stays-owned-and-current.
//
//   tools/doctor            # full report, offline (no GitHub calls)
//   tools/doctor --online   # also probe declared systems and fetch origin/main
//   tools/doctor --version  # installed vs origin/main version identity only (read-only, exits 0)
//   tools/doctor -h         # this help
//
// Exit 0 = all checks pass, 1 = one or more failed. Invoke via the tools/doctor
// launcher (exec bun run), not this file directly, to match the printctl/uv
// launcher pattern (CONTRIBUTING.md#language-tooling).

import { existsSync, lstatSync, readdirSync, readFileSync, readlinkSync, statSync } from "node:fs";
import { basename, resolve, join, relative } from "node:path";
// Only the --online reachability probe uses this, to tell a dead hostname from a stopped service.
// The offline doctor path never calls it, so the report stays network-free where it must be.
import { lookup as dnsLookup } from "node:dns/promises";
import { HARNESSES, harnessById, isInstalled, type Harness } from "./lib/harness-registry.ts";
import { statusesFor } from "./harnesses.ts";
import { resolveMachineToml, resolveOverlayRoot } from "./lib/overlay.ts";
import { releaseTagGlob } from "./lib/release.ts";

const HELP = `tools/doctor — health checks for an already-set-up Axon machine.

  tools/doctor            full report, offline (no GitHub calls)
  tools/doctor --online   also probe declared systems and fetch origin/main
  tools/doctor --version  installed vs origin/main version identity only
                          (read-only, exits 0; add --online for a live fetch first)
  tools/doctor -h         this help
`;

const SJEL_ROOT = resolve(import.meta.dir, "..");
const HOME = process.env.HOME ?? "";

/// Discover Rust sources recursively so policy checks cover conventional
/// multi-file binary roots such as `src/server/main.rs`, not only flat crates.
export function findRustSources(root: string): string[] {
  if (!existsSync(root)) return [];
  const sources: string[] = [];
  const pending = [root];
  while (pending.length > 0) {
    const dir = pending.pop()!;
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) pending.push(path);
      else if (entry.isFile() && entry.name.endsWith(".rs")) sources.push(path);
    }
  }
  return sources.sort();
}

function expandHome(p: string): string {
  return p.startsWith("~") ? HOME + p.slice(1) : p;
}

// Pure, unit-testable core of the "Systems (systems.toml)" section: which
// machine.toml state_mount tools are covered by a systems.toml identity entry,
// and which aren't (see tools/doctor.test.ts).
export function checkStateMountCoverage(
  mounts: Array<{ tool: string }>,
  systemIds: Set<string>,
): { covered: string[]; uncovered: string[] } {
  const covered: string[] = [];
  const uncovered: string[] = [];
  for (const m of mounts) (systemIds.has(m.tool) ? covered : uncovered).push(m.tool);
  return { covered, uncovered };
}

// Pure, unit-testable core of the "Undeclared connections" sweep: extract
// candidate sibling-repo names from a blob of text. Repo paths can nest
// (Developer/Personal/Knowledge-Base, Developer/Collab/VBB), so the
// candidate is the LAST path segment (the actual repo dir), matched by
// basename against systems.toml ids — not the first segment after
// Developer/, which would misidentify nested paths as their parent folder.
// Deliberately narrow to $HOME/Developer/* and ~/Developer/* so this stays a
// fast, low-noise signal, not a generic path-linter. Known blind spots (not
// caught by this sweep): paths built from env vars or config indirection, where
// the literal never appears in the file, and Packs/*/pack.toml `links` entries
// that name a system without a literal $HOME path.
// `selfNames` are the roots a reference may hang from and still be about US: the checkout
// and the selected overlay. Matching them against the ROOT segment rather than the returned
// basename is the whole point. `~/Developer/<overlay>/config` is a path inside the overlay,
// but its last segment is `config`, so a basename-only self-check let it through and the
// sweep reported an undeclared connection to a repo named "config" that has never existed.
// Overlay-path leakage in tracked files is already owned by tools/check-publication-hygiene.sh
// — which is what the surviving hit was: that gate's own fixture, reported by this one.
export function extractSiblingRepoRefs(text: string, selfNames: Iterable<string> = []): string[] {
  const pathPattern = /(?:\$HOME|~)\/Developer\/((?:[A-Za-z0-9._-]+\/)*[A-Za-z0-9._-]+)/g;
  const self = new Set(selfNames);
  const names: string[] = [];
  for (const match of text.matchAll(pathPattern)) {
    const segments = match[1].split("/");
    if (self.has(segments[0])) continue;
    names.push(segments[segments.length - 1]);
  }
  return names;
}

// Pure, unit-testable core of the sweep's skip rule: is this file exempt from the
// hardcoded-path sweep by what it IS, rather than by being named in a list?
//
// Two properties replace two literals the list used to carry (Axon#26). A generated
// artifact says so in its own header, so ARCHITECTURE.md needed no name — and the next
// generated file will not need one either. A `.example` template's whole job is to SHOW a
// path so a reader knows what the field takes, which is the same rule the capability env
// templates already live under.
//
// What stays a literal is what no file property can express: the sanctioned indirection
// itself, the bootstrap that names overlay locations before any resolver exists, and the
// sweep's own test fixtures, whose paths are the specification of what a reference looks
// like rather than references. Naming those three is the rule; naming a generated file was
// a list.
export function isSweepExempt(relPath: string, text: string): boolean {
  if (relPath === "tools/lib/paths.sh" || relPath === "tools/install.sh") return true;
  if (relPath === "tools/doctor.test.ts") return true;
  if (relPath.endsWith(".example")) return true;
  // The header convention is "Auto-generated by <tool>", within the first lines. Scanning
  // the whole file instead would exempt any document that merely mentions the phrase.
  return /auto-generated\b/i.test(text.split("\n", 6).join("\n"));
}

// Pure, unit-testable core of the why-block base set: the prefixes a reference may be
// written against, derived from `git ls-files` (Axon#26).
//
// Three depths, each earning its place. A top-level owner (`capabilities/`), a unit inside
// it (`capabilities/comms/`), and that unit's sources (`capabilities/comms/src/`) — the
// last is the convention of naming a path relative to the crate that owns it
// (`sources/mod.rs`). The referencing document's own directory is added separately by
// findDecisionPathRot and covers the common case.
//
// Derived from tracked paths rather than from readdir for two reasons. It cannot invent a
// base nothing lives under, which the previous hand-list did by appending `<unit>/src/` to
// every unit whether or not it had one; and it cannot pick up an untracked build tree —
// `dashboard/node_modules/` as a resolution base would quietly make missing paths resolve.
export function whyBlockBases(trackedFiles: string[]): string[] {
  const bases = new Set<string>([""]);
  for (const f of trackedFiles) {
    const seg = f.split("/");
    if (seg.length > 1) bases.add(`${seg[0]}/`);
    if (seg.length > 2) bases.add(`${seg[0]}/${seg[1]}/`);
    if (seg.length > 3 && seg[2] === "src") bases.add(`${seg[0]}/${seg[1]}/src/`);
  }
  return [...bases].sort();
}

// Mask cfg(test) items before enforcing production-only Rust source policy.
// Keep newlines so any future diagnostics can still report useful line numbers.
function rustCfgItemEnd(source: string, start: number): number {
  let depth = 0;
  let bodyStarted = false;

  for (let i = start; i < source.length; i++) {
    if (source.startsWith("//", i)) {
      const newline = source.indexOf("\n", i + 2);
      if (newline === -1) return source.length;
      i = newline;
      continue;
    }
    if (source.startsWith("/*", i)) {
      let commentDepth = 1;
      i += 2;
      while (i < source.length && commentDepth > 0) {
        if (source.startsWith("/*", i)) {
          commentDepth++;
          i += 2;
        } else if (source.startsWith("*/", i)) {
          commentDepth--;
          i += 2;
        } else {
          i++;
        }
      }
      i--;
      continue;
    }

    const raw = source.slice(i).match(/^(?:br|r)(#*)"/);
    if (raw) {
      const terminator = `"${raw[1]}`;
      const close = source.indexOf(terminator, i + raw[0].length);
      if (close === -1) return source.length;
      i = close + terminator.length - 1;
      continue;
    }
    if (source[i] === '"') {
      i++;
      while (i < source.length) {
        if (source[i] === "\\") i += 2;
        else if (source[i] === '"') break;
        else i++;
      }
      continue;
    }
    if (source[i] === "'" && /^'(?:\\.|[^\\'\r\n])'/.test(source.slice(i))) {
      const close = source.indexOf("'", i + 1);
      i = close === -1 ? source.length : close;
      continue;
    }

    if (source[i] === "{") {
      bodyStarted = true;
      depth++;
    } else if (source[i] === "}" && bodyStarted) {
      depth--;
      if (depth === 0) return i + 1;
    } else if (source[i] === ";" && !bodyStarted) {
      return i + 1;
    }
  }

  return source.length;
}

export function stripRustCfgTestItems(source: string): string {
  const chars = [...source];
  const cfgTest = /#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]/g;
  let match: RegExpExecArray | null;

  while ((match = cfgTest.exec(source)) !== null) {
    const end = rustCfgItemEnd(source, cfgTest.lastIndex);
    for (let i = match.index; i < end; i++) {
      if (chars[i] !== "\n") chars[i] = " ";
    }
    cfgTest.lastIndex = end;
  }

  return chars.join("");
}

const OWN_LISTENER = [
  { name: "axum::serve", pattern: /\baxum::serve\s*\(/ },
  { name: "TcpListener::bind", pattern: /TcpListener::bind\s*\(/ },
] as const;

export function findProductionListenerConstructs(source: string): string[] {
  const production = stripRustCfgTestItems(source);
  return OWN_LISTENER.filter(({ pattern }) => pattern.test(production)).map(({ name }) => name);
}

// Pure helper for the env-template contract check: parse `KEY=VALUE` lines with
// optional quotes and inline comments, skipping blank/comment-only lines.
export function parseEnvTemplateLines(text: string): Array<{ key: string; value: string }> {
  const out: Array<{ key: string; value: string }> = [];
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const hash = line.indexOf(" #");
    const stripped = hash === -1 ? line : line.slice(0, hash).trim();
    const match = stripped.match(/^([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$/);
    if (!match) continue;
    const key = match[1];
    let value = match[2].trim();
    if (value.length >= 2 && ((value.startsWith("'") && value.endsWith("'")) || (value.startsWith('"') && value.endsWith('"')))) {
      value = value.slice(1, -1);
    }
    out.push({ key, value });
  }
  return out;
}

const SENSITIVE_ENV_HINT = /(PASS|PASSWORD|TOKEN|SECRET|KEY|CREDENTIAL|BEARER|HASH|SIGNATURE|PRIVATE)/i;

function isEnvValuePlaceholder(value: string): boolean {
  const v = value.trim();
  if (!v) return true;
  if (/^<[^>]+>$/.test(v)) return true;
  if (/^\$\{[^}]+\}$/.test(v)) return true;
  if (/^required:/i.test(v)) return true;
  if (/^(example|placeholder|changeme|change me|replace me)/i.test(v)) return true;
  return false;
}

function likelyRawSecret(value: string): boolean {
  const v = value.trim();
  if (!v || isEnvValuePlaceholder(v)) return false;
  if (v.length < 16) return false;
  if (/^[A-Za-z0-9+/=]+$/.test(v) && /[A-Za-z]/.test(v) && /[0-9]/.test(v)) return true;
  if (/^\$argon2/.test(v)) return true; // token hashes belong in overlay, never template
  return false;
}

export function findPlaintextSecretsInEnvTemplate(text: string): string[] {
  const out: string[] = [];
  for (const { key, value } of parseEnvTemplateLines(text)) {
    if (!SENSITIVE_ENV_HINT.test(key)) continue;
    if (likelyRawSecret(value)) out.push(key);
  }
  return out;
}

// Pure, unit-testable core of the "Decision freshness" sweep. A decision entry is a claim
// about the present written in the past tense, and nothing detects when the present changes:
// on 2026-07-16 the root-is-the-spine restructure dissolved `apps/`, and three entries kept
// asserting it for twelve days — one in the `rule:` line the generated index displays.
//
// This catches the half a machine can see: a path an entry names must exist, and a path it
// declares deliberately absent (frontmatter `asserts_absent: ["a/b"]`, single-line array, same
// not-a-full-parser contract as tools/lib/toml.sh) must stay absent. The second direction
// matters as much — something building the thing a decision forbids is rot too.
//
// It lives in doctor rather than a repo gate deliberately. Decisions legitimately name
// gitignored paths (`graphify-out/`, local scratch), and anything that sees tracked files only
// cannot tell an absent path from a rotten one. Measured when this ran as a Bazel test, before
// PRD Q44 retired Bazel on 2026-08-25: 16 findings against the real tree's 0, every one of
// them that blindness. Same reasoning as the topology sweep folded into
// CONTRIBUTING.md#documentation-stays-owned-and-current: extend doctor, don't add a tool.
//
// Blind to semantic rot (reasoning that stopped applying while the paths stayed valid). Path
// rot was 100% of what the 2026-07-28 audit found by hand, so this is the cheap majority.
export function findDecisionPathRot(
  entries: Array<{ slug: string; text: string; assertsAbsent: string[]; dir?: string }>,
  exists: (p: string) => boolean,
  // Prefixes a reference may be written against. A path is resolved against the referencing
  // document's own directory FIRST -- `references/house-style.md` inside a SKILL.md means
  // exactly that, and reading it any other way produced 171 findings across 98 files, nearly
  // all of them this one mistake. These extra bases then cover the other real convention:
  // naming a path relative to the crate that owns it (`sources/mod.rs`) rather than the root.
  bases: string[] = [""],
): Array<{ slug: string; path: string; kind: "missing" | "present" }> {
  const out: Array<{ slug: string; path: string; kind: "missing" | "present" }> = [];
  // Collapse `a/b/../c` so a parent-relative reference resolves.
  const norm = (p: string): string => {
    const stack: string[] = [];
    for (const seg of p.split("/")) {
      if (seg === "" || seg === ".") continue;
      if (seg === "..") stack.pop();
      else stack.push(seg);
    }
    return stack.join("/");
  };
  for (const { slug, text, assertsAbsent, dir } of entries) {
    const docBases = dir === undefined ? bases : [`${dir}/`, ...bases];
    const named = new Set<string>();
    // A backticked slug inside a link label whose destination is a URL names an external
    // resource, not a repo path: a model id like `org/model-name` written as the label of its
    // own huggingface link is the case that produced the only false positive this check has
    // had. The destination sitting right next to it is the authority on where the thing lives,
    // so drop those labels before scanning.
    const scanned = text.replace(/\[[^\]]*\]\((?:https?|mailto):[^)]*\)/g, " ");
    for (const m of scanned.matchAll(/`([A-Za-z0-9_./-]+\/[A-Za-z0-9_./-]+)`/g)) {
      let p = m[1].split("#")[0].replace(/[.,;:)]+$/, "").replace(/\/$/, "");
      // Absolute paths, URLs, git refs and shell/placeholder forms are not repo paths.
      if (!p || /^[/~$<*]/.test(p) || p.includes("://") || p.startsWith("origin/")) continue;
      if (/\.(com|org|io|dev|net)\//.test(p)) continue;
      named.add(p);
    }
    for (const p of named) {
      if (assertsAbsent.includes(p)) continue;
      if (!docBases.some((b) => exists(norm(b + p)))) out.push({ slug, path: p, kind: "missing" });
    }
    for (const p of assertsAbsent) {
      if (exists(p)) out.push({ slug, path: p, kind: "present" });
    }
  }
  return out;
}

// Pure, unit-testable core of the why-block half of the Decision freshness sweep. CONTRIBUTING.md#decisions-live-with-their-owner puts
// a decision's reasoning in the README of whatever it governs, under a `## Why this shape:`
// heading, so the rot check has to follow the prose there rather than only watching decisions/.
//
// Scoped to those blocks on purpose. Sweeping every tracked markdown file instead produced 139
// findings across 98 files — GitHub slugs (`gorse-io/gorse`), overlay paths, gitignored runtime
// artifacts — which is a general path linter, not a rot detector, and noise that size trains
// people to ignore the gate. The four why-blocks that exist today produce zero.
//
// `<!-- asserts-absent: a/b, c/d -->` inside a block is the markdown equivalent of the
// frontmatter key decisions/ entries use, for the same "this absence is the point" case.
export function collectWhyBlocks(
  file: string,
  text: string,
): Array<{ slug: string; text: string; assertsAbsent: string[]; dir: string }> {
  const out: Array<{ slug: string; text: string; assertsAbsent: string[]; dir: string }> = [];
  const dir = file.includes("/") ? file.slice(0, file.lastIndexOf("/")) : "";
  for (const m of text.matchAll(/^## Why this shape([^\n]*)\n([\s\S]*?)(?=^## |$(?![\s\S]))/gm)) {
    const body = m[2];
    const declared = body.match(/<!--\s*asserts-absent:([^>]*?)-->/);
    const assertsAbsent = (declared?.[1] ?? "")
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    out.push({ slug: `${file}${m[1].trim() ? ` (${m[1].replace(/^:\s*/, "").trim()})` : ""}`, text: body, assertsAbsent, dir });
  }
  return out;
}

// Pure, unit-testable core of the third Decision freshness check: a reference to a
// decisions/<slug> that no longer exists. Dissolving an entry means repointing everything that
// cited it, and during the 2026-07-28 curation that step was missed three batches running --
// partly because the generator *emits* decision paths into ARCHITECTURE.md, so a sweep that
// excluded the generated file could not see them. Cheap to check, invisible when skipped.
//
// A REPO path, not an HTTP one. Measured 2026-09-05: the finance capability's route for
// recomputing investment proposals matched this pattern and failed the gate in four files at
// once, in Rust, TypeScript and Markdown.
//
// The exclusion is on what an HTTP ROUTE looks like, not on any preceding slash. Excluding
// every match that follows a slash would also silence `docs/decisions/<slug>`,
// `./decisions/<slug>` and `Knowledge-Base/decisions/<slug>` -- real citations of a dissolved
// entry that happen to carry a directory in front of them, which is exactly what this sweep
// exists to catch. So the three route shapes this repository writes are named instead: a path
// under `api/`, a path built on a base-URL template, and an absolute http(s) URL.
const ROUTE_PREFIX = /(api\/|\$\{[^}]*\}\/|https?:\/\/[^\s"'`]*\/)$/;

export function findDanglingDecisionRefs(
  files: Array<{ path: string; text: string }>,
  slugExists: (slug: string) => boolean,
): Array<{ file: string; slug: string }> {
  const out: Array<{ file: string; slug: string }> = [];
  const seen = new Set<string>();
  for (const { path, text } of files) {
    for (const m of text.matchAll(/decisions\/([a-z0-9][a-z0-9-]*)/g)) {
      const prefix = text.slice(Math.max(0, (m.index ?? 0) - 64), m.index ?? 0);
      // `benchmarks/decisions/<file>` is the live model-evaluation dataset, not the dissolved decision archive.
      if (ROUTE_PREFIX.test(prefix) || /(?:^|[^A-Za-z0-9_-])benchmarks\/$/.test(prefix)) continue;
      const slug = m[1];
      const key = `${path}::${slug}`;
      if (seen.has(key) || slugExists(slug)) continue;
      seen.add(key);
      out.push({ file: path, slug });
    }
  }
  return out;
}

// Pure, unit-testable core of the version identity (`--version` fast path and
// the Session orientation version line): render a `git describe
// --tags --always --dirty` string (tag, bare sha, or either with git's own
// "-dirty" suffix — passed through verbatim, never re-derived here) plus the
// commit date. No I/O — callers do the git calls (see tools/doctor.test.ts).
export function formatVersion(describe: string, commitDate: string): string {
  if (!describe) return "(unknown — not a git checkout?)";
  return commitDate ? `${describe} (${commitDate})` : describe;
}

// Pure, unit-testable core of the fetch-age readout: how stale is the cached
// origin/main ref, from .git/FETCH_HEAD's mtime. null = no FETCH_HEAD at all
// (fresh clone that never fetched) — reported honestly, not as an error.
export function formatFetchAge(fetchEpochSeconds: number | null, nowEpochSeconds: number): string {
  if (fetchEpochSeconds === null) return "no fetch recorded";
  const age = Math.max(0, nowEpochSeconds - fetchEpochSeconds);
  if (age < 60) return "fetched just now";
  const minutes = Math.floor(age / 60);
  if (minutes < 60) return `fetched ${minutes} minute(s) ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `fetched ${hours} hour(s) ago`;
  return `fetched ${Math.floor(hours / 24)} day(s) ago`;
}

// --- backup receipts -------------------------------------------------------------------------
//
// `tools/backup.sh` writes one receipt per capability into <overlay>/backup/receipts/<cap>.json
// after the destination's byte count matched. It is the only local record that a backup landed:
// cleanup() removes the staging tree and the tarball, so nothing else here can answer "when was
// the last successful backup".
//
// Read directly rather than through sjel-status. That surface reads the same files and its
// `backup_state` is the rule mirrored below — but it deliberately drops `target`, `tarball` and
// `sha256` from its projection so no destination can reach an HTTP response
// (capabilities/sjel-status/src/status/backup.rs, on the Receipt struct), and those are exactly
// the fields needed to go and look at the archive. It also has to be running, and doctor's whole
// point is to work on a machine where things are not.
//
// So there are two readers of one file, in two languages, and that is a drift risk stated out
// loud rather than hidden: the timestamp parser and the state rule below are ports of
// `parse_receipt_ts` and `backup_state`, and doctor.test.ts pins both against a receipt written
// in `date -u +%Y%m%dT%H%M%SZ` form, which is what backup.sh actually emits.
export type BackupReceipt = {
  capability?: string;
  completed_at?: string;
  target?: string;
  tarball?: string;
  bytes?: number;
};

/// `20260906T210709Z` — fixed-width UTC, the shape `date -u +%Y%m%dT%H%M%SZ` produces.
/// null on anything else, which reads downstream as "no usable receipt": the same answer as a
/// missing file, and the right one, because a receipt this process cannot date cannot be used to
/// claim a backup is fresh.
export function parseReceiptTimestamp(stamp: string): number | null {
  const m = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/.exec(stamp);
  if (!m) return null;
  const [y, mo, d, h, mi, s] = m.slice(1).map(Number);
  if (mo < 1 || mo > 12 || d < 1 || d > 31 || h > 23 || mi > 59 || s > 60) return null;
  return Math.floor(Date.UTC(y, mo - 1, d, h, mi, s) / 1000);
}

/// Age against the capability's own two thresholds — the port of sjel-status' `backup_state`,
/// including why the two words differ: `due` means the data is older than the owner said it
/// should be, `overdue` means the schedule that should have refreshed it did not. `never` outranks
/// everything, because a capability with a backup contract and no receipt has the problem whatever
/// its thresholds say, and `unknown` is what a manifest declaring no cadence gets — a red badge
/// derived from a number Axon invented is worse than no badge.
export function backupAgeState(
  ageSeconds: number | null,
  adviseDays: number,
  staleDays: number,
): "never" | "overdue" | "due" | "unknown" | "ok" {
  if (ageSeconds === null) return "never";
  const days = Math.floor(ageSeconds / 86_400);
  if (Number.isFinite(staleDays) && days >= staleDays) return "overdue";
  if (Number.isFinite(adviseDays) && days >= adviseDays) return "due";
  if (!Number.isFinite(staleDays) && !Number.isFinite(adviseDays)) return "unknown";
  return "ok";
}

/// Is the archive the receipt names still AT the destination, with the byte count the receipt
/// recorded?
///
/// This exists because a receipt is a claim about the past and a destination is a live directory.
/// The vault's first archive shipped 704 MB, verified its size at the target, wrote a receipt,
/// reported success and could never have been restored (2026-08-29, tools/backup.sh
/// verify_archive) — and separately, the destination this deployment ships to is an iCloud folder
/// that evicts under disk pressure, which leaves the archive listed, named, sized and not there
/// (2026-09-08, three of four archives). Both are invisible to anything that reads only the
/// receipt.
///
/// `flags` is BSD `stat -f %Sf`, empty where that is not available. `dataless` in it is macOS'
/// SF_DATALESS: the file provider evicted the contents and kept the name.
export function classifyArchiveAtTarget(input: {
  exists: boolean;
  sizeBytes: number | null;
  flags: string;
  receiptBytes: number;
}): { level: "ok" | "warn" | "bad"; detail: string } {
  if (!input.exists) {
    return { level: "bad", detail: "the archive the receipt names is not at the destination" };
  }
  if (input.flags.split(",").includes("dataless")) {
    const sizeNote = input.sizeBytes === input.receiptBytes
      ? "its name and size remain local"
      : "even its reported size differs from the receipt";
    return {
      level: "warn",
      detail:
        `the archive is offloaded: ${sizeNote}, but recovery requires an online ` +
        "download and verification; this offline check cannot prove the cloud copy is recoverable",
    };
  }
  if (input.sizeBytes !== input.receiptBytes) {
    return {
      level: "bad",
      detail: `the archive holds ${input.sizeBytes} bytes, the receipt recorded ${input.receiptBytes}`,
    };
  }
  return { level: "ok", detail: `${input.receiptBytes} bytes, present at the destination` };
}

/// A failed attempt, from the marker `capabilities/backup`'s runner writes.
///
/// The receipts cannot carry this: `tools/backup.sh` writes one only after a run lands, so a
/// run that failed leaves the previous receipt in place and the age line above keeps calling
/// it fresh. Measured 2026-09-29: store's iCloud uploads had failed for days, two gated runs
/// exited non-zero, and doctor still said "backed up 0.0d ago". The marker is removed the
/// moment a run succeeds, so one existing means the last attempt failed.
export function attemptFinding(
  attempt: { exit_code: number; at_epoch: number; detail: string },
  now: number,
): { level: "bad"; detail: string } {
  const since = Math.max(0, now - attempt.at_epoch);
  const hours = (since / 3_600).toFixed(1);
  const reason = attempt.detail.trim() === "" ? "" : `: ${attempt.detail.trim()}`;
  return {
    level: "bad",
    detail: `the last attempt FAILED ${hours}h ago (exit ${attempt.exit_code})${reason}`,
  };
}

// --- scheduled producers ----------------------------------------------------------------------
//
// A capability that declares `schedule` has no supervisor watching it. It is started, it runs, it
// exits, and the only thing that brings it back is the timer. So there is nothing to be "down":
// it stops producing and every surface keeps saying fine. Six units on this machine are in that
// shape and one of them is the backup.
//
// The boot-persistence check above asks whether the unit MATCHES THE DECLARATION. This asks the
// different question that nothing asked: did it actually run.

/// One row of `launchctl list`: `PID \t Status \t Label`, where Status is the job's last exit
/// status and either column may be `-` for "no answer". A label that is absent from this output
/// is not loaded, which for a timer means it will never fire.
/**
 * The capability a LaunchAgent file belongs to, under the current label prefix (com.sjel) or
 * the one before the 2026-09-26 rename (com.axon), which install-persistence clears.
 */
export function launchdUnitCapability(file: string): string | null {
  if (!file.endsWith(".plist")) return null;
  for (const prefix of ["com.sjel.", "com.axon."]) {
    if (file.startsWith(prefix)) return file.slice(prefix.length, -".plist".length);
  }
  return null;
}

export function parseLaunchdJobs(text: string): Map<string, { pid: number | null; lastExit: number | null }> {
  const jobs = new Map<string, { pid: number | null; lastExit: number | null }>();
  for (const line of text.split("\n")) {
    const cols = line.split("\t");
    if (cols.length < 3) continue;
    const label = cols[2].trim();
    if (!label || label === "Label") continue;
    const num = (c: string) => (/^-?\d+$/.test(c.trim()) ? Number(c.trim()) : null);
    jobs.set(label, { pid: num(cols[0]), lastExit: num(cols[1]) });
  }
  return jobs;
}

/// The three facts a scheduled LaunchAgent carries about its own running: how often, and where its
/// two output streams go.
///
/// Read out of the INSTALLED unit rather than recomputed from the manifest. The unit's interval is
/// the one launchd obeys, and the log paths are the files launchd truly appends to — a second copy
/// of `/tmp/axon-<cap>-schedule.log` in this file would be a literal to keep in step with
/// tools/service-runner.sh, and the drift would be silent (doctor would watch a file nothing
/// writes and report "no run has ever produced output"). Where the unit and the manifest disagree
/// about the interval, that is the boot-persistence check's `stale` state, not this one's.
export function parseLaunchdSchedule(plist: string): {
  intervalSeconds: number | null;
  stdoutPath: string | null;
  stderrPath: string | null;
} {
  const str = (key: string) =>
    new RegExp(`<key>${key}</key>\\s*<string>([^<]*)</string>`).exec(plist)?.[1] ?? null;
  const interval = /<key>StartInterval<\/key>\s*<integer>(\d+)<\/integer>/.exec(plist);
  return {
    intervalSeconds: interval ? Number(interval[1]) : null,
    stdoutPath: str("StandardOutPath"),
    stderrPath: str("StandardErrorPath"),
  };
}

export function formatAge(seconds: number): string {
  if (seconds < 90) return `${Math.round(seconds)}s`;
  if (seconds < 3600) return `${Math.round(seconds / 60)}m`;
  if (seconds < 172_800) return `${(seconds / 3600).toFixed(1)}h`;
  return `${(seconds / 86_400).toFixed(1)}d`;
}

export type ScheduledProducer = {
  name: string;
  unitInstalled: boolean;
  loaded: boolean;
  /// The job's last exit status as launchd remembers it; null when it has none to report.
  lastExit: number | null;
  intervalSeconds: number | null;
  /// Age of the newest of the unit's two output files, or null when neither exists.
  lastOutputAgeSeconds: number | null;
};

/// Did this producer run, and recently enough that its interval is being honoured?
///
/// The thresholds are one interval to warn and three to fail, and the third one is not arbitrary.
/// launchd's `StartInterval` does not fire while the machine sleeps; it fires once on wake. On a
/// laptop that is closed overnight, an hourly job legitimately shows an age of several intervals
/// with nothing wrong. Two is inside the ordinary range of "the lid was shut". Three means either
/// the producer stopped or the machine was off long enough that the report should say so anyway —
/// and for the daily contracts that matters most it is three days, against the twenty-seven that
/// D10 took to notice.
///
/// A non-zero exit outranks age: it is the more specific finding, and a job that fails fast still
/// touches its log, so its age can look perfectly healthy.
export function classifyScheduledProducer(p: ScheduledProducer): { level: "ok" | "warn" | "bad"; message: string } {
  const seen = p.lastOutputAgeSeconds === null ? "no output on record" : `last output ${formatAge(p.lastOutputAgeSeconds)} ago`;
  if (!p.unitInstalled) {
    // Reported, never passed over — but the boot-persistence check owns this finding and already
    // fails on it, and one condition counted twice reads as two problems.
    return { level: "ok", message: `${p.name} — no unit installed; the boot-persistence check above owns that` };
  }
  if (!p.loaded) {
    // Verifier, 2026-09-08: this one IS counted twice, unlike the branch above. The
    // boot-persistence check has its own `installed-not-loaded` state — service-runner.sh's
    // status_persistence asks launchctl the same question, and doctor warns on it
    // (the `case "installed-not-loaded"` arm of the Boot persistence check). Measured from the
    // main checkout: `tools/service-runner.sh persistence` returns
    // `host-patch  installed-not-loaded  the unit exists but the supervisor is not running it`.
    // It does not show up in a run taken from a git worktree because every unit reads `stale`
    // there — the generated unit embeds the runner's absolute path — and `stale` short-circuits
    // before the load state is asked for. So a worktree run cannot see the overlap.
    return {
      level: "warn",
      message: `${p.name} — its unit is installed and launchd has not loaded it, so the timer cannot fire (${seen})`,
    };
  }
  if (p.lastExit !== null && p.lastExit !== 0) {
    return {
      level: "bad",
      message: `${p.name} — its last scheduled run exited ${p.lastExit} (${seen})`,
    };
  }
  if (p.intervalSeconds === null) {
    return {
      level: "warn",
      message: `${p.name} — its unit declares no StartInterval, so there is no cadence to judge it against (${seen})`,
    };
  }
  const every = formatAge(p.intervalSeconds);
  if (p.lastOutputAgeSeconds === null) {
    // Not the same as "never ran". macOS clears /tmp of entries untouched for three days at boot,
    // and a run that prints nothing does not move an mtime either.
    return {
      level: "warn",
      message: `${p.name} — runs every ${every} and has written no output this machine still holds`,
    };
  }
  const age = formatAge(p.lastOutputAgeSeconds);
  if (p.lastOutputAgeSeconds >= p.intervalSeconds * 3) {
    return {
      level: "bad",
      message: `${p.name} — runs every ${every} and has produced nothing for ${age}; it has missed at least two runs`,
    };
  }
  if (p.lastOutputAgeSeconds >= p.intervalSeconds) {
    return { level: "warn", message: `${p.name} — runs every ${every}, last produced ${age} ago` };
  }
  return { level: "ok", message: `${p.name} — runs every ${every}, produced ${age} ago` };
}

// How long a single reachability probe may take, absent an overlay saying otherwise. Public Axon
// ships the default; a deployment that knows one of its endpoints is legitimately slow raises it
// for that entry via `probe_timeout_ms` rather than muting the check or raising it for everything.
// Measured case: build.nvidia.com answers 202 in ~9.2s, consistently, for both HEAD and GET.
export const PROBE_TIMEOUT_MS = 5000;

export type ProbeTarget =
  | { id: string; url: string; timeoutMs: number }
  | { id: string; skip: string };

// Pure core of the reachability check: which declared systems have an endpoint worth dialling,
// and where does its URL come from.
//
// Public Axon never holds a private endpoint. A private system's `url` is the literal sentinel
// `overlay:systems.local.toml`, and the real value is looked up by the SAME id in the active
// overlay. That indirection is the whole reason this can probe a private deployment without the
// public repo learning a hostname, so it is resolved here rather than by pattern-matching a URL
// shape somewhere in the section body.
//
// Everything that is not an http(s) endpoint is skipped WITH A REASON, never silently dropped:
// "nothing to probe" and "probe not attempted" are different answers, and a check that conflates
// them reports a green for an endpoint it never touched.
export function resolveProbeTargets(
  systemsToml: Record<string, any>,
  overlaySystems: Record<string, any>,
): ProbeTarget[] {
  const targets: ProbeTarget[] = [];
  for (const [id, entry] of Object.entries(systemsToml ?? {})) {
    if (!entry || typeof entry !== "object") continue;
    const overlayEntry = overlaySystems?.[id];
    // Probe policy is the OVERLAY's call, not Axon's: whether an endpoint should be dialled at
    // all depends on the deployment, and a public default that says "dial it" would be a
    // deployment decision shipped in a public repo.
    if (overlayEntry?.probe === "no") {
      targets.push({ id, skip: "overlay declares probe = \"no\"" });
      continue;
    }
    let url = typeof entry.url === "string" ? entry.url : "";
    if (url === "overlay:systems.local.toml") {
      const resolved = typeof overlayEntry?.url === "string" ? overlayEntry.url : "";
      if (!resolved) {
        targets.push({ id, skip: "private system, no url in the overlay" });
        continue;
      }
      url = resolved;
    }
    if (!url) {
      targets.push({ id, skip: "no url declared" });
      continue;
    }
    if (url === "local") {
      targets.push({ id, skip: "url = \"local\" — not an endpoint" });
      continue;
    }
    let parsed: URL;
    try {
      parsed = new URL(url);
    } catch {
      targets.push({ id, skip: "url is not parseable" });
      continue;
    }
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      targets.push({ id, skip: `${parsed.protocol.replace(":", "")} endpoint — only http(s) is probed` });
      continue;
    }
    // A URL carrying userinfo would put a credential on the wire on Axon's initiative. The
    // declaration is the operator's, but dialling it is ours, so this one is refused rather than
    // sent. Note the reason names the shape, never the value.
    if (parsed.username || parsed.password) {
      targets.push({ id, skip: "url embeds credentials — not probed" });
      continue;
    }
    // A per-entry timeout is probe policy, so it comes from the overlay like the rest of it. A
    // non-numeric or non-positive value falls back to the default rather than disabling the
    // timeout: an unbounded probe would hang the whole report on one bad declaration.
    const declared = Number(overlayEntry?.probe_timeout_ms);
    const timeoutMs = Number.isFinite(declared) && declared > 0 ? declared : PROBE_TIMEOUT_MS;
    targets.push({ id, url, timeoutMs });
  }
  return targets;
}

export type ProbeOutcome = "refused" | "timeout" | "unavailable";

// Which failure a thrown fetch error means, GIVEN that the hostname already resolved.
//
// That precondition is not a detail, it is the whole reason the caller resolves DNS separately.
// Bun's fetch reports `code: "ConnectionRefused"` for a genuinely refused socket AND for a
// hostname that does not exist — verified against both, 2026-08-06 on Bun 1.3.14. Classifying
// straight off the error therefore cannot tell a stopped service from a dead name, which is the
// one distinction an operator actually acts on differently. The DNS step upstream removes the
// ambiguity; by the time this runs, a connect failure really is a connect failure.
//
// Kept pure and separate so the mapping is testable without a network.
export function classifyProbeOutcome(err: unknown): ProbeOutcome {
  const name = (err as any)?.name ?? "";
  if (name === "TimeoutError" || name === "AbortError") return "timeout";
  const code = String((err as any)?.code ?? "");
  const message = String((err as any)?.message ?? "");
  if (code === "ETIMEDOUT" || /timed out/i.test(message)) return "timeout";
  // Bun's spelling and Node's, because this file runs under Bun but the shape is not guaranteed.
  if (code === "ConnectionRefused" || code === "ECONNREFUSED" || /refused|unable to connect/i.test(message)) {
    return "refused";
  }
  return "unavailable";
}

async function readToml(path: string): Promise<any> {
  return Bun.TOML.parse(await Bun.file(path).text());
}

// Trimmed stdout of a git command against this checkout, "" on failure —
// enough for the read-only version/orientation readouts below.
function gitOut(...args: string[]): string {
  const proc = Bun.spawnSync({ cmd: ["git", "-C", SJEL_ROOT, ...args], stdout: "pipe", stderr: "pipe" });
  return proc.exitCode === 0 ? proc.stdout.toString().trim() : "";
}

// Which tags are release tags — axon.toml [release] tag_glob, the one home shared with
// tools/lib/version.sh (CONTRIBUTING.md#the-release-line). Resolved once at load; a missing key is a
// broken manifest and should stop the tool, not be papered over with a literal.
const RELEASE_TAG_GLOB = releaseTagGlob(SJEL_ROOT);

// Newest semver release tag (vX.Y.Z), or "" if none cut yet. Mirrors tools/lib/delta.sh's
// latest_release_ref so doctor --version and update.sh agree on "the newest release" — the first
// tag in descending version order that is actually a dotted number (a stray -rc/non-version tag
// is skipped, not mistaken for the latest release).
function latestReleaseTag(): string {
  const tags = gitOut("tag", "-l", RELEASE_TAG_GLOB, "--sort=-v:refname");
  if (!tags) return "";
  for (const raw of tags.split("\n")) {
    const t = raw.trim();
    if (/^v?\d+(\.\d+)*$/.test(t)) return t;
  }
  return "";
}

// `tools/doctor --version` fast path — version identity only, no health
// checks. Read-only, always exits 0: "what am I running and is origin newer"
// is a question, not a check that can fail. Offline by default (reads the
// cached origin/main ref + FETCH_HEAD age, says so honestly); --online
// fetches first, same split as the full report.
function printVersion(online: boolean): void {
  console.log(`Axon doctor --version · ${SJEL_ROOT}`);
  if (online) {
    Bun.spawnSync({ cmd: ["git", "-C", SJEL_ROOT, "fetch", "--quiet", "origin", "main"], stdout: "pipe", stderr: "pipe" });
  }

  const describe = gitOut("describe", "--tags", "--always", "--dirty", "--match", RELEASE_TAG_GLOB);
  console.log(`  installed: ${formatVersion(describe, gitOut("log", "-1", "--format=%cs"))}`);

  // Release-aware: once tags exist, say where this checkout sits relative to the newest release,
  // not only the moving main branch. Silent when no release has been cut yet.
  const latestTag = latestReleaseTag();
  if (latestTag) {
    console.log(`  release:   ${formatVersion(latestTag, gitOut("log", "-1", "--format=%cs", latestTag))} — newest release tag`);
  }

  const originSha = gitOut("rev-parse", "--short", "origin/main");
  if (!originSha) {
    console.log("  latest:    unknown — no origin/main ref cached (run tools/doctor --version --online)");
    return;
  }

  // FETCH_HEAD mtime = when this checkout last asked origin anything. Resolve
  // the git dir properly (worktrees have a .git *file*), fall back gracefully.
  let fetchEpoch: number | null = null;
  try {
    const gitDir = gitOut("rev-parse", "--absolute-git-dir") || join(SJEL_ROOT, ".git");
    fetchEpoch = Math.floor(statSync(join(gitDir, "FETCH_HEAD")).mtimeMs / 1000);
  } catch {
    // no FETCH_HEAD — formatFetchAge(null, …) reports it
  }
  const fetchAge = formatFetchAge(fetchEpoch, Math.floor(Date.now() / 1000));
  const liveness = online ? "" : " (offline — run with --online for live)";
  console.log(`  latest:    ${formatVersion(originSha, gitOut("log", "-1", "--format=%cs", "origin/main"))} — origin/main, ${fetchAge}${liveness}`);

  // Same rev-list idiom as the full report's "Repo freshness" section.
  const counts = gitOut("rev-list", "--left-right", "--count", "HEAD...origin/main");
  if (!counts) return;
  const [aheadStr, behindStr] = counts.split(/\s+/);
  const ahead = Number(aheadStr) || 0;
  const behind = Number(behindStr) || 0;
  if (ahead === 0 && behind === 0) console.log("  up to date with origin/main");
  else if (behind > 0 && ahead === 0) console.log(`  ${behind} commit(s) behind origin/main — run tools/update.sh`);
  else if (ahead > 0 && behind === 0) console.log(`  ${ahead} commit(s) ahead of origin/main — push when ready`);
  else console.log(`  diverged from origin/main (${ahead} ahead, ${behind} behind) — merge before tools/update.sh`);
}

// Everything below is the executable report — guarded by the import.meta.main
// line at the bottom so tools/doctor.test.ts can import
// checkStateMountCoverage/extractSiblingRepoRefs without running the whole CLI
// (console output, process.exit) as a side effect of import.

// What one check hands the next. The report is a chain, not a set: the overlay
// path resolved first is what machine.toml is read from, machine.toml is where
// the mounts come from, and the sweep needs both plus systems.toml. Everything
// that crosses a section boundary travels here; anything a section only uses
// itself stays local to its run().
type CheckContext = {
  root: string;
  overlayPath: string;
  machineToml: any;
  mounts: any[];
  systemsToml: Record<string, any>;
  online: boolean;
  ok(msg: string): void;
  bad(msg: string): void;
  warn(msg: string): void;
};

// `name` is printed as the section header, so CHECKS' array order below IS the
// order of the report.
type Check = { name: string; run(ctx: CheckContext): void | Promise<void> };

/**
 * Packs sections generated from the harness registry rather than hardcoded: one
 * section per INSTALLED harness, plus one consolidated warning per absent harness
 * that still holds deployed units. The registry decides what counts (isInstalled),
 * because "which harnesses are here" is the question a health report must not
 * answer from a stale list — the 2026-09-07 scar: three Packs sat for an uninstalled
 * Codex while the installed pi got zero rows at all.
 */
function packStatusSections(): Check[] {
  const home = process.env.HOME ?? "";
  const hints: Record<string, { deploy: (pack: string) => string; sync: (pack: string) => string }> = {
    claude: {
      deploy: (pack) => `tools/packs.sh link ${pack}`,
      sync: (pack) => `tools/packs-claude sync ${pack}`,
    },
    codex: {
      deploy: (pack) => `tools/packs-codex deploy ${pack}`,
      sync: (pack) => `tools/packs-codex sync ${pack}`,
    },
    opencode: {
      deploy: (pack) => `tools/packs-opencode deploy ${pack}`,
      sync: (pack) => `tools/packs-opencode sync ${pack}`,
    },
    pi: {
      deploy: (pack) => `tools/packs-pi deploy ${pack}`,
      sync: (pack) => `tools/packs-pi sync ${pack}`,
    },
  };

  const sections: Check[] = [];
  for (const harness of HARNESSES) {
    if (!isInstalled(harness)) continue;
    sections.push({
      name: `Packs (${harness.label} deployed)`,
      run(ctx) {
        try {
          const rows = statusesFor(harness);
          if (rows.length === 0) {
            ctx.warn("no Packs/*/pack.toml found");
            return;
          }
          const commands = hints[harness.id];
          const unselected: string[] = [];
          for (const row of rows) {
            const label = `${row.pack}/${row.skill}`;
            const detail = row.detail ? ` — ${row.detail}` : "";
            switch (row.status) {
              case "current":
                ctx.ok(`${label} current`);
                break;
              case "not-deployed":
                // Registry selection (pi) is partial by design — profiles decide what
                // loads. A warning per unselected Pack would be the noise this section
                // exists to end; accumulate and summarize once instead.
                if (harness.model === "registry") unselected.push(row.pack);
                else ctx.warn(`${label} not deployed (${commands.deploy(row.pack)})`);
                break;
              case "discovered":
                ctx.ok(`${label} loaded by pi via discovery, not the ledger${detail}`);
                break;
              case "outdated":
                ctx.warn(`${label} outdated (${commands.sync(row.pack)})${detail}`);
                break;
              case "drifted":
                ctx.bad(`${label} has destination-side changes; sync/remove will refuse`);
                break;
              case "migration-required":
                ctx.warn(
                  `${label} needs generated-artifact ledger migration${harness.id === "codex" ? ` (tools/packs-codex migrate-generated ${row.pack} --accept-current)` : ""}${detail}`,
                );
                break;
              case "missing":
                ctx.bad(`${label} is ledger-owned but missing${detail}`);
                break;
              case "collision":
                ctx.bad(
                  `${label} destination is occupied by an unowned skill${harness.id === "claude" ? ` (tools/packs-claude adopt ${row.pack} if it is identical)` : ""}`,
                );
                break;
              case "invalid":
                ctx.bad(`${label} invalid${detail}`);
                break;
            }
          }
          if (harness.model === "registry" && unselected.length) {
            const packs = [...new Set(unselected)].sort();
            ctx.ok(`${packs.length} Pack(s) not selected for pi: ${packs.join(", ")} (selection is the design — profiles decide)`);
          }
        } catch (error) {
          ctx.bad(`${harness.label} Pack state unreadable: ${(error as Error).message}`);
        }
      },
    });
  }

  // Absent harnesses still holding deployed units: one warning line instead of a
  // section — and when that destination is also a discovery root of an installed
  // harness (pi reads ~/.agents/skills), the units are NOT inert: say who reads them.
  for (const harness of HARNESSES) {
    if (isInstalled(harness) || harness.model !== "materialized") continue;
    const deployed = statusesFor(harness).filter((row) => row.status !== "not-deployed");
    if (!deployed.length) continue;
    const packs = [...new Set(deployed.map((row) => row.pack))].sort();
    const piReads = isInstalled(harnessById("pi")) && harness.config().destination === join(home, ".agents", "skills");
    sections.push({
      name: `Packs (${harness.label} NOT installed)`,
      run(ctx) {
        const base = `${deployed.length} units from ${packs.length} Pack(s) sit at ${harness.config().destination} for a ${harness.label} that is not installed`;
        if (piReads) {
          ctx.warn(
            `${base} — pi IS installed and discovers that directory, so pi is loading these right now. ` +
              `Keep them in pi (tools/packs-pi deploy ${packs.join(" ")}) before removing; otherwise they leave both harnesses.`,
          );
        } else {
          ctx.warn(`${base}; nothing reads them. Remove: ${harness.cli} remove ${packs.join(" ")}`);
        }
      },
    });
  }
  return sections;
}

const CHECKS: Check[] = [
  // overlay location. axon.local.toml (gitignored, per-machine) wins; the tracked
  // axon.toml carries only a shipped default, which is what keeps the repo gates
  // working — a fresh CI clone has the tracked file and no local one.
  // Mirrors tools/lib/paths.sh's resolution order; see
  // schemas/machine.toml.example.
  {
    name: "Overlay",
    async run(ctx) {
      const overlay = resolveOverlayRoot(ctx.root);
      if (!overlay) {
        ctx.bad("no 'overlay' in axon.local.toml or axon.toml — run tools/install.sh");
      } else {
        ctx.overlayPath = overlay.root;
        if (existsSync(ctx.overlayPath)) {
          ctx.ok(`overlay at ${ctx.overlayPath} (from ${overlay.source})`);
          // Falling back means this machine never recorded its own location. It works only
          // as long as the shipped default happens to be right, so say so out loud.
          if (overlay.source === "axon.toml") {
            ctx.warn("no axon.local.toml — this machine is running on the shipped default; run tools/install.sh to pin it");
          }
        } else {
          ctx.bad(`overlay declared (from ${overlay.source}) but missing at ${ctx.overlayPath} — run tools/install.sh`);
        }
      }
    },
  },

  // machine.toml — this machine's whole identity: platform, enabled capabilities,
  // and its state-mount registry.
  {
    name: "Machine identity",
    async run(ctx) {
      if (ctx.overlayPath && existsSync(ctx.overlayPath)) {
        // An overlay may own several machines. Say which one was resolved and how —
        // a report that silently read the wrong machine's manifest would still look
        // clean, and every check below inherits this answer.
        const machine = resolveMachineToml(ctx.overlayPath);
        const machineTomlPath = machine?.path ?? join(ctx.overlayPath, "config", "machine.toml");
        if (!existsSync(machineTomlPath)) {
          const named = machine?.source === "axon.local.toml"
            ? ` — axon.local.toml names machine '${machine.name}', which has no manifest`
            : " — run tools/install.sh";
          ctx.bad(`missing ${machineTomlPath}${named}`);
        } else {
          if (machine?.source === "config/machine.toml") ctx.ok("machine: single-file layout");
          else ctx.ok(`machine: ${machine?.name} (from ${machine?.source})`);
          ctx.machineToml = await readToml(machineTomlPath);
          if (ctx.machineToml.os) ctx.ok(`os = ${ctx.machineToml.os}`);
          else ctx.bad("machine.toml: missing 'os'");
          if (ctx.machineToml.container_runtime) ctx.ok(`container_runtime = ${ctx.machineToml.container_runtime}`);
          else ctx.bad("machine.toml: missing 'container_runtime'");
        }

        // The schema has one home. Three overlay-local copies of it existed until
        // 2026-08-04 and had already drifted — never in structure, only in which OS each
        // happened to name, which is the drift you get for free when a template is copied
        // per consumer. An overlay commits its real machine.toml anyway, so an example
        // beside it is a second thing to update and the first to go stale.
        const strayExamples = [
          join(ctx.overlayPath, "config", "machine.toml.example"),
          join(ctx.overlayPath, "config", "machines", "machine.toml.example"),
        ].filter(existsSync);
        if (strayExamples.length === 0) {
          ctx.ok("machine schema: not duplicated into the overlay");
        } else {
          for (const stray of strayExamples) {
            ctx.bad(
              `${stray} duplicates schemas/machine.toml.example — delete it and drop its ` +
                `.gitignore allowlist line; the schema lives in Axon only`,
            );
          }
        }
      } else {
        ctx.warn("skipped — no overlay to check");
      }
    },
  },

  // Host toolchain — delegate to tools/toolchain-check, don't reimplement: the script owns
  // the rule, doctor reports. This is the exemplar the delegations below point at. Reads
  // --json so a required miss maps to bad and an optional absence to warn, rather than
  // collapsing everything into one exit code. os/runtime come from machine.toml when we
  // have it; the checker self-resolves from uname otherwise.
  {
    name: "Host toolchain (tools/toolchain-check)",
    run(ctx) {
      const checkerPath = join(ctx.root, "tools", "toolchain-check");
      if (!existsSync(checkerPath)) {
        ctx.warn(`missing ${checkerPath}`);
        return;
      }
      const args = ["--json"];
      if (ctx.machineToml?.os) args.push("--os", ctx.machineToml.os);
      if (ctx.machineToml?.container_runtime) args.push("--runtime", ctx.machineToml.container_runtime);
      const proc = Bun.spawnSync({ cmd: [checkerPath, ...args], stdout: "pipe", stderr: "pipe" });
      let data: any;
      try {
        data = JSON.parse(proc.stdout.toString());
      } catch {
        ctx.warn("toolchain-check did not emit JSON — run tools/toolchain-check for detail");
        return;
      }
      const entries: any[] = Array.isArray(data.entries) ? data.entries : [];
      const missing = entries.filter((e) => e.status === "missing");
      const outdatedReq = entries.filter((e) => e.status === "outdated" && e.class !== "optional");
      const absent = entries.filter((e) => e.status === "absent");
      const outdatedOpt = entries.filter((e) => e.status === "outdated" && e.class === "optional");
      for (const e of missing) ctx.bad(`${e.bin} missing (${e.class}) — install: ${e.install}`);
      for (const e of outdatedReq) ctx.bad(`${e.bin} ${e.note} — install: ${e.install}`);
      for (const e of absent) ctx.warn(`${e.bin} absent (optional) — install: ${e.install}`);
      for (const e of outdatedOpt) ctx.warn(`${e.bin} ${e.note}`);
      if (missing.length === 0 && outdatedReq.length === 0) {
        const tail = absent.length ? `, ${absent.length} optional absent` : "";
        // Say the count is SCOPED. Without this the number silently shrank when needed_by landed,
        // and "8/8 present" on a machine whose manifest declares seventeen tools reads as a
        // partial check rather than a complete one over the applicable set (#163).
        const naCount = entries.filter((e) => e.status === "n/a").length;
        const scope = naCount ? `, ${naCount} n/a here` : "";
        ctx.ok(`${data.totals?.ok ?? 0}/${data.totals?.count ?? 0} required present${tail}${scope}`);
      }
      if (entries.some((e) => e.status === "n/a")) {
        ctx.ok("scoped to this machine — 'tools/toolchain-check --workflow backup|restore|audit|build' before running one");
      }
    },
  },

  // The tracked UI bunfig files own Axon's repo-level hold. This check owns only the optional
  // laptop-wide copy: it is a warning, not a failure, because the global file affects projects
  // outside Axon and install.sh asks before creating it. The scanner must NOT be global; Bun
  // requires it in each project's dependency tree, which the tree-local gate checks.
  {
    name: "Global Bun/npm install policy",
    async run(ctx) {
      if (!HOME) {
        ctx.warn("HOME is unset — cannot inspect ~/.bunfig.toml");
        return;
      }
      const path = join(HOME, ".bunfig.toml");
      if (!existsSync(path)) {
        ctx.warn("~/.bunfig.toml is absent — run tools/install.sh and accept the optional 24h npm/Bun hold");
        return;
      }
      try {
        const config = await readToml(path);
        const age = config?.install?.minimumReleaseAge;
        if (age === 86400) {
          ctx.ok("~/.bunfig.toml minimumReleaseAge = 86400 (24h)");
        } else if (age === undefined) {
          ctx.warn("~/.bunfig.toml has no install.minimumReleaseAge = 86400 — run tools/install.sh to add the hold");
        } else {
          ctx.warn(`~/.bunfig.toml minimumReleaseAge is ${String(age)}, expected 86400 — run tools/install.sh to reconcile it`);
        }
      } catch (error) {
        ctx.bad(`~/.bunfig.toml is not valid TOML — ${(error as Error).message}`);
      }
    },
  },

  // Local inference roles — delegate to tools/model-check --local, same shape as the
  // toolchain-check delegation above. Only the loopback backends: the full sweep dials
  // third-party APIs and spends quota, which nothing running on every invocation may do.
  //
  // This check exists because nothing had it. The `embedding` role named oMLX for as long as
  // oMLX had been uninstalled, `summarization` named it too, and every surface agreed the
  // machine was healthy: toolchain-check did not know the binary, doctor --online probed the
  // project homepage rather than the endpoint, and the Feed simply served its lexical fallback.
  // A declared role whose model is not there is the same class as the apple-on-device typo that
  // survived months, one layer down.
  {
    name: "Local inference roles (tools/model-check --local)",
    run(ctx) {
      const checker = join(ctx.root, "tools", "model-check.ts");
      if (!existsSync(checker)) {
        ctx.warn(`missing ${checker}`);
        return;
      }
      const proc = Bun.spawnSync({
        cmd: ["bun", checker, "--local", "--json"],
        stdout: "pipe",
        stderr: "pipe",
      });
      let data: any;
      try {
        data = JSON.parse(proc.stdout.toString());
      } catch {
        ctx.warn("model-check did not emit JSON — run 'bun tools/model-check.ts --local' for detail");
        return;
      }
      const entries: any[] = Array.isArray(data.entries) ? data.entries : [];
      if (entries.length === 0) {
        ctx.ok("no inference role names a loopback backend on this machine");
        return;
      }
      for (const e of entries) {
        // A model the backend does not list is a declaration that cannot work — bad. A backend
        // that is not running is machine state, and every consumer degrades rather than fails
        // (relevance to its lexical control, the digest ladder to the light and cloud rungs), so
        // it is a warning. Collapsing the two would make a stopped server fail doctor on a
        // laptop that is deliberately not serving models.
        if (e.status === "missing" || e.status === "incomplete") {
          ctx.bad(`${e.role}: ${e.model} on ${e.backend} — ${e.detail}`);
        } else if (e.status === "unreachable") {
          // Name the consequence, not just the state. "backend not reachable" reads as
          // infrastructure noise; "the Feed is ranking lexically" is the thing the operator
          // actually wanted to know, and it is what makes this line worth a second of attention.
          const cost =
            e.role === "embedding"
              ? " — relevance falls back to its lexical control until it is up"
              : e.role.startsWith("summarization")
                ? " — the digest ladder falls through to the remaining rungs"
                : "";
          ctx.warn(
            `${e.role}: ${e.model} on ${e.backend} — ${e.detail}${cost}. ` +
              `Axon does not supervise a systems.toml tool (see [${e.backend}]); start it, or point the role elsewhere.`,
          );
        }
      }
      const t = data.totals ?? {};
      if (!t.missing && !t.incomplete && !t.unreachable) {
        ctx.ok(`${t.ok}/${t.count} local role(s) answering`);
      }
    },
  },

  // Assistant harness integrations. This section is optional infrastructure, so
  // stale/incomplete state is a warning, not a hard failure; install-time
  // guidance is handled separately in tools/install.sh.
  {
    name: "AI assistant integrations",
    run(ctx) {
      const scriptPath = join(ctx.root, "tools", "agent-integrations.sh");
      if (!existsSync(scriptPath)) {
        ctx.warn(`missing ${scriptPath}`);
        return;
      }
      const proc = Bun.spawnSync({
        cmd: [scriptPath, "status", "--json"],
        stdout: "pipe",
        stderr: "pipe",
      });
      if (proc.exitCode !== 0) {
        ctx.warn(`agent-integrations status failed (run: tools/agent-integrations.sh status --json)`);
        return;
      }

      let payload: {
        integrations?: Array<{
          upstream?: string;
          harnesses?: Array<{
            name?: string;
            state?: "runnable" | "configured" | "integrated" | "stale" | string;
            command?: string;
            command_version?: string;
            graph_state?: string;
            install_command?: string;
            config_dir?: string;
          }>;
        }>;
      };

      try {
        payload = JSON.parse(proc.stdout.toString()) as any;
      } catch {
        ctx.warn("agent-integrations status did not emit JSON — run: tools/agent-integrations.sh status --json");
        return;
      }

      // Flatten the per-upstream envelope; each row keeps its upstream so the hints
      // below can say which installer owns it (graphify-specific advice stays
      // graphify-shaped — integration rows are not all graphify since 2026-09-11).
      const harnesses = (payload?.integrations ?? [])
        .flatMap((i) => (i.harnesses ?? []).map((h) => ({ upstream: h.upstream ?? i.upstream ?? "unknown", ...h })))
        .filter((h) => h.name);
      if (harnesses.length === 0) {
        ctx.warn("no assistant integration rows reported");
        return;
      }

      for (const item of harnesses) {
        const name = item.name || "unknown";
        const upstream = item.upstream || "graphify";
        const state = item.state || "unknown";
        const installCommand = (item.install_command || `tools/agent-integrations.sh install ${upstream} ${name}`).trim();
        const location = item.config_dir ? ` (${item.config_dir})` : "";
        const graphState = item.graph_state || "unknown";
        const command = item.command || "unknown";
        const commandVersion = item.command_version ? ` (${item.command_version})` : "";
        const stateSuffix = `graph=${graphState}; command=${command}${commandVersion}`;
        if (state === "integrated") {
          if (upstream === "graphify" && graphState !== "present") {
            ctx.warn(`${name}: ${state}${location}; ${stateSuffix}; check graph with tools/graphify.sh`);
          } else {
            ctx.ok(`${name}: ${state}${location}; ${stateSuffix}`);
          }
        } else if (state === "runnable") {
          ctx.warn(`${name}: ${state}${location}; ${stateSuffix}; install with: ${installCommand}`);
        } else if (state === "configured") {
          ctx.warn(
            `${name}: ${state}${location}; ${stateSuffix}; install command failed partially — check: ${installCommand}`,
          );
        } else if (state === "stale") {
          ctx.warn(`${name}: ${state}${location}; ${stateSuffix}; refresh with: ${installCommand}`);
        } else if (state === "missing") {
          ctx.warn(`${name}: ${state}${location}; ${stateSuffix}; install flow depends on harness presence`);
        } else {
          ctx.warn(`${name}: ${state}${location}; ${stateSuffix}; run: tools/agent-integrations.sh status --json`);
        }
      }
    },
  },

  // Enabled capability set. These two checks used to live in the repo gate
  // tools/check-manifest-integrity.sh, which could read the enabled set while it sat in
  // the tracked axon.toml. It now sits in the overlay, outside this repo, so
  // the machine-level checks belong to the machine-level tool. The gate keeps the
  // invariants that are intrinsic to the repo (every service.toml `requires =` resolves).
  // The Claude Code floor used to be a root-owned managed policy that no session could edit,
  // and a file nobody can edit needs no checker. Since the principal retired that layer on
  // 2026-10-02 the floor lands in ~/.claude/settings.json, which a session *can* edit, so this
  // is the only thing in the tree that would notice. Drift is a warn and never a bad: the
  // baseline also carries personal defaults (permission mode, env), so a changed mode is drift
  // and is not a security event — the message names the key paths so a lifted deny list can be
  // told from a changed preference. A machine with no settings file yet is skipped rather than
  // reported as total drift, which is what keeps this honest in CI, where HOME holds no harness
  // config at all.
  //
  // The binary is used only if it is already built, for the reason the Build artifacts section
  // gives: the fast local sweep must not pay for a release compile.
  {
    name: "Claude Code settings (sjel claude)",
    run(ctx) {
      const targetDir = process.env.CARGO_TARGET_DIR || join(ctx.root, "target");
      const bin = join(targetDir, "release", "sjel-claude-config");
      if (!existsSync(bin)) {
        ctx.warn("sjel-claude-config not built — run `sjel claude check` to compare settings");
        return;
      }
      const configuredDir = process.env.CLAUDE_CONFIG_DIR;
      const configDir = configuredDir
        ? expandHome(configuredDir)
        : join(process.env.HOME ?? "", ".claude");
      const target = join(configDir, "settings.json");
      if (!existsSync(target)) {
        ctx.warn(`no ${target} yet — apply Sjel's baseline with: sjel claude`);
        return;
      }
      // Spawned directly rather than through the launcher, so pass what the launcher would
      // have resolved: SJEL_ROOT locates the baseline, SJEL_OVERLAY_ROOT locates this
      // deployment's fragment. Without the second, every fragment rule would read as drift.
      // Only override it when this run actually resolved an overlay: ctx.overlayPath is empty
      // when it did not, and an empty string would mean "an overlay with no fragment here"
      // rather than "no overlay", which are different answers.
      const env: Record<string, string | undefined> = { ...process.env, SJEL_ROOT: ctx.root };
      if (ctx.overlayPath) env.SJEL_OVERLAY_ROOT = ctx.overlayPath;
      const proc = Bun.spawnSync({
        cmd: [bin, "check"],
        env,
        stdout: "pipe",
        stderr: "pipe",
      });
      const output = `${proc.stdout.toString()}${proc.stderr.toString()}`;
      if (proc.exitCode === 0) {
        const match = output.split("\n").find((line) => line.includes("matches the baseline"));
        ctx.ok(match?.replace(/^claude-code-config:\s*/, "") ?? "matches the baseline");
        return;
      }
      if (proc.exitCode !== 3) {
        // 2 is a settings.json that is not valid JSON, which the tool refuses to compare rather
        // than clobber; 1 is a missing baseline or a usage error. Neither is drift.
        ctx.warn(`check did not run: ${output.split("\n").find(Boolean) ?? `exit ${proc.exitCode}`}`);
        return;
      }
      const drift = output
        .split("\n")
        .filter((line) => /^\s+(changed|missing)\s{2}\S/.test(line))
        .map((line) => line.trim());
      ctx.warn(
        `${target} has drifted from Sjel's baseline: ${drift.join("; ") || "unknown keys"} · restore: sjel claude --force`,
      );
    },
  },
  {
    name: "Capabilities (enabled set)",
    async run(ctx) {
      const enabledCaps: string[] = Array.isArray(ctx.machineToml?.capabilities) ? ctx.machineToml.capabilities : [];
      if (!ctx.machineToml || Object.keys(ctx.machineToml).length === 0) {
        ctx.warn("skipped — no machine.toml to read");
      } else if (enabledCaps.length === 0) {
        ctx.warn("none enabled (tools/capability.sh enable <name>)");
      } else {
        const capRequires = new Map<string, string[]>();
        let allDirsPresent = true;
        for (const name of enabledCaps) {
          // Two roots, same as tools/lib/paths.sh's axon_manifest_for: public Axon holds
          // reusable capabilities, the active overlay holds deployment-specific ones.
          // Reported by root so a missing directory says which tree was searched, and
          // never by listing the overlay's contents — an overlay capability's name is a
          // fact about a private deployment.
          const rootDir = join(ctx.root, "capabilities", name);
          const overlayDir = join(ctx.overlayPath, "capabilities", name);
          const dir = existsSync(rootDir) ? rootDir : overlayDir;
          if (!existsSync(dir)) {
            ctx.bad(
              `'${name}' is enabled but exists in neither capabilities/${name}/ nor the overlay's`,
            );
            allDirsPresent = false;
            continue;
          }
          if (existsSync(rootDir) && existsSync(overlayDir)) {
            ctx.bad(`'${name}' is declared in both roots — rename one`);
            allDirsPresent = false;
            continue;
          }
          const svc = join(dir, "service.toml");
          if (existsSync(svc)) {
            const parsed = await readToml(svc);
            capRequires.set(name, Array.isArray(parsed?.requires) ? parsed.requires : []);
          } else {
            capRequires.set(name, []);
          }
        }
        if (allDirsPresent) ctx.ok(`${enabledCaps.length} enabled, every one a real capabilities/<name>/ dir in Axon or the overlay`);

        // Dependency closure: enabling X without what X requires is a machine that looks
        // configured and fails at start time. capability.sh resolves this on enable, but
        // hand-editing the list is legal, so it gets re-checked here.
        const enabledSet = new Set(enabledCaps);
        const missingDeps: string[] = [];
        for (const [name, reqs] of capRequires) {
          for (const dep of reqs) {
            if (!enabledSet.has(dep)) missingDeps.push(`${name} requires '${dep}', which is not enabled`);
          }
        }
        if (missingDeps.length === 0) ctx.ok("enabled set is dependency-closed");
        else for (const m of missingDeps) ctx.bad(m);
      }
    },
  },

  // Capabilities this machine CONSUMES from another overlay's deployment (retired-tracker#169).
  // The block above answers "what does this machine run"; this is the other half, and it lives
  // here rather than in a repo gate for the same reason the enabled set does — the declaration
  // is in the overlay, outside this repo.
  //
  // No URL is ever printed, matching the rule systems.toml already states for `doctor --online`:
  // a private endpoint resolved from the overlay has to survive being pasted into a report. The
  // id is enough to act on and names nothing.
  {
    name: "Capabilities (external references)",
    async run(ctx) {
      if (!ctx.machineToml || Object.keys(ctx.machineToml).length === 0) {
        ctx.warn("skipped — no machine.toml to read");
        return;
      }
      const perCapability = (ctx.machineToml.capability ?? {}) as Record<string, Record<string, unknown>>;
      const declared = Object.entries(perCapability)
        .filter(([, section]) => typeof section?.provided_by === "string" && section.provided_by !== "")
        .map(([name, section]) => [name, section.provided_by as string] as const);

      if (declared.length === 0) {
        ctx.ok("none declared — every capability here is locally managed");
        return;
      }

      const systemsPath = join(ctx.overlayPath, "config", "systems.local.toml");
      const systems = existsSync(systemsPath) ? ((await readToml(systemsPath)) ?? {}) : null;
      const enabled = new Set<string>(Array.isArray(ctx.machineToml?.capabilities) ? ctx.machineToml.capabilities : []);

      for (const [name, providerId] of declared) {
        // Both at once is a contradiction rather than a preference to resolve: the runner would
        // hold a local copy up while every client dialled the remote one, and the two would
        // diverge in silence, each perfectly healthy on its own.
        if (enabled.has(name)) {
          ctx.bad(`'${name}' is enabled AND declared as provided by '${providerId}' — it cannot be both`);
          continue;
        }
        if (systems === null) {
          ctx.bad(`'${name}' names provider '${providerId}', but the overlay has no config/systems.local.toml`);
          continue;
        }
        const entry = systems[providerId] as Record<string, unknown> | undefined;
        const url = typeof entry?.url === "string" ? entry.url : "";
        if (!url) {
          ctx.bad(`'${name}' names provider '${providerId}', which has no url in config/systems.local.toml`);
          continue;
        }
        ctx.ok(`'${name}' resolves through systems.local.toml [${providerId}]`);
      }
    },
  },

  // Boot persistence for the autostart set. A capability could declare autostart = true, be
  // enabled, run fine all day, and simply be gone after the next reboot — because nothing ever
  // called install-persistence and nothing ever checked (#9). Delegated to service-runner.sh,
  // which owns the rule; doctor reports. Same shape as the tools/toolchain-check delegation
  // above.
  //
  // The second half is the inverse and is not cosmetic: watchdog.sh calls
  // `service-runner.sh start <cap>` every 30s and consults nothing about the enabled set, so a
  // unit left behind by `capability.sh disable` walks a disabled capability back up.
  {
    name: "Boot persistence (autostart + schedule set)",
    async run(ctx) {
      const runner = join(ctx.root, "tools", "service-runner.sh");
      if (!existsSync(runner)) {
        ctx.warn(`missing ${runner}`);
        return;
      }
      const proc = Bun.spawnSync({ cmd: [runner, "persistence"], stdout: "pipe", stderr: "pipe" });
      const lines = proc.stdout.toString().trim().split("\n").filter(Boolean);
      if (lines.length === 0) {
        ctx.warn("service-runner.sh persistence returned nothing — persistence state unverified");
      }
      let owed = 0;
      for (const line of lines) {
        const [name, state, detail] = line.split("\t");
        switch (state) {
          case "installed":
          case "n/a":
            break;
          case "missing":
            // "owes a unit", not "declares autostart": a capability declaring `schedule` owes one
            // too, and for that one the unit is not a safety net against reboots — it is the only
            // thing that ever runs it.
            ctx.bad(`'${name}' owes a supervisor unit and has none installed — it will not run after a reboot (tools/service-runner.sh install-persistence ${name})`);
            owed++;
            break;
          case "misdeclared":
            // bad, not warn: a manifest claiming both autostart and schedule can never have
            // persistence installed at all, so there is no degraded mode to keep running in.
            ctx.bad(`'${name}': ${detail}`);
            owed++;
            break;
          case "stale":
            ctx.warn(`'${name}': ${detail}`);
            owed++;
            break;
          case "installed-not-loaded":
            ctx.warn(`'${name}': ${detail}`);
            owed++;
            break;
          case "unsupported":
            ctx.warn(`'${name}': ${detail}`);
            break;
          default:
            ctx.warn(`'${name}': unexpected persistence state '${state}'`);
            break;
        }
      }

      // Units for capabilities this machine does not enable. Names come from the unit filenames,
      // which are Axon's own (com.axon.<cap> / axon-<cap>.service), never from listing the
      // overlay — an overlay capability's name is a fact about a private deployment.
      const enabled = new Set<string>(Array.isArray(ctx.machineToml?.capabilities) ? ctx.machineToml.capabilities : []);
      const os = ctx.machineToml?.os;
      const home = process.env.HOME ?? "";
      const unitDir =
        os === "macos" ? join(home, "Library", "LaunchAgents")
        : os === "linux" ? join(process.env.XDG_CONFIG_HOME ?? join(home, ".config"), "systemd", "user")
        : "";
      // Names that legitimately own a unit while being absent from the enabled set. A spine
      // component is one (tools/lib/paths.sh's axon_manifest_for reads its manifest from the
      // repo root rather than capabilities/), and so is anything that component declares as a
      // `sidecars` entry — a unit it owns that has no manifest of its own because Axon does
      // not own the program.
      //
      // Derived from the manifests rather than from a literal. This used to be `cap !==
      // "dashboard"`, which flagged the dashboard's own macmon sidecar and told the operator
      // to remove the unit its Systems page polls (#65). A second hardcoded name would have
      // fixed that instance and left the next one.
      const exempt = new Set<string>();
      for (const entry of readdirSync(ctx.root, { withFileTypes: true })) {
        if (!entry.isDirectory()) continue;
        const svc = join(ctx.root, entry.name, "service.toml");
        if (!existsSync(svc)) continue;
        exempt.add(entry.name);
        const parsed = await readToml(svc);
        if (Array.isArray(parsed?.sidecars)) {
          for (const s of parsed.sidecars) if (typeof s === "string" && s) exempt.add(s);
        }
      }

      const orphans: string[] = [];
      if (unitDir && existsSync(unitDir)) {
        for (const f of readdirSync(unitDir)) {
          const cap =
            os === "macos" ? launchdUnitCapability(f)
            : (f.startsWith("axon-") && f.endsWith(".service") ? f.slice("axon-".length, -".service".length) : null);
          if (cap && !exempt.has(cap) && !enabled.has(cap)) orphans.push(cap);
        }
      }
      for (const cap of orphans.sort()) {
        ctx.warn(`persistence is installed for '${cap}', which this machine does not enable — its watchdog will start it anyway (tools/service-runner.sh remove-persistence ${cap})`);
      }

      if (owed === 0 && orphans.length === 0 && lines.length > 0) {
        ctx.ok(`${lines.length} enabled capabilities checked, persistence matches the declaration`);
      }
    },
  },

  // Did the scheduled producers actually run?
  //
  // The check above compares the installed unit to the declaration. That is a different question,
  // and a unit can match its declaration perfectly while the job behind it has not produced
  // anything for a week. Nothing asked the second question, for any of them: a `schedule`
  // capability has no supervisor, so it cannot be "down" — it simply stops, and every surface
  // stays green. This is D10's shape with six subjects instead of one.
  {
    name: "Scheduled producers (did they run)",
    async run(ctx) {
      const os = ctx.machineToml?.os;
      if (os !== "macos") {
        // systemd records a timer's last elapse in `systemctl show --property=LastTriggerUSec`,
        // which is a better source than a log mtime and a different implementation. Reported as a
        // skip with its reason rather than passed over: this machine's producers are the subject,
        // and a Linux host's are simply not covered yet.
        return ctx.ok(`skipped — os = ${os ?? "unknown"}; this reads launchd units, and systemd timers are not covered yet`);
      }
      const proc = Bun.spawnSync({
        cmd: [join(ctx.root, "tools/capability.sh"), "registry"],
        stdout: "pipe",
        stderr: "pipe",
      });
      if (proc.exitCode !== 0) return ctx.warn("capability.sh registry failed — skipping");
      let registry: Array<Record<string, string>>;
      try {
        registry = JSON.parse(proc.stdout.toString());
      } catch {
        return ctx.warn("capability.sh registry did not return JSON — skipping");
      }
      const enabled = new Set<string>(
        Array.isArray(ctx.machineToml?.capabilities) ? ctx.machineToml.capabilities : [],
      );
      const scheduled = registry.filter(
        (s) => s.schedule && s.schedule.trim() !== "" && (enabled.size === 0 || enabled.has(s.name)),
      );
      if (scheduled.length === 0) return ctx.ok("no capability on this machine declares a schedule");

      // One call, not one per unit. An absent label is the answer for "not loaded", so the whole
      // table has to be in hand before any of them is judged.
      const list = Bun.spawnSync({ cmd: ["launchctl", "list"], stdout: "pipe", stderr: "pipe" });
      if (list.exitCode !== 0) return ctx.warn("launchctl list failed — scheduled producers unverified");
      const jobs = parseLaunchdJobs(list.stdout.toString());

      const unitDir = join(process.env.HOME ?? "", "Library", "LaunchAgents");
      const now = Date.now() / 1000;
      for (const service of scheduled) {
        const label = [`com.sjel.${service.name}`, `com.axon.${service.name}`].find(
          (l) => existsSync(join(unitDir, `${l}.plist`)) || jobs.has(l),
        ) ?? `com.sjel.${service.name}`;
        const unitPath = join(unitDir, `${label}.plist`);
        const unitInstalled = existsSync(unitPath);
        const unit = unitInstalled
          ? parseLaunchdSchedule(readFileSync(unitPath, "utf8"))
          : { intervalSeconds: null, stdoutPath: null, stderrPath: null };
        // The newest of the two streams. A job that fails writes only to stderr and a job that
        // succeeds may write only to stdout, so taking one of them would make half the runs
        // invisible.
        let newest: number | null = null;
        for (const p of [unit.stdoutPath, unit.stderrPath]) {
          if (!p) continue;
          try {
            const mtime = statSync(p).mtimeMs / 1000;
            if (newest === null || mtime > newest) newest = mtime;
          } catch {
            // absent, which is not the same as never ran — classifyScheduledProducer says so.
          }
        }
        const job = jobs.get(label);
        const verdict = classifyScheduledProducer({
          name: service.name,
          unitInstalled,
          loaded: job !== undefined,
          lastExit: job?.lastExit ?? null,
          intervalSeconds: unit.intervalSeconds,
          lastOutputAgeSeconds: newest === null ? null : Math.max(0, now - newest),
        });
        // Every producer gets a line, including the healthy ones. A section that printed only its
        // problems would let a producer that quietly left the set — dropped from the registry,
        // renamed — read exactly like a producer that is fine.
        if (verdict.level === "bad") ctx.bad(verdict.message);
        else if (verdict.level === "warn") ctx.warn(verdict.message);
        else ctx.ok(verdict.message);
      }
    },
  },

  {
    // The shared SQLite database (PRD Q45, 2026-08-27). Every capability's tables are in one
    // file, so "is it there and does it open" is a machine-level question with one answer,
    // which is what makes it a doctor check rather than nine readiness handlers.
    //
    // Resolved exactly as `sjel_config::database_path()` resolves it — SJEL_DB_PATH first,
    // then the overlay — because a doctor that checked a different file than the capabilities
    // open would report on nothing. Absent is a WARNING, not a failure: a machine that has
    // never run a capability legitimately has no database yet, and `sjel_store::pool_for`
    // creates it on first open.
    name: "Shared store (SQLite)",
    run(ctx) {
      const envPath = (process.env.SJEL_DB_PATH ?? "").trim();
      const dbPath = envPath ? expandHome(envPath) : join(ctx.overlayPath, "data", "axon", "axon.db");
      const from = envPath ? "SJEL_DB_PATH" : "overlay default";
      if (!ctx.overlayPath && !envPath) return ctx.warn("skipped — no overlay to resolve the database path from");
      if (!existsSync(dbPath)) {
        ctx.warn(`no database at ${dbPath} (${from}) — created on the first write by any capability`);
        return;
      }

      // `integrity_check` on the LIVE file, and this one is a read: sqlite3 opens it
      // read-only through the URI, so a check cannot journal or write to the database every
      // capability is using. Bounded to the first line — a corrupt database answers with a
      // list, and the first entry is the one worth reporting.
      const proc = Bun.spawnSync({
        cmd: ["sqlite3", `file:${dbPath}?mode=ro`, "pragma integrity_check;"],
        stdout: "pipe",
        stderr: "pipe",
      });
      if (proc.exitCode === null || proc.exitCode !== 0) {
        const why = proc.stderr.toString().trim() || "sqlite3 is not on PATH";
        ctx.bad(`${dbPath} could not be read: ${why}`);
      } else {
        const first = proc.stdout.toString().trim().split("\n")[0] ?? "";
        if (first === "ok") ctx.ok(`${dbPath} (${from}) — integrity_check ok`);
        else ctx.bad(`${dbPath} failed integrity_check: ${first}`);
      }

      // The file has a backup contract, and the contract only reaches a backup surface
      // through the registry. A machine holding the database without `store` enabled has a
      // database nothing will ever back up — which is exactly the state a cutover from the
      // retired postgres capability leaves behind if only half of it is done.
      const enabled: string[] = Array.isArray(ctx.machineToml?.capabilities) ? ctx.machineToml.capabilities : [];
      if (!enabled.includes("store")) {
        ctx.warn(
          "'store' is not in this machine's enabled set — the database exists and no backup contract covers it (tools/capability.sh enable store)",
        );
      }
    },
  },

  {
    name: "State mounts",
    run(ctx) {
      ctx.mounts = ctx.machineToml?.state_mount ?? [];
      if (ctx.mounts.length === 0) {
        ctx.warn("none declared");
      } else {
        for (const m of ctx.mounts) {
          const p = expandHome(m.path);
          if (existsSync(p)) ctx.ok(`${m.tool} — ${p}`);
          else ctx.bad(`${m.tool} — ${p} missing`);
        }
      }
    },
  },

  // Capability env contract. Every env-backed capability in `capabilities/` must ship a
  // tracked `<name>.env.example` next to service.toml so non-secret defaults are versioned while
  // secret-bearing values remain private. Axon#188 owns the public contract and gate.
  //
  // The repo root only, deliberately, unlike the bind-policy check below. This gate exists
  // so a stranger cloning public Axon can see which variables a capability needs without
  // any of their values. An overlay capability has no such reader: the repository holding
  // it is already private, and its real env file lives beside it. Widening this check
  // would demand a template whose only audience already has the original.
  {
    name: "Capability env templates (public/private split)",
    async run(ctx) {
      const capsDir = join(ctx.root, "capabilities");
      if (!existsSync(capsDir)) return ctx.warn("no capabilities/ dir");
      let checked = 0;
      let foundTemplate = 0;
      for (const capDirEntry of readdirSync(capsDir, { withFileTypes: true })) {
        if (!capDirEntry.isDirectory()) continue;
        const capDir = join(capsDir, capDirEntry.name);
        const svc = join(capDir, "service.toml");
        if (!existsSync(svc)) continue;
        const parsed = await readToml(svc);
        const envFile = parsed?.env_file;
        if (typeof envFile !== "string" || !envFile.trim()) continue;
        checked += 1;
        const envBase = basename(envFile);
        if (!envBase.endsWith(".env")) {
          ctx.warn(`capabilities/${capDirEntry.name}: env_file should probably end with .env`);
          continue;
        }
        const template = join(capDir, `${envBase}.example`);
        if (!existsSync(template)) {
          ctx.bad(`capabilities/${capDirEntry.name}: missing ${envBase}.example for env-backed service`);
          continue;
        }
        foundTemplate += 1;
        let text: string;
        try {
          text = readFileSync(template, "utf8");
        } catch {
          ctx.bad(`capabilities/${capDirEntry.name}: cannot read ${envBase}.example`);
          continue;
        }
        const leaks = findPlaintextSecretsInEnvTemplate(text);
        if (leaks.length > 0) {
          for (const key of leaks) {
            ctx.bad(
              `capabilities/${capDirEntry.name}: ${envBase}.example contains raw-looking secret-like value for ${key} (use placeholders only)`,
            );
          }
        }
      }
      if (checked === 0) ctx.ok("no env_file-backed capabilities found");
      else if (foundTemplate === checked) ctx.ok(`${foundTemplate}/${checked} env-backed capabilities ship .env.example`);
      else ctx.bad(`${foundTemplate}/${checked} env-backed capabilities ship .env.example`);
    },
  },

  // systems.toml — coverage + undeclared-connection sweep. Read-only,
  // offline, mechanical: cross-reference against machine.toml's state_mount
  // (a system declared local="yes" ought to have a monitored path somewhere)
  // and grep this repo's own tracked files for hardcoded sibling-system paths
  // that bypass tools/lib/paths.sh's indirection — the two classes of drift
  // that hand-authored manifests silently accumulate (see
  // CONTRIBUTING.md#documentation-stays-owned-and-current for the
  // discovered-in-the-wild example: a sync script's env-overridable default
  // path diverging from the mount its own machine.toml declared).
  {
    name: "Systems (systems.toml)",
    async run(ctx) {
      const systemsTomlPath = join(ctx.root, "systems.toml");
      if (!existsSync(systemsTomlPath)) {
        ctx.warn("no systems.toml — skipped");
      } else {
        ctx.systemsToml = await readToml(systemsTomlPath);
        const systemIds = new Set(Object.keys(ctx.systemsToml));
        // Direction that's actually meaningful: machine.toml's [[state_mount]] is the
        // narrower, path-bearing list (CONTRIBUTING.md#one-manifest-per-concern — "one manifest per concern");
        // systems.toml is the broader identity/why registry. A mount with no
        // matching identity entry is a real gap (doctor already flags via `bad`
        // below). The reverse is NOT generally a gap — most local=yes systems
        // (tools, services, projects with no persisted state Axon backs up) never
        // need a mount by design, so flagging every one would just be noise.
        const { covered, uncovered } = checkStateMountCoverage(ctx.mounts, systemIds);
        for (const tool of covered) ctx.ok(`${tool} — state_mount has a matching systems.toml identity`);
        for (const tool of uncovered) ctx.bad(`${tool} — machine.toml [[state_mount]] with no systems.toml entry — undeclared system`);
        const localCount = Object.values(ctx.systemsToml).filter((e: any) => e?.local === "yes").length;
        const mountedCount = [...systemIds].filter((id) => ctx.mounts.some((m) => m.tool === id)).length;
        ctx.ok(`${mountedCount}/${localCount} local="yes" systems have a state_mount (rest are mount-less by design — tools/services with no persisted state)`);
      }
    },
  },

  // systems.toml reachability. The section above asks whether a system is DECLARED; this one asks
  // whether the declared endpoint answers. Both questions matter and neither substitutes for the
  // other: a complete manifest pointing at a dead host reports perfectly green without this.
  //
  // --online only, and the offline path does no network work at all — not a shortened timeout,
  // not a DNS lookup. `tools/doctor` is the thing an operator runs on a machine with no route out,
  // and a check that quietly dials in that state makes the whole report slow and unreliable
  // exactly where it is most needed.
  //
  // What it never prints: the URL. Not for a private system, not for a public one. The result
  // names the system id, the outcome, and the HTTP status when there was one, which is the bounded
  // evidence the declaration is entitled to. Printing a resolved endpoint would put a private
  // hostname into a report an operator pastes into an issue, and getting that right per-entry is a
  // rule someone eventually forgets. Cheaper to never print any.
  {
    name: "Systems reachability (--online)",
    async run(ctx) {
      if (!ctx.online) {
        ctx.ok("skipped — run 'tools/doctor --online' to probe declared endpoints");
        return;
      }
      if (!ctx.systemsToml || Object.keys(ctx.systemsToml).length === 0) {
        ctx.warn("no systems.toml entries — nothing to probe");
        return;
      }
      let overlaySystems: Record<string, any> = {};
      if (ctx.overlayPath) {
        const overlaySystemsPath = join(ctx.overlayPath, "config", "systems.local.toml");
        if (existsSync(overlaySystemsPath)) {
          try {
            overlaySystems = await readToml(overlaySystemsPath);
          } catch {
            // An unparseable overlay manifest is the overlay's finding, not this check's. Say the
            // probe set is incomplete rather than reporting every private system as unreachable,
            // which is the same false-red a missing file would produce.
            ctx.warn("overlay systems.local.toml is unreadable — private endpoints not resolved");
          }
        }
      }

      const targets = resolveProbeTargets(ctx.systemsToml, overlaySystems);
      const probed = targets.filter((t): t is { id: string; url: string; timeoutMs: number } => "url" in t);
      const skipped = targets.filter((t): t is { id: string; skip: string } => "skip" in t);

      // Concurrent, because these are independent network waits and a serial sweep would make the
      // section's cost the SUM of every timeout rather than the worst one.
      const results = await Promise.all(
        probed.map(async ({ id, url, timeoutMs }) => {
          // Resolve first, connect second. Bun's fetch reports the same ConnectionRefused code for
          // a refused socket and a nonexistent hostname, so without this step "the service is
          // stopped" and "the name is gone" arrive as one answer — and they are the two findings
          // an operator would act on most differently. An IP literal resolves trivially, so this
          // costs nothing for a LAN address.
          try {
            await dnsLookup(new URL(url).hostname);
          } catch {
            return { id, outcome: "unavailable" as ProbeOutcome, status: 0, timeoutMs };
          }
          try {
            const res = await fetch(url, {
              method: "HEAD",
              redirect: "manual",
              signal: AbortSignal.timeout(timeoutMs),
            });
            return { id, outcome: "reachable" as const, status: res.status, timeoutMs };
          } catch (err) {
            return { id, outcome: classifyProbeOutcome(err), status: 0, timeoutMs };
          }
        }),
      );

      for (const r of results) {
        // Any HTTP response proves the service answered. A 401 or a 403 is a reachable service
        // declining an unauthenticated HEAD, which is the correct behaviour for most of these and
        // must not read as an outage — this check asks "is it up", never "am I allowed in".
        if (r.outcome === "reachable") ctx.ok(`${r.id} — reachable (HTTP ${r.status})`);
        else if (r.outcome === "refused") ctx.bad(`${r.id} — connection refused`);
        else if (r.outcome === "timeout") ctx.warn(`${r.id} — no answer within ${r.timeoutMs}ms (overlay probe_timeout_ms raises it)`);
        else ctx.bad(`${r.id} — unavailable (no route, DNS failure, or TLS error)`);
      }
      for (const s of skipped) ctx.ok(`${s.id} — skipped: ${s.skip}`);
      if (probed.length === 0) ctx.warn("no probeable endpoint among the declared systems");
    },
  },

  // Undeclared-connection sweep: every declared canonical path (state mounts +
  // SJEL_ROOT + overlay) vs. every hardcoded sibling-repo path actually
  // committed in this tree. tools/lib/paths.sh is the sanctioned indirection
  // (SJEL_ROOT / SJEL_PERSONAL_ROOT); anything else hardcoding a path to a
  // declared system, or referencing a $HOME path to a system with NO
  // systems.toml entry at all, is exactly the kind of drift systems.toml can't
  // see by construction (it's hand-authored, so it only knows what someone
  // remembered to add).
  {
    name: "Undeclared connections (grep sweep)",
    async run(ctx) {
      const declaredIds = new Set(Object.keys(ctx.systemsToml));
      const selfRoots = [ctx.root, ctx.overlayPath].filter(Boolean).map((p) => basename(p));

      // A path the overlay declares as protected is a DENY rule, not a connection (Axon#147).
      // This sweep's premise is that a hardcoded sibling path implies an undeclared
      // integration, and for a protection zone that inference runs backwards: the path is
      // named precisely so that nothing integrates with it. Demanding a systems.toml entry
      // for it would be asking the operator to declare a connection to the one place they
      // declared off limits. Read here rather than threaded through ctx — this is its only
      // consumer, and the file is the overlay's, not Axon's.
      const protectedNames = new Set<string>();
      if (ctx.overlayPath) {
        const zonesPath = join(ctx.overlayPath, "config", "protection-zones.toml");
        if (existsSync(zonesPath)) {
          try {
            for (const m of readFileSync(zonesPath, "utf8").matchAll(/"([^"]+)"/g)) {
              const base = basename(m[1].replace(/\/+$/, ""));
              if (base) protectedNames.add(base);
            }
          } catch {
            // An unreadable policy is tools/protection-zones' finding to report, not this
            // sweep's. Saying it twice in two vocabularies teaches a reader to skim both.
          }
        }
      }

      const lsFiles = Bun.spawnSync({ cmd: ["git", "-C", ctx.root, "ls-files"], stdout: "pipe", stderr: "pipe" });
      if (lsFiles.exitCode !== 0) {
        ctx.warn("git ls-files failed — skipping sweep");
      } else {
        const files = lsFiles.stdout.toString().split("\n").filter(Boolean);
        const hits = new Map<string, Set<string>>(); // repo-name -> files referencing it
        for (const rel of files) {
          const abs = join(ctx.root, rel);
          let text: string;
          try {
            text = await Bun.file(abs).text();
          } catch {
            continue; // binary or unreadable — not a path-reference source
          }
          // Exemptions are a property of the file, not a list of names — see
          // isSweepExempt. Each skip it keeps has a reason no property can express:
          //
          //   tools/lib/paths.sh    the sanctioned indirection itself.
          //   tools/install.sh      the bootstrap namer. It shows the suggested overlay
          //                         path for each recognized boundary and writes the chosen
          //                         one into axon.local.toml, all before paths.sh can
          //                         resolve anything.
          //   tools/doctor.test.ts  its sibling-repo paths are ARGUMENTS to
          //                         extractSiblingRepoRefs, the very function this sweep
          //                         runs — the specification of what a reference looks
          //                         like, not a reference. Reading them as one made the
          //                         sweep report its own fixture permanently, which is the
          //                         kind of finding that teaches a reader to stop reading
          //                         the findings.
          //
          // axon.toml is deliberately NOT exempt: since the state mounts moved to the
          // overlay it holds no paths of its own, so a hardcoded one appearing there is a
          // real regression that should be reported, not hidden.
          if (isSweepExempt(rel, text)) continue;
          // The checkout and selected overlay are self-references, at any depth beneath
          // them. Any other hardcoded sibling path is a real undeclared connection.
          for (const name of extractSiblingRepoRefs(text, selfRoots)) {
            if (!hits.has(name)) hits.set(name, new Set());
            hits.get(name)!.add(rel);
          }
        }
        if (hits.size === 0) {
          ctx.ok("no hardcoded sibling-repo paths found outside tools/lib/paths.sh");
        } else {
          for (const [name, refFiles] of hits) {
            const slug = name.toLowerCase();
            if (protectedNames.has(name)) {
              ctx.ok(`${name} — declared a protected path in the overlay's protection-zones.toml; a deny rule, not a connection`);
            } else if (declaredIds.has(slug)) {
              ctx.warn(`${name} — hardcoded path in ${[...refFiles].join(", ")} (declared in systems.toml as '${slug}', but bypasses paths.sh indirection)`);
            } else {
              ctx.bad(`${name} — hardcoded path in ${[...refFiles].join(", ")}, no matching systems.toml entry — undeclared connection`);
            }
          }
        }
      }
    },
  },

  // Server bind policy. libs/sjel-server exists so a capability server cannot
  // bind the LAN or skip the SJEL_PORT contract by accident, and its README said
  // so while two servers contradicted it: scout-server bound 0.0.0.0 with
  // permissive CORS behind a mutating POST, and comms-server hand-rolled its
  // startup. A README claiming a guarantee nothing enforces is worse than no
  // claim, so the guarantee gets a check.
  //
  // Lives in doctor rather than a repo gate for the same reason the decision path-rot
  // sweep does (CONTRIBUTING.md#documentation-stays-owned-and-current): half the servers it has to
  // cover are in the overlay, outside this repo, and a gate that only globs Axon would report
  // a clean bind policy while an overlay server binds the LAN. doctor reads both real trees.
  {
    name: "Server bind policy (sjel-server)",
    run(ctx) {
      // Both roots. This is a security gate, not a public-code style rule: a server the
      // overlay owns can bind 0.0.0.0 just as wrongly as one in Axon, and it would be
      // the more dangerous of the two. Findings print to the terminal only, so naming an
      // overlay capability here does not put it in a tracked artifact.
      const roots = [
        { dir: join(ctx.root, "capabilities"), label: "capabilities" },
        { dir: join(ctx.overlayPath, "capabilities"), label: "overlay capabilities" },
      ];
      const rootCaps = join(ctx.root, "capabilities");
      if (!existsSync(rootCaps)) return ctx.warn("no capabilities/ dir");
      let checked = 0;
      let offenders = 0;
      for (const { dir: capsDir, label } of roots) {
        if (!existsSync(capsDir)) continue; // an overlay need not own any capability
        for (const cap of readdirSync(capsDir, { withFileTypes: true })) {
          if (!cap.isDirectory()) continue;
          const srcDir = join(capsDir, cap.name, "src");
          if (!existsSync(srcDir)) continue;
          for (const path of findRustSources(srcDir)) {
            const sourcePath = relative(srcDir, path);
            const text = readFileSync(path, "utf8");
            const production = stripRustCfgTestItems(text);
            if (!/\bRouter::new\s*\(/.test(production)) continue; // not a server root
            checked++;
            const hand = findProductionListenerConstructs(text);
            if (hand.length === 0) {
              // Either entry point: `serve_local` is loopback with the deployment's
              // inbound token, `serve` spells the reach and the gate out (comms passes
              // its own `api_secret_file` token). Both go through the same bind and the
              // same middleware, so accepting only the first would flag a correct server.
              if (!/sjel_server::serve(_local)?\s*\(/.test(production)) {
                ctx.warn(`${label}/${cap.name}/src/${sourcePath} builds a Router but neither serves it nor uses sjel_server`);
              }
              continue;
            }
            offenders++;
            ctx.bad(`${label}/${cap.name}/src/${sourcePath} binds its own listener — use sjel_server::serve_local (loopback + port contract)`);
          }
        }
      }
      if (checked === 0) return ctx.bad("no capability server sources found — this check is looking in the wrong place");
      if (offenders === 0) ctx.ok(`${checked} capability server(s) across both roots, none binds by hand`);
    },
  },

  // Tailnet identity gate. `SJEL_TAILNET_OPERATOR` says "admit this login from the
  // tailnet", and the thing that makes that statement true is not in this repository:
  // it is the shape of `tailscale serve`. An HTTPS web handler authenticates the peer
  // and injects `Tailscale-User-Login`, overwriting whatever the client sent (measured
  // against tailscale 1.102.3, 2026-09-06). A raw TCP forward injects nothing.
  //
  // So a serve config switched from web to TCP turns every tailnet request into
  // something the gate cannot distinguish from a loopback one, and libs/sjel-server
  // falls through to the token rule — which on this deployment is no rule at all. The
  // gate would stop gating, silently, with every process still healthy and every test
  // still green. That is the exact failure shape PRD §13 records four times over, so
  // the declaration gets a checker rather than a sentence.
  {
    name: "Tailnet identity gate (SJEL_TAILNET_OPERATOR)",
    run(ctx) {
      if (!ctx.overlayPath || !existsSync(ctx.overlayPath)) return ctx.warn("no overlay — cannot read deployment.env");
      const envPath = join(ctx.overlayPath, "config", "deployment.env");
      if (!existsSync(envPath)) return ctx.ok("no deployment.env — no tailnet gate declared");
      // The key under the name the rename settled on; libs/sjel-config's deployment_value
      // reads it the same way.
      const lines = readFileSync(envPath, "utf8").split("\n").map((l) => l.trim());
      const valueOf = (key: string) =>
        lines.find((l) => l.startsWith(`${key}=`))?.slice(key.length + 1).trim() || undefined;
      const declared = valueOf("SJEL_TAILNET_OPERATOR");
      if (!declared) {
        // Not a failure. The undeclared deployment is the one that predates this gate,
        // and libs/sjel-server ignores the identity header entirely in that state.
        return ctx.ok("no operator declared — the identity header is ignored, not trusted");
      }

      const serve = Bun.spawnSync({ cmd: ["tailscale", "serve", "status", "--json"], stdout: "pipe", stderr: "pipe" });
      if (!serve.success) return ctx.bad(`operator declared but 'tailscale serve status' failed — the gate depends on a proxy this machine cannot describe`);
      let config: any;
      try {
        config = JSON.parse(serve.stdout.toString() || "{}");
      } catch {
        return ctx.bad("operator declared but 'tailscale serve status --json' did not parse");
      }

      // A TCP forward is the dangerous shape: it proxies bytes and injects no identity,
      // so the gate sees every tailnet caller as a local one.
      const tcp = Object.entries(config.TCP ?? {}).filter(([, v]: any) => !v?.HTTPS);
      const webHandlers = Object.values(config.Web ?? {}).flatMap((host: any) => Object.entries(host?.Handlers ?? {}));
      const proxied = webHandlers.filter(([, h]: any) => typeof h?.Proxy === "string");

      if (proxied.length === 0) {
        return ctx.bad(`operator '${declared}' is declared but 'tailscale serve' publishes no HTTPS web handler — nothing injects an identity header, so the gate admits every tailnet caller as loopback`);
      }
      for (const [port] of tcp) {
        ctx.bad(`'tailscale serve' forwards raw TCP on ${port} — a TCP forward injects no identity header, so the gate cannot see who is calling`);
      }
      if (tcp.length === 0) {
        ctx.ok(`operator '${declared}', ${proxied.length} HTTPS web handler(s) — identity is injected and overwritten by the proxy`);
      }

      // Funnel is the internet, which PRD N3 refuses outright. Serve's own status
      // distinguishes them, and this is the one place that reads it.
      const funnel = Bun.spawnSync({ cmd: ["tailscale", "funnel", "status"], stdout: "pipe", stderr: "pipe" });
      const funnelText = funnel.stdout.toString();
      if (funnelText.includes("Funnel on")) {
        ctx.bad("'tailscale funnel' is on — PRD N3 refuses internet exposure; the identity gate covers the tailnet, not the public internet");
      }
    },
  },

  // Data freshness — the check that would have caught a nine-day outage (PRD D13).
  //
  // Every other check here verifies a DECLARATION: the manifest is well formed, the unit matches
  // it, the port is unique, the process answers /health. None of those is falsified by a producer
  // nobody is running, which is why the Feed's newest item was nine days old on 2026-08-30 while
  // doctor reported a clean machine. `feed-sweep` had been deleted; comms stayed up, healthy, and
  // empty.
  //
  // A stored path into the vault is a claim that a file exists. Nothing checked those claims
  // until 2026-09-07, and on that day three separate sets of them were found broken at once:
  // `trips` named ten `Atlas/Events/` notes its own migration had deleted, all seven
  // `finance_subscriptions.source_path` values pointed into a folder that no longer existed —
  // which silently kept finance on pattern B when Q31 says prefer C — and `scouting` carries a
  // `vault_link` column plus a whole `scouting_links` table that have never held a row.
  //
  // None of them broke a service, which is exactly why none of them surfaced. A pointer into a
  // human's notes rots without an exception: the vault is reorganised by hand, and no foreign key
  // reaches across the boundary the direction rule (PRD §5.5) deliberately keeps one-way.
  //
  // `warn`, not `bad`: every instance found so far was inert at the moment it was found. The
  // cost is a wrong branch or a dead link, not a stopped capability.
  //
  // The column list below is TYPED, and that is a known weakness worth stating rather than
  // hiding: nothing in the repo declares "this column holds a vault-relative path", so a new one
  // is invisible here until somebody adds a line. The alternative — sniffing every TEXT column
  // for something that looks like a path — would report a false positive on the first note title
  // containing a slash.
  {
    name: "Vault pointers (stored paths that must resolve)",
    run(ctx) {
      const envPath = (process.env.SJEL_DB_PATH ?? "").trim();
      const dbPath = envPath ? expandHome(envPath) : join(ctx.overlayPath, "data", "axon", "axon.db");
      if (!existsSync(dbPath)) return ctx.warn("no database — nothing to resolve");

      // The vault root is a per-capability declaration, and they are allowed to differ. Read
      // each one rather than assuming a single vault: a machine that points comms at one root
      // and trips at another is legal, and a check that assumed otherwise would blame the wrong
      // capability for a path that resolves perfectly well against its own root.
      const rootFor = (config: string, key: string): string | null => {
        const file = join(ctx.overlayPath, "config", config);
        if (!existsSync(file)) return null;
        try {
          const parsed = JSON.parse(readFileSync(file, "utf8"));
          const root = parsed?.obsidian?.[key];
          return typeof root === "string" && root.trim() !== "" ? expandHome(root) : null;
        } catch {
          return null;
        }
      };

      const sources = [
        { cap: "trips", config: "trips.json", table: "trips_plans", label: "source_ref", column: "source_ref", where: "source_kind = 'obsidian'" },
        // `json_extract(payload,'$.vault_path')` and NOT `external_id`, which this check itself
        // proved is overloaded: `item_type = 'note'` holds ten Obsidian imports whose external_id
        // IS a vault path, and two sparpreis fare-drop notes whose external_id is a synthetic key
        // (`sparpreis-drop:8000044:...`). Keying on the field that literally means "a vault path"
        // is the rule that cannot acquire a third meaning behind our backs.
        { cap: "trips", config: "trips.json", table: "trips_plan_items", label: "payload.vault_path", column: "json_extract(payload,'$.vault_path')", where: "item_type = 'note'" },
        { cap: "finance", config: "finance.json", table: "finance_subscriptions", label: "source_path", column: "source_path", where: "1=1" },
        { cap: "scouting", config: "scouting.json", table: "scouting_opportunities", label: "vault_link", column: "vault_link", where: "1=1" },
        { cap: "scouting", config: "scouting.json", table: "scouting_links", label: "vault_path", column: "vault_path", where: "1=1" },
      ];

      let checked = 0;
      let dangling = 0;
      let skipped = 0;

      for (const src of sources) {
        const root = rootFor(src.config, "root");
        if (!root) {
          skipped += 1;
          continue;
        }
        const proc = Bun.spawnSync({
          cmd: [
            "sqlite3",
            `file:${dbPath}?mode=ro`,
            `SELECT DISTINCT ${src.column} FROM ${src.table} WHERE ${src.where} AND ${src.column} IS NOT NULL AND TRIM(${src.column}) <> '';`,
          ],
          stdout: "pipe",
          stderr: "pipe",
        });
        // A table this machine's capability set never created is not a finding. sqlite3 says
        // "no such table" and that is the honest answer for a host that does not run trips.
        if (proc.exitCode !== 0) continue;

        const missing: string[] = [];
        for (const line of proc.stdout.toString().split("\n")) {
          const rel = line.trim();
          if (rel === "") continue;
          checked += 1;
          if (!existsSync(join(root, rel))) missing.push(rel);
        }
        if (missing.length > 0) {
          dangling += missing.length;
          // Three names, then a count. The whole list belongs in the capability's own tooling;
          // what doctor owes is enough to recognise WHICH set is broken.
          const shown = missing.slice(0, 3).map((m) => `'${m}'`).join(", ");
          const rest = missing.length > 3 ? ` (+${missing.length - 3} more)` : "";
          ctx.warn(`${src.cap}: ${missing.length} of ${src.table}.${src.label} point at nothing — ${shown}${rest}`);
        }
      }

      if (skipped === sources.length) return ctx.warn("no capability declares a vault root — nothing to resolve");
      if (checked === 0) ctx.ok("no stored vault pointers on this machine");
      else if (dangling === 0) ctx.ok(`${checked} stored vault pointer(s) resolve`);
    },
  },

  // Asks the capability, rather than reading its data. `GET /__axon/freshness` answers
  // `{"last_arrival_at": <epoch>}` and nothing else, so this stays a check about liveness of a
  // FLOW and never becomes a second reader of anyone's tables.
  //
  // A stopped capability is `skipped`, not failed. On-demand is the normal resting state here
  // (PRD B20) and a doctor that failed on it would fail on every capability the dashboard has not
  // opened yet — the permanent-warning failure `axon.toml [audit]` already argues about.
  {
    name: "Data freshness (declared contracts)",
    async run(ctx) {
      const proc = Bun.spawnSync({
        cmd: [join(ctx.root, "tools/capability.sh"), "registry"],
        stdout: "pipe",
        stderr: "pipe",
      });
      if (proc.exitCode !== 0) return ctx.warn("capability.sh registry failed — skipping");
      let registry: Array<Record<string, string>>;
      try {
        registry = JSON.parse(proc.stdout.toString());
      } catch {
        return ctx.warn("capability.sh registry did not return JSON — skipping");
      }

      // Same source as the capability checks above: the machine's own list, not the registry's,
      // because the registry answers for both roots and this must only speak about what runs here.
      const enabled = new Set<string>(
        Array.isArray(ctx.machineToml?.capabilities) ? ctx.machineToml.capabilities : [],
      );
      const declaring = registry.filter(
        (s) => s.freshness_stale_hours && (enabled.size === 0 || enabled.has(s.name)),
      );
      if (declaring.length === 0) {
        return ctx.ok("no capability declares a freshness contract");
      }

      for (const service of declaring) {
        // `|| 0` was wrong here and the negative test is what said so: a declared `0` is falsy,
        // so `if (stale && ...)` skipped the comparison entirely and the strictest possible
        // contract became the one that could never fail. Absence and zero are different answers,
        // so absence is NaN and every use is guarded on finiteness.
        const hours_declared = (value: string | undefined) =>
          value && value.trim() !== "" ? Number(value) : Number.NaN;
        const advise = hours_declared(service.freshness_advise_hours);
        const stale = hours_declared(service.freshness_stale_hours);
        const port = service.port;
        if (!port) {
          ctx.warn(`${service.name} declares a freshness contract but has no port to ask`);
          continue;
        }
        let body: string;
        try {
          const response = await fetch(`http://127.0.0.1:${port}/__axon/freshness`, {
            signal: AbortSignal.timeout(4000),
          });
          if (!response.ok) {
            ctx.ok(`${service.name} — skipped, not answering (HTTP ${response.status})`);
            continue;
          }
          body = await response.text();
        } catch {
          ctx.ok(`${service.name} — skipped, not running`);
          continue;
        }
        let last: number | null;
        try {
          last = JSON.parse(body).last_arrival_at ?? null;
        } catch {
          ctx.bad(`${service.name} — /__axon/freshness did not answer JSON`);
          continue;
        }
        if (last === null) {
          ctx.bad(`${service.name} — nothing has ever arrived, and a contract says something should`);
          continue;
        }
        const hours = (Date.now() / 1000 - last) / 3600;
        const age = hours < 1 ? `${Math.round(hours * 60)}m` : `${hours.toFixed(1)}h`;
        if (Number.isFinite(stale) && hours >= stale) {
          ctx.bad(`${service.name} — nothing has arrived for ${age} (stale past ${stale}h); its producer is not running`);
        } else if (Number.isFinite(advise) && hours >= advise) {
          ctx.warn(`${service.name} — last arrival ${age} ago (due past ${advise}h)`);
        } else {
          ctx.ok(`${service.name} — data arrived ${age} ago`);
        }
      }
    },
  },

  // Backup freshness, and then the archive itself.
  //
  // D10 is the argument for both halves. The vault's only backup died and stayed dead for 27 days
  // before anyone noticed, and what made it invisible was not a subtle bug — it was that nothing
  // asked. The dashboard has shown backup ages since (sjel-status' /backups), but a dashboard is
  // something you have to open; doctor is what runs before a change, and it said nothing about
  // backups at all.
  //
  // The second half exists because the first is not enough. A receipt is a claim about a past
  // moment, and the destination is a live directory that a full disk, an unmounted volume, a
  // retention rule or a cloud provider's eviction can empty afterwards. Checking the receipt alone
  // is the same instrument that reported 704 MB shipped and verified for an archive that could
  // never have been restored.
  {
    name: "Backups (receipts, and the archives they name)",
    async run(ctx) {
      if (!ctx.overlayPath || !existsSync(ctx.overlayPath)) {
        return ctx.warn("skipped — no overlay to read backup receipts from");
      }
      const proc = Bun.spawnSync({
        cmd: [join(ctx.root, "tools/capability.sh"), "registry"],
        stdout: "pipe",
        stderr: "pipe",
      });
      if (proc.exitCode !== 0) return ctx.warn("capability.sh registry failed — skipping");
      let registry: Array<Record<string, string>>;
      try {
        registry = JSON.parse(proc.stdout.toString());
      } catch {
        return ctx.warn("capability.sh registry did not return JSON — skipping");
      }

      // The machine's own enabled list, exactly as the freshness check above uses it: the registry
      // answers for both roots, and this must only speak about contracts that run here. `scope`
      // drops a capability this machine merely consumes — its data lives on the deployment that
      // provides it, and so does its backup (tools/backup-all.sh makes the same cut).
      const enabled = new Set<string>(
        Array.isArray(ctx.machineToml?.capabilities) ? ctx.machineToml.capabilities : [],
      );
      const contracts = registry.filter(
        (s) => s.backup_target && s.scope !== "external" && (enabled.size === 0 || enabled.has(s.name)),
      );
      if (contracts.length === 0) return ctx.ok("no capability on this machine declares a backup contract");

      // Destination coordinates are a fact about a deployment, so they are in the overlay. An
      // unreadable file is the overlay's finding, not this one's — say the archives could not be
      // located rather than reporting every one of them missing.
      let targets: Record<string, any> = {};
      const systemsLocal = join(ctx.overlayPath, "config", "systems.local.toml");
      if (existsSync(systemsLocal)) {
        try {
          targets = await readToml(systemsLocal);
        } catch {
          ctx.warn("overlay systems.local.toml is unreadable — archives cannot be located");
        }
      }

      const now = Math.floor(Date.now() / 1000);
      for (const service of contracts) {
        const declared = (v: string | undefined) => (v && v.trim() !== "" ? Number(v) : Number.NaN);
        const advise = declared(service.backup_advise_days);
        const stale = declared(service.backup_stale_days);
        const receiptPath = join(ctx.overlayPath, "backup", "receipts", `${service.name}.json`);
        let receipt: BackupReceipt | null = null;
        if (existsSync(receiptPath)) {
          try {
            receipt = JSON.parse(readFileSync(receiptPath, "utf8")) as BackupReceipt;
          } catch {
            // An unreadable receipt is not a fresh backup. Falling through to `never` is the
            // honest reading and matches sjel-status, which parses the same file with the same
            // "unparseable means no usable receipt" rule.
            ctx.bad(`${service.name} — ${receiptPath} is not readable JSON; treat this contract as unverified`);
            continue;
          }
        }
        const at = receipt?.completed_at ? parseReceiptTimestamp(receipt.completed_at) : null;
        // saturating: a receipt dated in the future is a clock problem, not a negative age.
        const age = at === null ? null : Math.max(0, now - at);
        const state = backupAgeState(age, advise, stale);
        const days = age === null ? 0 : (age / 86_400).toFixed(1);

        // Checked before the age switch, and independent of it: a fresh receipt and a failed
        // last attempt are exactly the combination that used to read as healthy, and the
        // `never` case below would otherwise `continue` past this.
        const attemptPath = join(
          ctx.overlayPath,
          "backup",
          "receipts",
          "attempts",
          `${service.name}.json`,
        );
        if (existsSync(attemptPath)) {
          try {
            const attempt = JSON.parse(readFileSync(attemptPath, "utf8")) as {
              exit_code?: number;
              at_epoch?: number;
              detail?: string;
            };
            if (typeof attempt.at_epoch === "number") {
              ctx.bad(
                `${service.name} — ${attemptFinding(
                  {
                    exit_code: typeof attempt.exit_code === "number" ? attempt.exit_code : -1,
                    at_epoch: attempt.at_epoch,
                    detail: typeof attempt.detail === "string" ? attempt.detail : "",
                  },
                  now,
                ).detail}`,
              );
            } else {
              ctx.warn(`${service.name} — ${attemptPath} has no timestamp; a failed attempt cannot be dated`);
            }
          } catch {
            ctx.warn(`${service.name} — ${attemptPath} is not readable JSON; whether the last attempt failed is unknown`);
          }
        }

        switch (state) {
          case "never":
            ctx.bad(`${service.name} — declares a backup contract and has no usable receipt; nothing has ever landed (tools/backup.sh ${service.name})`);
            continue;
          case "overdue":
            ctx.bad(`${service.name} — last backup ${days}d ago, past its ${stale}d stale threshold; the schedule that should refresh it is not working`);
            break;
          case "due":
            ctx.warn(`${service.name} — last backup ${days}d ago (due past ${advise}d)`);
            break;
          case "unknown":
            ctx.warn(`${service.name} — last backup ${days}d ago, and the manifest declares no cadence to judge that against`);
            break;
          default:
            ctx.ok(`${service.name} — backed up ${days}d ago`);
        }

        // Now go and look. Everything below is about the artifact, not the record of it.
        const targetId = receipt?.target ?? "";
        const tarball = receipt?.tarball ?? "";
        const bytes = typeof receipt?.bytes === "number" ? receipt.bytes : Number.NaN;
        if (!targetId || !tarball || !Number.isFinite(bytes)) {
          ctx.warn(`${service.name} — its receipt names no target/tarball/bytes, so the archive cannot be verified`);
          continue;
        }
        const target = targets[targetId];
        if (!target) {
          ctx.warn(`${service.name} — receipt names target '${targetId}', which the overlay does not declare`);
          continue;
        }
        // `ssh` is the default kind, exactly as tools/backup.sh resolves it.
        const kind = typeof target.kind === "string" && target.kind ? target.kind : "ssh";
        if (kind !== "local") {
          // Not a gap that can be closed here. Reaching a push target costs an ssh round trip and
          // an unlocked vault agent (backup.sh says so at its head), and doctor is offline by
          // contract. Reported as a skip with the reason, never passed over — "nothing to check"
          // must not read as "checked fine".
          ctx.ok(`${service.name} — archive not verified: target '${targetId}' is kind=${kind}, which needs ssh and an unlocked vault`);
          continue;
        }
        const rawPath = typeof target.path === "string" ? target.path : "";
        if (!rawPath) {
          ctx.warn(`${service.name} — target '${targetId}' is kind=local and declares no path`);
          continue;
        }
        const archive = join(expandHome(rawPath), service.name, tarball);
        // stat, never read. The archive is up to 4 GB and, at this destination, may be an evicted
        // placeholder — opening one would pull the whole thing back over the network, which is the
        // opposite of what a health check should cost.
        let exists = false;
        let size: number | null = null;
        try {
          size = statSync(archive).size;
          exists = true;
        } catch {
          exists = false;
        }
        // BSD st_flags, the only place SF_DATALESS is visible. `stat -f` is BSD-only and Node's
        // Stats does not carry st_flags at all, so this is a shell-out on Darwin and an empty
        // string everywhere else.
        //
        // Verifier, 2026-09-08: an earlier version of this comment said the empty string reads
        // as "not asked" rather than "not evicted". It does not. classifyArchiveAtTarget has no
        // such branch — `flags = ""` and `flags = "-"` both fall through to the same
        // `✓ <n> bytes, present at the destination`. So on Darwin, a `stat` that fails for any
        // reason reports an evicted archive as present, which is the silent green this whole
        // section exists to remove; the `kind != local` row above gets this right and says
        // "archive not verified" out loud. Left as it stands rather than fixed here: the fix
        // needs a fourth input on the classifier, and the branch could then only be watched
        // failing on macOS, which tools/lib/test-support.sh's skippable() refuses in CI. See
        // the verifier's report.
        let flags = "";
        if (exists && process.platform === "darwin") {
          const st = Bun.spawnSync({ cmd: ["stat", "-f", "%Sf", archive], stdout: "pipe", stderr: "pipe" });
          if (st.exitCode === 0) flags = st.stdout.toString().trim();
        }
        const verdict = classifyArchiveAtTarget({ exists, sizeBytes: size, flags, receiptBytes: bytes });
        const line = `${service.name} — ${tarball}: ${verdict.detail}`;
        if (verdict.level === "bad") ctx.bad(line);
        else if (verdict.level === "warn") ctx.warn(line);
        else ctx.ok(line);
      }
    },
  },

  // Port uniqueness across both roots.
  //
  // Added 2026-08-30, after `vault` (Axon) and `ytalbum` (the overlay) both declared 8094 while
  // doctor reported a clean machine. Each manifest justified the number in a comment as "the next
  // free port across both roots" — two surveys, each true when written, neither re-run, and
  // neither able to see the other. Nothing had collided only because both are on-demand and never
  // ran at the same moment; the second to start would have failed to bind, on a machine every
  // other check called healthy. That is the shape this file exists to catch: a declaration nobody
  // verified.
  //
  // DECLARED ports, not enabled ones. A collision that appears the moment a capability is enabled
  // is one to find before enabling it, and the registry only sees the enabled set.
  //
  // `port` and `panel_port` are collected together because they are claims on the same namespace,
  // and one capability serving its panel on its own port is normal (ytalbum did) — so duplicates
  // are counted per capability NAME, never per claim.
  {
    name: "Port uniqueness (declared, both roots)",
    async run(ctx) {
      const claims = new Map<string, Set<string>>();
      const roots = [
        { dir: join(ctx.root, "capabilities"), label: "" },
        { dir: join(ctx.overlayPath ?? "", "capabilities"), label: "overlay:" },
      ];
      for (const { dir, label } of roots) {
        if (!dir || !existsSync(dir)) continue;
        for (const cap of readdirSync(dir, { withFileTypes: true })) {
          if (!cap.isDirectory()) continue;
          const svc = join(dir, cap.name, "service.toml");
          if (!existsSync(svc)) continue;
          const parsed = await readToml(svc);
          for (const field of ["port", "panel_port"]) {
            const value = String(parsed?.[field] ?? "").trim();
            if (!value) continue;
            if (!claims.has(value)) claims.set(value, new Set());
            claims.get(value)!.add(`${label}${cap.name}`);
          }
        }
      }
      const collisions = [...claims].filter(([, owners]) => owners.size > 1);
      if (collisions.length === 0) {
        ctx.ok(`${claims.size} declared port(s) across both roots, each claimed by one capability`);
        return;
      }
      for (const [port, owners] of collisions) {
        ctx.bad(`port ${port} is declared by ${[...owners].join(" and ")} — whichever starts second cannot bind`);
      }
    },
  },

  // Packs — Claude Code deployment state.
  //
  // This used to assert that every destination was a SYMLINK into Packs/. It stopped being
  // Packs sections used to be two hardcoded entries here — Claude, then Codex — so an
  // absent harness got ~30 warnings for a directory nothing read, and pi got zero rows.
  // The registry (tools/lib/harness-registry.ts) was written for exactly that and doctor
  // was the one consumer that never adopted it; 2026-09-09 it did. Lesson that predates
  // it, preserved: deployment checks read the same ledger the deployer writes, never a
  // symlink test — eight "occupied by a non-symlink" hard failures on correct state
  // (2026-08-09) are the scar that earns that rule.
  ...packStatusSections(),

  // Decision freshness — do the entries still describe this tree? See
  // findDecisionPathRot above for why this is here and not a repo gate.
  {
    name: "Doctrine freshness (README why-blocks)",
    run(ctx) {
      try {
        // CONTRIBUTING.md#decisions-live-with-their-owner: a call's reasoning lives beside the thing it governs, under `## Why this shape:`.
        // This checks the prose still matches the tree -- every path a block names must resolve, and
        // every path it declares deliberately absent must stay absent.
        const whyBlocks: Array<{ slug: string; text: string; assertsAbsent: string[]; dir: string }> = [];
        const tracked = gitOut("ls-files").split("\n");
        for (const f of tracked) {
          if (!f.endsWith(".md") || !existsSync(join(ctx.root, f))) continue;
          whyBlocks.push(...collectWhyBlocks(f, readFileSync(join(ctx.root, f), "utf8")));
        }
        // Derived from the tree, not listed (Axon#26). The old list named three parents
        // plus tools/, which left dashboard/ and schemas/ resolving against nothing, and
        // appended a `<unit>/src/` base for every unit whether or not it had one — a
        // prefix nothing could ever resolve under. Both are the same mistake: a hand-list
        // standing in for what `git ls-files` already says.
        const bases = whyBlockBases(tracked);
        const overlay = process.env.SJEL_PERSONAL_ROOT ?? "";
        const rot = findDecisionPathRot(
          whyBlocks,
          (p) => existsSync(join(ctx.root, p)) || (overlay !== "" && existsSync(join(overlay, p))),
          bases,
        );

        // decisions/ was dissolved on 2026-07-28 (CONTRIBUTING.md#decisions-live-with-their-owner). Nothing may cite it again: an empty slug
        // set makes every `decisions/<slug>` reference a finding, which is the guard against the
        // folder quietly coming back one entry at a time.
        const dangling = findDanglingDecisionRefs(
          tracked
            .filter((f) => f && existsSync(join(ctx.root, f)) &&
              !/\.(test|spec)\.[tj]s$/.test(f) && !f.endsWith("test.sh"))
            .map((f) => ({ path: f, text: readFileSync(join(ctx.root, f), "utf8") })),
          () => false,
        );

        for (const r of rot) {
          if (r.kind === "missing") ctx.bad(`${r.slug} names ${r.path}, which no longer exists`);
          else ctx.bad(`${r.slug} declares ${r.path} absent, but it exists now`);
        }
        for (const d of dangling) ctx.bad(`${d.file} cites decisions/${d.slug}; that directory was dissolved (CONTRIBUTING.md#decisions-live-with-their-owner)`);

        if (rot.length === 0 && dangling.length === 0) {
          ctx.ok(`${whyBlocks.length} why-blocks — every named path resolves, every asserted absence holds`);
        } else {
          console.log(
            "  → repair the paths if the reasoning still holds, delete it if it is spent, or mark the\n" +
            "    absence deliberate with <!-- asserts-absent: <path> --> inside the block",
          );
        }
      } catch (error) {
        ctx.bad(`doctrine sweep failed: ${(error as Error).message}`);
      }
    },
  },

  // self.json freshness — the committed self-model vs the working tree. Stale is a warn,
  // not a fail: an out-of-date self-model is a regenerate away and never breaks a build.
  {
    name: "Self-model freshness (self.json)",
    run(ctx) {
      const selfPath = join(ctx.root, "tools", "self");
      if (!existsSync(selfPath)) {
        ctx.warn(`missing ${selfPath}`);
        return;
      }
      const proc = Bun.spawnSync({ cmd: [selfPath, "check"], stdout: "pipe", stderr: "pipe" });
      const out = [proc.stdout.toString().trim(), proc.stderr.toString().trim()]
        .filter(Boolean)
        .join("\n");
      if (out) console.log(out.split("\n").map((l) => `  ${l}`).join("\n"));
      if (proc.exitCode !== 0) ctx.warn("self.json is stale — run: tools/self generate");
    },
  },

  // Architecture-generator inputs — delegate to tools/check-generator-inputs-tracked.sh.
  // Needs `git ls-files` and the real checkout, which is what doctor already has. Same
  // delegation shape as tools/toolchain-check above: doctor reports, the script owns the rule.
  {
    name: "Architecture-generator input visibility (Axon#30)",
    run(ctx) {
      const checkPath = join(ctx.root, "tools", "check-generator-inputs-tracked.sh");
      if (!existsSync(checkPath)) {
        ctx.warn(`missing ${checkPath}`);
        return;
      }
      const proc = Bun.spawnSync({ cmd: [checkPath], stdout: "pipe", stderr: "pipe" });
      const out = [proc.stdout.toString().trim(), proc.stderr.toString().trim()]
        .filter(Boolean)
        .join("\n");
      if (out) console.log(out.split("\n").map((l) => `  ${l}`).join("\n"));
      if (proc.exitCode !== 0) {
        ctx.bad("the architecture generator reads an input others cannot see — see above");
      }
    },
  },

  // The service.toml gate CI runs, here over the overlay as well. CI has no overlay, so a
  // private manifest is held to the same rules only on the machine that runs it. Among them:
  // a port serves GET /routes or names why it cannot (ISA ISC-34).
  {
    name: "Service manifests (both roots)",
    run(ctx) {
      const checkPath = join(ctx.root, "tools", "check-service-tomls.sh");
      const proc = Bun.spawnSync({
        cmd: ["bash", checkPath],
        env: { ...process.env, SJEL_CHECK_OVERLAY: "1" },
        stdout: "pipe",
        stderr: "pipe",
      });
      const failures = proc.stderr.toString().split("\n").filter((l) => l.startsWith("FAIL"));
      for (const line of failures) ctx.bad(line.replace(/^FAIL /, ""));
      if (proc.exitCode === 0) ctx.ok("every manifest passes tools/check-service-tomls.sh, overlay included");
      else if (failures.length === 0) ctx.bad(`tools/check-service-tomls.sh exited ${proc.exitCode}`);
    },
  },

  // Public-checkout hygiene — delegate to the index scanner so Doctor and CI enforce
  // the same rule. It inspects tracked blob contents, including binary metadata, rather
  // than pretending .gitignore can remove a file that is already in the index.
  {
    name: "Publication hygiene (tracked tree)",
    run(ctx) {
      const checkPath = join(ctx.root, "tools", "check-publication-hygiene.sh");
      if (!existsSync(checkPath)) {
        ctx.warn(`missing ${checkPath}`);
        return;
      }
      const proc = Bun.spawnSync({ cmd: [checkPath], stdout: "pipe", stderr: "pipe" });
      const out = [proc.stdout.toString().trim(), proc.stderr.toString().trim()]
        .filter(Boolean)
        .join("\n");
      if (out) console.log(out.split("\n").map((l) => `  ${l}`).join("\n"));
      if (proc.exitCode !== 0) ctx.bad("tracked content is not safe for a public checkout — see above");
    },
  },

  // UI type-check coverage — delegate to tools/discover-ui-packages, the same tool the CI
  // job loops over, so "which UIs are checked" has one answer locally and in CI. Doctor
  // runs discovery only, never the checks themselves: a `bun install` per package is a
  // network operation, and doctor is the fast local sweep. What it catches is the class
  // CI cannot — a UI that would be dropped from coverage, before the commit that drops it.
  {
    name: "UI type-check coverage (Axon#139)",
    run(ctx) {
      const checkPath = join(ctx.root, "tools", "discover-ui-packages");
      if (!existsSync(checkPath)) {
        ctx.warn(`missing ${checkPath}`);
        return;
      }
      const proc = Bun.spawnSync({ cmd: [checkPath], stdout: "pipe", stderr: "pipe" });
      const out = [proc.stdout.toString().trim(), proc.stderr.toString().trim()]
        .filter(Boolean)
        .join("\n");
      if (out) console.log(out.split("\n").map((l) => `  ${l}`).join("\n"));
      if (proc.exitCode !== 0) ctx.bad("a UI package declares a surface CI cannot type-check — see above");
    },
  },

  // There was an "Upstream audit" check here until 2026-08-28. It delegated to
  // tools/upstream-checker, which PRD Q41 retired along with the rest of the homegrown
  // freshness stack; Dependabot reports drift on GitHub now (.github/dependabot.yml), which
  // — unlike the Renovate App named here until 2026-09-02 — needs nothing installed to run.
  //
  // Nothing replaces it in this file, and that is the honest outcome rather than a gap
  // worth papering over. doctor answers "is this machine healthy", offline, in a second.
  // "Is a dependency behind" needs the network and a registry per ecosystem — it was
  // always the check here that could not answer offline, and it spent most of its life
  // printing a manifest-format result under a supply-chain heading. What it did answer
  // without the network — that every entry has a verdict — is doctrine about a documentation
  // file, and CONTRIBUTING.md#dependency-verdicts-and-provenance now says plainly that a human
  // owns it.

  // There was an "Accepted-finding policies" check here until 2026-09-02. It delegated to
  // `tools/audit --expiry`, which read osv-scanner.toml's ignoreUntil dates and warned before
  // one lapsed. Q74 deleted the flag and the clock behind it: osv-scanner enforces those
  // dates itself and names a lapsed entry under "unused ignores", so the pre-warning was the
  // only thing Axon added and the only thing lost. The notice now arrives on the day, from
  // the scanner, on the next push or the next weekly security.yml run.

  // Host patch — did the daily upgrade job run. A launchd StartInterval unit does not fire on
  // a sleeping Mac, so "scheduled" and "ran" are different questions and only the receipt
  // answers the second. tools/host-patch.sh writes it; doctor reports. Same delegation shape
  // as the toolchain check above.
  {
    name: "Host patch (capabilities/host-patch)",
    run(ctx) {
      const enabled: string[] = Array.isArray(ctx.machineToml?.capabilities) ? ctx.machineToml.capabilities : [];
      if (!enabled.includes("host-patch")) {
        ctx.ok("host-patch not enabled on this machine — nothing to report");
        return;
      }
      const receipt = join(ctx.overlayPath, "data", "host-patch", "last.json");
      if (!existsSync(receipt)) {
        ctx.warn("host-patch is enabled but has never written a receipt — it has not run");
        return;
      }
      let r: any;
      try {
        r = JSON.parse(readFileSync(receipt, "utf8"));
      } catch {
        ctx.bad("<overlay>/data/host-patch/last.json is not valid JSON — the last run could not record what it did");
        return;
      }
      const ageH = (Date.now() - Date.parse(r.at)) / 3_600_000;
      if (!Number.isFinite(ageH)) {
        ctx.bad("the host-patch receipt carries no readable timestamp");
        return;
      }
      if (ageH > 48) ctx.warn(`last patch run was ${Math.round(ageH)}h ago — a 24h job that has not run in two days is not running`);
      if (r.failed) ctx.warn(`last patch run had failed steps:${r.failed}`);
      if (r.audit === "finding") ctx.bad("the last patch run's audit found something — run tools/audit");
      else if (r.audit === "scanner-missing") ctx.warn("the last patch run's audit could not run a scanner");
      else if (ageH <= 48 && !r.failed) ctx.ok(`patched ${Math.round(ageH)}h ago, audit clean`);
    },
  },

  // Container refresh — the same question as the host patch above, asked of the images. Q77
  // (2026-09-02) made every service.toml tag a rolling channel, and a channel that nothing pulls
  // is a version literal with extra steps. tools/container-refresh.sh writes the receipt; doctor
  // reports it. Gated on the capability being enabled, because a workstation runs no containers
  // and a warning there would teach people to skim past this line.
  {
    name: "Container refresh (capabilities/container-refresh)",
    run(ctx) {
      const enabled: string[] = Array.isArray(ctx.machineToml?.capabilities) ? ctx.machineToml.capabilities : [];
      if (!enabled.includes("container-refresh")) {
        ctx.ok("container-refresh not enabled on this machine — nothing to report");
        return;
      }
      const receipt = join(ctx.overlayPath, "data", "container-refresh", "last.json");
      if (!existsSync(receipt)) {
        ctx.warn("container-refresh is enabled but has never written a receipt — it has not run");
        return;
      }
      let r: any;
      try {
        r = JSON.parse(readFileSync(receipt, "utf8"));
      } catch {
        ctx.bad("<overlay>/data/container-refresh/last.json is not valid JSON — the last run could not record what it did");
        return;
      }
      const ageH = (Date.now() - Date.parse(r.at)) / 3_600_000;
      if (!Number.isFinite(ageH)) {
        ctx.bad("the container-refresh receipt carries no readable timestamp");
        return;
      }
      if (ageH > 48) ctx.warn(`last image refresh was ${Math.round(ageH)}h ago — a 24h job that has not run in two days is not running`);
      if (r.failed) ctx.warn(`last image refresh had failed steps:${r.failed}`);
      // "Nothing to refresh" is a legitimate outcome and reads as one, so a host that enables the
      // capability and declares no image is not reported as healthy-by-accident.
      else if (ageH <= 48 && String(r.skipped ?? "").includes("no-container-capabilities")) {
        ctx.ok(`checked ${Math.round(ageH)}h ago — no enabled capability declares an image`);
      } else if (ageH <= 48) {
        ctx.ok(`images refreshed ${Math.round(ageH)}h ago${r.ran ? ` (recreated:${r.ran})` : ", none moved"}`);
      }
    },
  },

  // Build artifacts — PRD §9's R6, the resource rule §9 had no check for until now. Q53
  // (2026-08-28) ratified it and named tools/doctor as the checker; it stayed unimplemented
  // until 2026-09-03, when tools/storage grew the `target` verb that can answer it.
  //
  // Delegated, not reimplemented, exactly as the toolchain check above is: sjel-storage
  // owns the walk, the buckets, the ratio and the toolchain comparison, and doctor reads
  // its verdict. A warn rather than a bad, for the reason Q53 itself gives about gates that
  // fire when nothing is wrong: a checkout mid-refactor legitimately carries a debug tree
  // that no release build matches, and the tool reports the unit counts that show it.
  //
  // The binary is used only if it is already built. Building it here would make the fast
  // local sweep pay for a release compile, which is the same reason the UI section below
  // runs discovery and never the checks.
  {
    name: "Build artifacts (PRD §9 R6)",
    run(ctx) {
      const launcher = join(ctx.root, "tools", "storage", "storage");
      if (!existsSync(launcher)) {
        ctx.warn(`missing ${launcher}`);
        return;
      }
      const targetDir = process.env.CARGO_TARGET_DIR || join(ctx.root, "target");
      const bin = join(targetDir, "release", "sjel-storage");
      if (!existsSync(bin)) {
        ctx.warn("sjel-storage not built — run `sjel storage target` to check R6");
        return;
      }
      const proc = Bun.spawnSync({ cmd: [bin, "target", "--json"], stdout: "pipe", stderr: "pipe" });
      let data: any;
      try {
        data = JSON.parse(proc.stdout.toString());
      } catch {
        ctx.warn("sjel-storage target did not emit JSON — run `sjel storage target` for detail");
        return;
      }
      const gb = (b: number) => `${(b / 1024 ** 3).toFixed(1)} GB`;
      const units = (name: string) =>
        (data.profiles ?? []).find((p: any) => p.name === name)?.units ?? 0;

      if (data.ratio === null || data.ratio === undefined) {
        ctx.warn(`${gb(data.bytes ?? 0)} in ${data.target_dir} — no release build, so R6 has no control`);
      } else if (data.r6 === "over") {
        // Say the unit counts in the same line as the ratio. R6's control is "same crates,
        // same machine, same moment" (Q53), and a mismatch there is the difference between
        // a real finding and two builds that were never comparable.
        ctx.warn(
          `target/debug is ${data.ratio.toFixed(1)}× target/release, over R6's ${data.r6_max_ratio}× ` +
            `(${units("debug")} vs ${units("release")} units) — sjel storage prune --incremental, or ` +
            `build both profiles and re-check`,
        );
      } else {
        ctx.ok(`target/debug is ${data.ratio.toFixed(1)}× target/release, within R6's ${data.r6_max_ratio}× (${gb(data.bytes ?? 0)} total)`);
      }

      // The rot no ratio detects: both profiles carry it equally. On 2026-09-03 this
      // workspace's target dir held 21 GB, most of it output from rustc versions no longer
      // installed, and a clean rebuild of the same tree was 4.7 GB.
      const tc = data.toolchain ?? {};
      if (tc.matches === false) {
        ctx.warn(
          `target/.rustc_info.json records rustc ${String(tc.recorded).slice(0, 9)} but this machine runs ` +
            `${String(tc.current).slice(0, 9)} — ${gb(tc.stale_candidate_bytes ?? 0)} of deps and fingerprints ` +
            `was built by a compiler that is gone; sjel storage prune --target`,
        );
      } else if (tc.recorded) {
        // "last recorded", not "clean": cargo rewrites .rustc_info.json on its first run
        // under a new compiler and leaves the older generation's output in deps/, so a
        // match means cargo has run since the last roll and nothing stronger.
        ctx.ok(`cargo last recorded rustc ${String(tc.recorded).slice(0, 9)}, which is the one installed`);
      }
    },
  },

  // Repo freshness — is this checkout, on any deployment host, behind origin/main. `--online`
  // fetches first for a live answer; offline reads whatever origin/main ref
  // was last fetched, best-effort. This is the only check left that the flag makes
  // network-dependent apart from the systems probe above — the upstream audit was the
  // third until 2026-08-28. Being behind isn't broken — `warn`, not
  // `bad` — but it's the one thing nothing previously surfaced at all: a
  // multi-host Axon deployments otherwise have no way to
  // tell a stale checkout from a current one short of eyeballing `git log`.
  {
    name: "Repo freshness (origin/main)",
    run(ctx) {
      if (ctx.online) {
        Bun.spawnSync({ cmd: ["git", "-C", ctx.root, "fetch", "--quiet", "origin", "main"], stdout: "pipe", stderr: "pipe" });
      }
      const revList = Bun.spawnSync({
        cmd: ["git", "-C", ctx.root, "rev-list", "--left-right", "--count", "HEAD...origin/main"],
        stdout: "pipe",
        stderr: "pipe",
      });
      if (revList.exitCode !== 0) {
        ctx.warn("no origin/main ref cached — run 'tools/doctor --online' (or 'git fetch') to check freshness");
      } else {
        const [aheadStr, behindStr] = revList.stdout.toString().trim().split(/\s+/);
        const ahead = Number(aheadStr) || 0;
        const behind = Number(behindStr) || 0;
        if (ahead === 0 && behind === 0) ctx.ok("up to date with origin/main");
        else if (behind > 0 && ahead === 0) ctx.warn(`${behind} commit(s) behind origin/main — run tools/update.sh`);
        else if (ahead > 0 && behind === 0) ctx.ok(`${ahead} commit(s) ahead of origin/main — push when ready`);
        else ctx.warn(`diverged from origin/main (${ahead} ahead, ${behind} behind) — merge before tools/update.sh`);
      }
    },
  },

  // Session orientation — the dynamic, always-current answer to "what's
  // the state of this checkout right now," replacing a hand-maintained status
  // doc (CONTRIBUTING.md#documentation-stays-owned-and-current forbids adding one: nothing executable
  // would reference it). Point of this section: a fresh agent session in this repo runs
  // `tools/doctor` first and gets branch/HEAD/dirty-file-count for free,
  // instead of a static file someone has to remember to update.
  {
    name: "Session orientation",
    run(ctx) {
      ctx.ok(`version: ${formatVersion(gitOut("describe", "--tags", "--always", "--dirty", "--match", RELEASE_TAG_GLOB), gitOut("log", "-1", "--format=%cs"))}`);
      const branchProc = Bun.spawnSync({ cmd: ["git", "-C", ctx.root, "branch", "--show-current"], stdout: "pipe" });
      const branch = branchProc.stdout.toString().trim() || "(detached HEAD)";
      const headProc = Bun.spawnSync({ cmd: ["git", "-C", ctx.root, "log", "-1", "--format=%h %s (%cr)"], stdout: "pipe" });
      ctx.ok(`${branch} @ ${headProc.stdout.toString().trim()}`);
      const dirtyProc = Bun.spawnSync({ cmd: ["git", "-C", ctx.root, "status", "--porcelain"], stdout: "pipe" });
      const dirtyCount = dirtyProc.stdout.toString().split("\n").filter(Boolean).length;
      if (dirtyCount === 0) ctx.ok("working tree clean");
      else ctx.warn(`${dirtyCount} uncommitted change(s) — git status for detail`);
      // The backlog is ISAs (CONTRIBUTING.md#the-backlog-is-isas), so this counts unchecked claims
      // across every tracked ISA.md rather than naming a tracker. Derived, not remembered: a
      // hand-maintained number here would be the status doc this section exists to replace.
      // `--others --exclude-standard` so an ISA written this session counts before it is
      // committed; without it the number reads 0 while the file sits in front of you.
      const isaFiles = gitOut("ls-files", "--cached", "--others", "--exclude-standard",
        "ISA.md", "*/ISA.md", "*/*/ISA.md", "*/*/*/ISA.md")
        .split("\n")
        .filter(Boolean);
      const openClaims = isaFiles.reduce((sum, f) => {
        try {
          return sum + (readFileSync(join(ctx.root, f), "utf8").match(/^- \[ \] /gm)?.length ?? 0);
        } catch {
          return sum;
        }
      }, 0);
      ctx.ok(`open backlog: ${openClaims} claim(s) across ${isaFiles.length} ISA(s) · doctrine: CONTRIBUTING.md`);
    },
  },
];

async function main() {
  if (process.argv.includes("-h") || process.argv.includes("--help")) {
    console.log(HELP);
    process.exit(0);
  }

  // --version: version identity only, skip the full check run entirely.
  if (process.argv.includes("--version")) {
    printVersion(process.argv.includes("--online"));
    process.exit(0);
  }

  let failed = 0;
  const ctx: CheckContext = {
    root: SJEL_ROOT,
    overlayPath: "",
    machineToml: {},
    mounts: [],
    systemsToml: {},
    online: process.argv.includes("--online"),
    ok: (msg) => { console.log(`  ✓ ${msg}`); },
    bad: (msg) => { console.log(`  ✗ ${msg}`); failed++; },
    warn: (msg) => { console.log(`  ⚠ ${msg}`); },
  };

  console.log(`Axon doctor · ${SJEL_ROOT}`);
  for (const check of CHECKS) {
    console.log(`\n${check.name}`);
    await check.run(ctx);
  }

  console.log();
  if (failed === 0) {
    console.log("doctor: all checks passed");
    process.exit(0);
  } else {
    console.log(`doctor: ${failed} check(s) failed`);
    process.exit(1);
  }
}

if (import.meta.main) await main();
