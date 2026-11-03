import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { build, renderRoadmap } from "./roadmap.ts";

describe("roadmap", () => {
  test("renders active, planned and launched sections with cards", () => {
    const html = renderRoadmap();
    expect(html).toContain("Active");
    expect(html).toContain("Planned");
    expect(html).toContain("Launched");
    expect(html).toContain("GPUI Desktop &amp; Mobile Port");
    expect(html).toContain("Agent Containment");
    expect(html).toContain("Initial Milestones");
  });

  test("builds roadmap HTML without external scripts or leaking secrets", async () => {
    const dir = mkdtempSync(join(tmpdir(), "sjel-roadmap-"));
    build(dir);
    const content = await Bun.file(join(dir, "roadmap/index.html")).text();
    expect(content).not.toContain("<script");
    expect(content).toContain("Roadmap — Sjel");
    expect(content).toContain('class="roadmap-badge"');
  });
});
