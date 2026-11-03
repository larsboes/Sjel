import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { brokenLinks } from "./links.ts";
import { rewriteHref } from "./markdown.ts";
import { parseRoutes } from "./routes.ts";

describe("routes", () => {
  test("reads helper calls and Route structs, joins continued lines, and sees a schema", () => {
    const block = `const ROUTES: &[route_manifest::Route] = &[
    route_manifest::get("GET", "/health", "Liveness."),
    // a comment with "GET" in it
    route_manifest::Route {
        method: "POST",
        path: "/api/items/{id}",
        summary: "Change one item: \\
                  { name? }.",
        request_schema: Some(route_manifest::schema_of::<Edit>),
    },
    r("DELETE", "/api/items/{id}", "Remove it."),`;
    expect(parseRoutes(block)).toEqual([
      { method: "GET", path: "/health", summary: "Liveness.", schema: false },
      { method: "POST", path: "/api/items/{id}", summary: "Change one item: { name? }.", schema: true },
      { method: "DELETE", path: "/api/items/{id}", summary: "Remove it.", schema: false },
    ]);
  });
});

describe("markdown links", () => {
  const from = "website/content/architecture/index.md";
  const page = "docs/architecture/index.html";
  test("a docs link becomes a site page, a repository link becomes GitHub, others stay", () => {
    expect(rewriteHref("security.md#data-classes", from, page)).toBe("security.html#data-classes");
    expect(rewriteHref("../index.md", from, page)).toBe("../index.html");
    expect(rewriteHref("../../../CONTRIBUTING.md#x", from, page)).toBe("https://github.com/larsboes/Sjel/blob/main/CONTRIBUTING.md#x");
    expect(rewriteHref("../../../libs/sjel-server/", from, page)).toBe("https://github.com/larsboes/Sjel/tree/main/libs/sjel-server");
    expect(rewriteHref("https://example.org/a.md", from, page)).toBe("https://example.org/a.md");
    expect(rewriteHref("#local", from, page)).toBe("#local");
  });
});

describe("link check", () => {
  test("a missing page and a missing anchor are both broken", async () => {
    const site = mkdtempSync(join(tmpdir(), "sjel-links-"));
    await Bun.write(join(site, "index.html"), '<a href="a.html#here">a</a><a href="gone.html">g</a><a href="a.html#nowhere">n</a><a href="https://x.org">x</a>');
    await Bun.write(join(site, "a.html"), '<h2 id="here">h</h2>');
    expect(brokenLinks(site)).toEqual(["index.html: gone.html (no such file)", "index.html: a.html#nowhere (no #nowhere on that page)"]);
  });
});
