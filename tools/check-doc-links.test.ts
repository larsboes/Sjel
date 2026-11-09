import { afterAll, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { check, headingAnchors, stripCode } from "./check-doc-links";

const scratch = mkdtempSync(join(tmpdir(), "doc-links-"));
afterAll(() => rmSync(scratch, { recursive: true, force: true }));

function tree(name: string, files: Record<string, string>): string {
  const root = join(scratch, name);
  for (const [path, body] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), body);
  }
  return root;
}

describe("check-doc-links", () => {
  test("a link to an existing file and heading passes", () => {
    const root = tree("ok", {
      "a.md": "See [b](docs/b.md#two-words) and [self](#top).\n\n# Top\n",
      "docs/b.md": "# Two words\n",
    });
    expect(check(root).broken).toEqual([]);
  });

  test("a moved target is reported as missing", () => {
    const root = tree("missing", { "a.md": "[old](capabilities/x/README.md)\n" });
    expect(check(root).broken).toEqual([{ file: "a.md", target: "capabilities/x/README.md", reason: "missing" }]);
  });

  test("a renamed heading is reported as a broken anchor", () => {
    const root = tree("anchor", { "a.md": "[b](b.md#old-name)\n", "b.md": "# New name\n" });
    expect(check(root).broken.map((b) => b.reason)).toEqual(["anchor"]);
  });

  test("links inside code are examples, and URLs are not checked", () => {
    const root = tree("code", {
      "a.md": "Use `[text](url)` here.\n\n```md\n[x](nowhere.md)\n```\n\n[web](https://example.com/x)\n",
    });
    expect(check(root).broken).toEqual([]);
  });

  test("third-party notices and the overlay template are skipped", () => {
    const root = tree("skip", {
      "tools/templates/x/README.md": "[up](../Sjel/)\n",
      "tools/distribution/notices/n.md": "[l](LICENSE)\n",
      "a.md": "# A\n",
    });
    expect(check(root).broken).toEqual([]);
  });

  test("heading slugs follow GitHub: punctuation dropped, repeats numbered", () => {
    const a = headingAnchors("## 11. UI stack\n### Generated views (Priority 3)\n## Done\n## Done\n");
    expect([...a]).toEqual(["11-ui-stack", "generated-views-priority-3", "done", "done-1"]);
  });

  test("stripCode removes fences and inline code only", () => {
    expect(stripCode("a `b` c\n```\nd\n```\ne")).toBe("a  c\n\ne");
  });
});
