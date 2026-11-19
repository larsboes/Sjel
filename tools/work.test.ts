import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { inventory, loadWork, outcomeState, validateReceipt } from "./lib/work.ts";
import { checkRelease, publication, run } from "./work.ts";

const roots: string[] = [];
function fixture(): string {
  const root = mkdtempSync(join(tmpdir(), "sjel-work-test-")); roots.push(root); return root;
}
function put(root: string, path: string, source: string): void {
  mkdirSync(join(root, path, ".."), { recursive: true }); writeFileSync(join(root, path), source);
}
export function task(id = "task-alpha", extra = "", state = "backlog"): string {
  return `---\nid: ${id}\ntitle: Find original notes\nstate: ${state}\narea: storage\nkind: feature\npriority: 1\n${extra}---\n\n# Find original notes\n\n## Outcome\n\nFind notes without creating another copy.\n\n## Acceptance\n\n- A title search returns the fixture's original file.\n`;
}
export function outcome(): string {
  return `---\nid: notes-in-finder\ntitle: Notes in Finder\nsummary: Find notes by title and open their source files.\norder: 1\ntasks: [task-alpha]\n---\n\n# Notes in Finder\n`;
}
function git(root: string, ...args: string[]): string {
  const result = Bun.spawnSync(["git", ...args], { cwd: root, env: { ...process.env, GIT_AUTHOR_NAME: "Test", GIT_AUTHOR_EMAIL: "test@example.invalid", GIT_COMMITTER_NAME: "Test", GIT_COMMITTER_EMAIL: "test@example.invalid" } });
  if (result.exitCode) throw new Error(result.stderr.toString()); return result.stdout.toString().trim();
}
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });

