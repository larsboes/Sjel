#!/usr/bin/env bun
// repos.ts — version identity for the two repos this installation is made of.
//
// Axon is public-safe and its overlay is private, so "which Axon am I running" is
// really two answers that have to be read separately and shown together. Both repos
// carry zero tags today; `git describe` degrades to a short sha, and that is a fact
// worth rendering rather than an error worth hiding.
//
// Read-only on purpose. Nothing here creates, moves or pushes a tag: a browser button that
// writes to git needs a gate in front of it, and tags are cut by tools/release under its own
// clean/on-main/not-behind/doctor gate (README.md#cutting-and-consuming-a-release).
//
// The remote is normalised to an https URL because its only consumer is an <a href>.
// An ssh remote (git@github.com:owner/repo.git) is not a URL a browser can follow.
//
// usage: tools/repos [--json]

import { basename } from "node:path";
import { releaseTagGlob } from "./lib/release.ts";

interface RepoStatus {
  /** Display name — the directory the repo lives in, never its absolute path. */
  name: string;
  role: "spine" | "overlay";
  /** Browsable https URL, or null when the remote is missing or not http(s)-expressible. */
  remote_url: string | null;
  branch: string | null;
  /** `git describe` over the release line — a release tag when one is reachable, else a short sha. */
  describe: string | null;
  /** The most recent reachable RELEASE tag, or null when the repo carries none. */
  tag: string | null;
  /** Commits since `tag`; null when there is no tag to count from. */
  commits_since_tag: number | null;
  /** Relative to the tracked upstream. null when the branch tracks nothing. */
  ahead: number | null;
  behind: number | null;
  dirty: boolean;
  last_commit_date: string | null;
  /** Non-null when the directory could not be read as a git repo at all. */
  error: string | null;
}

function git(cwd: string, ...args: string[]): string | null {
  const out = Bun.spawnSync(["git", "-C", cwd, ...args], { stderr: "ignore" });
  if (out.exitCode !== 0) return null;
  const text = out.stdout.toString().trim();
  return text === "" ? null : text;
}

/**
 * Every remote form git accepts, as one browsable https URL.
 *
 * scp-style (git@host:owner/repo.git) is the one that matters here — it is what `gh
 * repo clone` leaves behind and it is not a URL. A remote this cannot express becomes
 * null rather than a guess: a wrong link is worse than an absent one.
 */
function browsableRemote(raw: string | null): string | null {
  if (!raw) return null;
  const trimmed = raw.replace(/\.git$/, "");
  const scp = /^(?:ssh:\/\/)?(?:[^@/]+@)?([^:/]+):(.+)$/.exec(trimmed);
  if (scp && !trimmed.startsWith("http")) return `https://${scp[1]}/${scp[2]}`;
  if (trimmed.startsWith("https://") || trimmed.startsWith("http://")) return trimmed;
  return null;
}

// releaseGlob is passed in rather than read here: it comes from the spine's axon.toml, and this
// function also runs against the overlay, which has no release line of its own to consult.
function read(path: string, role: RepoStatus["role"], releaseGlob: string): RepoStatus {
  const base: RepoStatus = {
    name: basename(path),
    role,
    remote_url: null,
    branch: null,
    describe: null,
    tag: null,
    commits_since_tag: null,
    ahead: null,
    behind: null,
    dirty: false,
    last_commit_date: null,
    error: null,
  };

  if (git(path, "rev-parse", "--git-dir") === null) {
    return { ...base, error: `not a git repository: ${basename(path)}` };
  }

  const tag = git(path, "describe", "--tags", "--abbrev=0", "--match", releaseGlob);
  // --count needs both ends; with no tag there is nothing to count from, and reporting
  // "all commits ever" as commits-since-tag would read like a release is overdue.
  const commitsSinceTag =
    tag === null ? null : Number(git(path, "rev-list", "--count", `${tag}..HEAD`) ?? "0");

  // One `git status -sb --porcelain` would conflate the two: ahead/behind only exists
  // when the branch tracks something, and a detached HEAD has neither.
  const tracking = git(path, "rev-parse", "--abbrev-ref", "@{upstream}");
  let ahead: number | null = null;
  let behind: number | null = null;
  if (tracking) {
    const counts = git(path, "rev-list", "--left-right", "--count", `${tracking}...HEAD`);
    if (counts) {
      const [b, a] = counts.split(/\s+/).map(Number);
      behind = b ?? null;
      ahead = a ?? null;
    }
  }

  return {
    ...base,
    remote_url: browsableRemote(git(path, "remote", "get-url", "origin")),
    branch: git(path, "rev-parse", "--abbrev-ref", "HEAD"),
    describe: git(path, "describe", "--tags", "--always", "--dirty", "--match", releaseGlob),
    tag,
    commits_since_tag: commitsSinceTag,
    ahead,
    behind,
    dirty: git(path, "status", "--porcelain") !== null,
    last_commit_date: git(path, "log", "-1", "--format=%cI"),
  };
}

const axonRoot = process.env.SJEL_ROOT;
if (!axonRoot) {
  console.error("repos.ts: SJEL_ROOT is not set — run through tools/repos, or export it");
  process.exit(1);
}
// The overlay is optional: a fresh public clone has none, and that is a valid machine,
// not an error. It is also private, so only its basename is ever reported.
const overlayRoot = process.env.SJEL_PERSONAL_ROOT;

// One home for "which tags are release tags" — axon.toml [release] tag_glob, shared with
// tools/lib/version.sh and tools/doctor.ts (README.md#the-release-line).
const releaseGlob = releaseTagGlob(axonRoot);

const repos = [read(axonRoot, "spine", releaseGlob)];
if (overlayRoot && (await Bun.file(`${overlayRoot}/.git/HEAD`).exists())) {
  repos.push(read(overlayRoot, "overlay", releaseGlob));
}

if (process.argv.includes("--json") || !process.stdout.isTTY) {
  console.log(JSON.stringify({ repos }, null, 2));
} else {
  for (const r of repos) {
    if (r.error) {
      console.log(`${r.name} (${r.role}): ${r.error}`);
      continue;
    }
    const since = r.tag ? `${r.tag} +${r.commits_since_tag}` : `untagged (${r.describe})`;
    const sync = r.ahead === null ? "no upstream" : `↑${r.ahead} ↓${r.behind}`;
    console.log(
      `${r.name} (${r.role}): ${since} · ${r.branch} · ${sync}${r.dirty ? " · dirty" : ""}`,
    );
    if (r.remote_url) console.log(`  ${r.remote_url}`);
  }
}
