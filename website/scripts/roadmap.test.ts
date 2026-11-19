import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { build, renderRoadmap } from "./roadmap.ts";
import { type Work } from "../../tools/lib/work.ts";

const roots: string[] = [];
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
function empty(): Work { return { tasks: [], outcomes: [], releases: [], receipts: [] }; }
function work(state: "backlog" | "active" | "verify" | "done" | "cancelled"): Work {
  const graph = empty();
  graph.tasks.push({ id: "task-alpha", title: "Find notes", state, area: "storage", kind: "feature", priority: 1, depends_on: [], evidence: [], path: "docs/tasks/alpha.md", body: "" });
  graph.outcomes.push({ id: "notes-in-finder", title: "Notes in Finder", summary: "Find notes by title and open the source file.", order: 1, tasks: ["task-alpha"], path: "docs/roadmap/notes.md", body: "" });
  return graph;
}

describe("roadmap", () => {
  test("empty state does not invent launched features", () => {
    const html = renderRoadmap(empty()); expect(html).toContain("No upcoming outcomes"); expect(html).not.toContain("Initial Milestones");
  });
  test.each([["backlog", "Planned"], ["active", "Active"], ["verify", "Active"], ["done", "Awaiting release"]] as const)("%s maps to %s", (state, heading) => {
    const html = renderRoadmap(work(state)); expect(html).toContain(`<h2>${heading}</h2>`); expect(html).toContain("Notes in Finder"); expect(html).toContain("../docs/roadmap/notes.html");
  });
  test("cancelled outcomes do not appear", () => { expect(renderRoadmap(work("cancelled"))).not.toContain("Notes in Finder"); });
  test("escapes text without adding external scripts", () => {
    const graph = work("backlog"); graph.outcomes[0].title = '<script>alert("x")</script>';
    const html = renderRoadmap(graph); expect(html).not.toContain("<script>"); expect(html).toContain("&lt;script&gt;");
  });
  test("build writes static upcoming, delivered, release and task pages", () => {
    const root = mkdtempSync(join(tmpdir(), "sjel-roadmap-test-")); roots.push(root);
    const out = join(root, "site"); build(out, root);
    expect(readFileSync(join(out, "roadmap/index.html"), "utf8")).toContain("Roadmap — Sjel");
    expect(readFileSync(join(out, "roadmap/delivered/index.html"), "utf8")).toContain("No published releases");
    expect(readFileSync(join(out, "releases/index.html"), "utf8")).toContain("No release records");
    expect(readFileSync(join(out, "docs/tasks/index.html"), "utf8")).toContain("Open tasks");
  });
  test("a draft release never claims delivery", () => {
    const root = mkdtempSync(join(tmpdir(), "sjel-roadmap-test-")); roots.push(root);
    for (const dir of ["tasks", "releases"]) mkdirSync(join(root, "docs", dir), { recursive: true });
    writeFileSync(join(root, "docs/tasks/alpha.md"), "---\nid: task-alpha\ntitle: Find notes\nstate: verify\narea: storage\nkind: feature\npriority: 1\n---\n# Find notes\n\n## Outcome\nFind originals.\n\n## Acceptance\n- Search opens the original.\n");
    writeFileSync(join(root, "docs/releases/v0.0.3.md"), "---\nversion: v0.0.3\ntitle: Notes preview\ntasks: [task-alpha]\nrequired_assets: [sjel.tar.gz]\n---\n# Notes preview\n");
    build(join(root, "site"), root);
    const html = readFileSync(join(root, "site/releases/v0.0.3/index.html"), "utf8"); expect(html).toContain("Not published"); expect(html).toContain("acceptance pending"); expect(html).not.toContain("Download release");
  });
});