describe("canonical work graph", () => {
  test("empty graph and CLI have honest empty states", async () => {
    const root = fixture(); expect(loadWork(root).tasks).toEqual([]); expect(await run(["list"], root)).toBe("No open tasks.");
  });
  test("area and state filters compose for text and JSON", async () => {
    const root = fixture();
    put(root, "docs/tasks/a.md", task("task-alpha", "", "verify"));
    put(root, "docs/tasks/b.md", task("task-beta", "", "verify").replace("area: storage", "area: gpui"));
    put(root, "docs/tasks/c.md", task("task-gamma", "evidence: [Fixture passed]\n", "done"));
    const selected = JSON.parse(await run(["list", "--area", "storage", "--state", "verify", "--json"], root));
    expect(selected.map((record: { id: string }) => record.id)).toEqual(["task-alpha"]);
    expect(await run(["list", "--area", "storage"], root)).not.toContain("task-gamma");
    expect(await run(["list", "--area", "storage", "--all"], root)).toContain("task-gamma");
    expect(await run(["list", "--state", "done"], root)).toContain("task-gamma");
    expect(await run(["list", "--area", "gpui", "--state", "done"], root)).toBe("No matching tasks.");
    await expect(run(["list", "--area", "missing"], root)).rejects.toThrow("Unknown area");
    await expect(run(["list", "--state", "missing"], root)).rejects.toThrow("Unknown state");
  });
  test("reads YAML and derives roadmap state", () => {
    const root = fixture(); put(root, "docs/tasks/alpha.md", task()); put(root, "docs/roadmap/notes.md", outcome());
    const work = loadWork(root); expect(work.tasks[0].body).toStartWith("# Find"); expect(outcomeState(work, work.outcomes[0])).toBe("planned");
  });
  test.each([
    ["state: nonsense", "state: backlog", "unknown state"],
    ["area: mystery", "area: storage", "unknown area"],
    ["kind: random", "kind: feature", "unknown kind"],
    ["priority: 9", "priority: 1", "priority"],
    ["priority: first", "priority: 1", "integer"],
    ["state: active", "state: backlog", "assignee"],
    ["state: done", "state: backlog", "evidence"],
    ["state: cancelled", "state: backlog", "reason"],
    ["## Checks", "## Acceptance", "Acceptance"],
  ])("rejects invalid record %s", (replacement, original, error) => {
    const root = fixture(); put(root, "docs/tasks/alpha.md", task().replace(original, replacement)); expect(() => loadWork(root)).toThrow(error);
  });
  test("rejects unknown keys and duplicate IDs", () => {
    const root = fixture(); put(root, "docs/tasks/a.md", task("task-alpha", "status: ready\n")); expect(() => loadWork(root)).toThrow("unknown field");
    put(root, "docs/tasks/a.md", task()); put(root, "docs/tasks/b.md", task()); expect(() => loadWork(root)).toThrow("duplicate record");
  });
  test("requires real references and rejects cycles", () => {
    const root = fixture(); put(root, "docs/tasks/a.md", task("task-alpha", "depends_on: [task-beta]\n")); expect(() => loadWork(root)).toThrow("missing dependency");
    put(root, "docs/tasks/b.md", task("task-beta", "depends_on: [task-alpha]\n")); expect(() => loadWork(root)).toThrow("cycle");
  });
  test("ready work cannot have unfinished dependencies", () => {
    const root = fixture(); put(root, "docs/tasks/a.md", task("task-alpha", "depends_on: [task-beta]\n", "ready")); put(root, "docs/tasks/b.md", task("task-beta")); expect(() => loadWork(root)).toThrow("unfinished");
  });
  test("blocked verification remains open, accepted work is not launched", async () => {
    const root = fixture(); put(root, "docs/tasks/a.md", task("task-alpha", "blocked_by: Physical device unavailable\n", "verify")); put(root, "docs/roadmap/notes.md", outcome());
    let work = loadWork(root); expect(outcomeState(work, work.outcomes[0])).toBe("active"); expect(await run(["list"], root)).toContain("blocked");
    put(root, "docs/tasks/a.md", task("task-alpha", "evidence: [Fixture search passed]\n", "done")); work = loadWork(root);
    expect(outcomeState(work, work.outcomes[0])).toBe("awaiting-release"); expect(await run(["list"], root)).toBe("No open tasks."); expect(await run(["list", "--all"], root)).toContain("task-alpha");
  });
  test("new creates distinct draft records without overwriting", async () => {
    const root = fixture(); await run(["new", "storage", "Find notes"], root); await run(["new", "storage", "Find notes"], root);
    const work = loadWork(root); expect(work.tasks.length).toBe(2); expect(work.tasks[0].id).not.toBe(work.tasks[1].id);
    expect(work.tasks.every(t => t.state === "backlog")).toBe(true);
  });
  test("inventory includes headings, ignores fenced headings and never writes", () => {
    const root = fixture(); put(root, "PLAN.md", "# Plan\n\n## Open\n\n```sh\n# not a heading\n```\n");
    const sections = inventory(root); expect(sections.map(s => s.heading)).toEqual(["Plan", "Open"]); expect(sections[1].text).toContain("not a heading");
  });
  test("published receipt requires canonical identity and assets", () => {
    expect(() => validateReceipt({ version: "v0.0.3", commit: "a".repeat(40), url: "https://example.invalid", published_at: "yesterday", assets: [] })).toThrow("receipt");
  });
  test("release scope checks acceptance in the tagged source, not the working tree", () => {
    const root = fixture(); put(root, "docs/tasks/a.md", task());
    put(root, "docs/releases/v0.0.3.md", "---\nversion: v0.0.3\ntitle: Notes preview\ntasks: [task-alpha]\nrequired_assets: [sjel.tar.gz]\n---\n\n# Notes preview\n");
    git(root, "init", "-q"); git(root, "add", "."); git(root, "-c", "commit.gpgsign=false", "commit", "-qm", "fixture");
    put(root, "docs/tasks/a.md", task("task-alpha", "evidence: [Fixture search passed]\n", "done"));
    expect(() => checkRelease(root, "v0.0.3")).toThrow("has not passed acceptance");
    git(root, "add", "."); git(root, "-c", "commit.gpgsign=false", "commit", "-qm", "accepted");
    expect(checkRelease(root, "v0.0.3")).toMatch(/^[0-9a-f]{40}$/);
  });
  test("a published release can be recorded, checked and reconciled without rewriting its receipt", async () => {
    const root = fixture(); put(root, "docs/tasks/a.md", task("task-alpha", "evidence: [Fixture search passed]\n", "done"));
    put(root, "docs/releases/v0.0.3.md", "---\nversion: v0.0.3\ntitle: Notes preview\ntasks: [task-alpha]\nrequired_assets: [sjel.tar.gz]\n---\n# Notes preview\n");
    git(root, "init", "-q"); git(root, "add", "."); git(root, "-c", "commit.gpgsign=false", "commit", "-qm", "accepted");
    git(root, "-c", "tag.gpgsign=false", "tag", "v0.0.3"); const commit = git(root, "rev-parse", "HEAD");
    const remote = { draft: false, tag_name: "v0.0.3", published_at: "2026-11-18T12:00:00Z", html_url: "https://github.com/larsboes/Sjel/releases/tag/v0.0.3", assets: [{ name: "sjel.tar.gz", size: 123, browser_download_url: "https://github.com/larsboes/Sjel/releases/download/v0.0.3/sjel.tar.gz" }] };
    const request = (_root: string, args: string[]) => args[1].includes("git/ref") ? { object: { type: "commit", sha: commit } } : remote;
    expect(await run(["receipt", "v0.0.3"], root, request)).toContain("Recorded v0.0.3");
    const path = join(root, "docs/releases/receipts/v0.0.3.json");
    const saved = readFileSync(path, "utf8");
    expect(JSON.parse(saved).commit).toBe(commit);
    expect(await run(["check"], root)).toContain("1 publication receipts");
    expect(await run(["reconcile", "v0.0.3"], root, request)).toContain("publication matches its saved receipt");
    remote.assets[0].size++;
    await expect(run(["reconcile", "v0.0.3"], root, request)).rejects.toThrow("publication differs from saved receipt");
    expect(readFileSync(path, "utf8")).toBe(saved);
    remote.draft = true; expect(() => publication(root, "v0.0.3", request)).toThrow("not a published release"); remote.draft = false;
    remote.assets = []; expect(() => publication(root, "v0.0.3", request)).toThrow("receipt");
    expect(() => publication(root, "v0.0.3", (_root, args) => args[1].includes("git/ref") ? { object: { type: "commit", sha: "b".repeat(40) } } : remote)).toThrow("identity differ");
  });
  test("bad command or unknown record fails instead of silently listing work", async () => {
    const root = fixture(); await expect(run(["publish"], root)).rejects.toThrow("tools/work"); await expect(run(["show", "missing"], root)).rejects.toThrow("unknown record");
  });
});
