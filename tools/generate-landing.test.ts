import { describe, expect, test } from "bun:test";
import { renderLanding } from "./generate-landing.ts";

describe("landing page", () => {
  test("every capability is listed and links to its reference page, an unmapped one under Platform", () => {
    const html = renderLanding({
      units: [
        { name: "trips", kind: "capability" },
        { name: "brand-new", kind: "capability" },
        { name: "agent", kind: "lib" },
      ],
      upstreams: [],
    });
    expect(html).toContain('href="docs/trips.html"');
    expect(html).toContain('href="docs/brand-new.html"');
    const platform = html.slice(html.indexOf("<h3>Platform"));
    expect(platform).toContain("brand-new");
    expect(html).toContain("<b>2</b><span>capabilities</span>");
    expect(html).not.toContain("<h3>Assistant");
  });
});
