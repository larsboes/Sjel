// Tests for tools/generate-site.ts (#14).
//
// The acceptance criteria this file exists to hold are properties of the OUTPUT, not of the code:
// nothing overlay-scoped may appear, nothing may request another host, and no rendered fact may
// come from an input a fresh clone cannot reproduce. Each of those can regress silently through a
// one-line change to a template, so each gets an assertion against a rendered page.

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { renderSite } from "./generate-site.ts";

const SJEL_ROOT = join(import.meta.dir, "..");

const MODEL = {
  schema: 1,
  generator: "tools/self.ts",
  units: [
    { name: "axon-status", kind: "capability" as const, service: { kind: "process", requires: ["store"], port: "8082" } },
    { name: "vaultwarden", kind: "capability" as const, service: { kind: "container", requires: [], image: "vaultwarden/server" } },
    { name: "axon-config", kind: "lib" as const },
    { name: "writing", kind: "pack" as const },
    { name: "tools", kind: "spine" as const },
  ],
  coupling: [{ from: "axon-status", to: "axon-config", kinds: ["cargo-dep"], evidence: ["capabilities/axon-status/Cargo.toml"] }],
  upstreams: [
    { name: "bun", verdict: "adopt" },
    { name: "stop-slop", verdict: "reject" },
    { name: "pascal-editor", verdict: "inspiration" },
  ],
};

describe("renderSite", () => {
  const html = renderSite(MODEL);

  test("renders every unit kind it was given", () => {
    for (const u of MODEL.units) expect(html).toContain(u.name);
    for (const label of ["Capabilities", "Spine", "Libraries", "Packs"]) expect(html).toContain(label);
  });

  test("renders the service contract, not just the name", () => {
    expect(html).toContain("8082");
    expect(html).toContain("process");
    expect(html).toContain("container");
  });

  // The register records no version since Q77 (2026-09-02), so the page groups names under
  // their verdict and renders nothing that could read as "the version this deployment runs".
  test("groups upstreams by verdict, and publishes no version", () => {
    for (const verdict of ["adopt", "reject", "inspiration"]) expect(html).toContain(verdict);
    for (const u of MODEL.upstreams) expect(html).toContain(u.name);
    expect(html).not.toContain("unpinned");
  });

  // #14: "Self-contained: no external CSS, JS, font or CDN request."
  //
  // Written against SUBRESOURCES rather than against every URL in the document. It used to
  // assert that no `https://` appeared anywhere at all, which is a stricter rule than the
  // criterion and a different one: an `<a href>` a reader may click is not something the page
  // fetches, and #168 needs the footer to link to the repository the whole site is generated
  // from. What matters is that nothing loads while the page renders, so that is what is
  // asserted — src, stylesheet links, and CSS url() — and it now catches an external image or
  // font, which the old blanket match would have reported without distinguishing.
  test("loads nothing from another host", () => {
    const subresources = [
      ...html.matchAll(/\bsrc\s*=\s*["']([^"']+)/g),
      ...html.matchAll(/<link\b[^>]*\bhref\s*=\s*["']([^"']+)/g),
      ...html.matchAll(/url\(\s*["']?([^"')]+)/g),
    ].map((m) => m[1]);
    expect(subresources.filter((u) => /^(https?:)?\/\//.test(u))).toEqual([]);
    expect(html).not.toContain("<script");
    expect(html).not.toContain("@import");
  });

  // The links that ARE allowed still have to be ones this repository owns — a generated page
  // that grew a third-party link would be a supply-chain surface nobody reviewed.
  test("links off-site only to the repository it is generated from", () => {
    const hosts = new Set(
      [...html.matchAll(/https?:\/\/([^/"' )]+)/g)].map((m) => m[1]),
    );
    expect([...hosts]).toEqual(["github.com"]);
  });

  // #14: "Every rendered fact traces to a manifest or a generated artifact." The code graph is
  // neither — graphify-out/ is untracked, so a fresh clone reproduces none of it. A future edit
  // that helpfully surfaces those numbers would break the criterion silently; this catches it.
  test("renders nothing sourced from the untracked code graph", () => {
    const withGraph = renderSite({
      ...MODEL,
      // @ts-expect-error — deliberately feeding fields the renderer must ignore
      graph: { nodes: 5355, present: true, stale: ["tools/lib/publish.sh"], external: 12, unmatched: [] },
      units: MODEL.units.map((u) => ({ ...u, code: { files: 99, nodes: 1234 } })),
    });
    expect(withGraph).not.toContain("5355");
    expect(withGraph).not.toContain("1234");
    expect(withGraph).not.toContain("publish.sh");
  });

  test("escapes manifest text rather than trusting it", () => {
    const nasty = renderSite({
      ...MODEL,
      units: [{ name: "<img src=x onerror=1>", kind: "capability" as const }],
    });
    expect(nasty).not.toContain("<img src=x");
    expect(nasty).toContain("&lt;img");
  });
});

// #14: "No overlay-scoped capability, port or host appears in the output." Asserted against the
// REAL self.json, because the property is about what the committed artifact contains — a fixture
// would only prove the fixture is clean.
describe("the real self.json", () => {
  const model = JSON.parse(readFileSync(join(SJEL_ROOT, "self.json"), "utf8"));

  test("carries no overlay-owned capability", () => {
    // tools/lib/paths.sh: generators that write tracked artifacts scan SJEL_CAPS_DIR alone,
    // because a capability name is itself a fact about a private deployment. This is the
    // downstream check on that rule.
    const overlayCaps = ["server"];
    const names = new Set(model.units.map((u: { name: string }) => u.name));
    for (const cap of overlayCaps) expect(names.has(cap)).toBe(false);
  });

  test("renders without a host, home path or tailnet name", () => {
    const html = renderSite(model);
    expect(html).not.toMatch(/\/Users\/[A-Za-z0-9._-]+\//);
    expect(html).not.toMatch(/\/home\/[A-Za-z0-9._-]+\//);
    expect(html).not.toMatch(/\.ts\.net/);
    expect(html).not.toMatch(/\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b/);
  });
});
