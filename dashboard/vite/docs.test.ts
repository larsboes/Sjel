import { describe, expect, test } from "bun:test";
import { resolveDocLink } from "../src/lib/docs/content";

const link = (path: string) => `/demo${path}`;
const repo = "https://github.com/larsboes/Sjel/blob/main";

describe("documentation links", () => {
  test("section indexes resolve siblings from the source directory", () => {
    expect(resolveDocLink("storage.md", "docs/architecture/index.md", link))
      .toBe("/demo/docs/architecture/storage");
    expect(resolveDocLink("../capabilities/manifest.md", "docs/architecture/index.md", link))
      .toBe("/demo/docs/capabilities/manifest");
  });

  test("repository documents and source files keep their repository paths", () => {
    expect(resolveDocLink("../ISA.md", "docs/README.md", link)).toBe(`${repo}/ISA.md`);
    expect(resolveDocLink("../../CONTRIBUTING.md#architecture-and-ownership", "docs/architecture/index.md", link))
      .toBe(`${repo}/CONTRIBUTING.md#architecture-and-ownership`);
    expect(resolveDocLink("../../dashboard/service.toml", "docs/architecture/index.md", link))
      .toBe(`${repo}/dashboard/service.toml`);
  });

  test("index links reach their document and preserve anchors", () => {
    expect(resolveDocLink("../capabilities/index.md", "docs/architecture/storage.md", link))
      .toBe("/demo/docs/capabilities");
    expect(resolveDocLink("index.md#boundary-invariants", "docs/architecture/storage.md", link))
      .toBe("/demo/docs/architecture#boundary-invariants");
    expect(resolveDocLink("../README.md", "docs/architecture/storage.md", link))
      .toBe("/demo/docs/README");
  });

  test("external and same-document references remain unchanged", () => {
    for (const href of ["https://example.org/a#b", "mailto:hello@example.org", "#boundary-invariants", "//example.org/a"]) {
      expect(resolveDocLink(href, "docs/README.md", link)).toBe(href);
    }
  });
});
