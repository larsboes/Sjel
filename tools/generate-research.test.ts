import { describe, expect, test } from "bun:test";

import { rewriteLinks } from "./generate-research.ts";

describe("rewriteLinks", () => {
  test("keeps a link between entries on the site", () => {
    expect(rewriteLinks('<a href="why-sjel.md">x</a>')).toBe('<a href="why-sjel.html">x</a>');
    expect(rewriteLinks('<a href="README.md">x</a>')).toBe('<a href="index.html">x</a>');
  });
  test("sends a link to another repository file to GitHub, anchor kept", () => {
    expect(rewriteLinks('<a href="../README.md#what-it-does">x</a>')).toBe(
      '<a href="https://github.com/larsboes/Sjel/blob/main/README.md#what-it-does">x</a>',
    );
  });
  test("leaves external links and in-page anchors alone", () => {
    const html = '<a href="https://example.org/a">x</a><a href="#top">y</a>';
    expect(rewriteLinks(html)).toBe(html);
  });
});
