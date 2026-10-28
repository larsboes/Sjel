import { describe, expect, test } from "bun:test";
import { validateBase, validateNote } from "../src/validate.ts";

describe("Base validation", () => {
	test("accepts a Base with a defined formula", () => {
		const result = validateBase(`
formulas:
  age: '(today() - file.ctime).days'
views:
  - type: table
    name: Recent
    order:
      - file.name
      - formula.age
`);
		expect(result).toEqual({ valid: true, diagnostics: [] });
	});

	test("reports duplicate YAML keys and undefined formulas", () => {
		const duplicate = validateBase("views: []\nviews: []\n");
		expect(duplicate.valid).toBe(false);
		expect(duplicate.diagnostics.some((item) => item.code === "DUPLICATE_KEY")).toBe(true);

		const missing = validateBase(`
views:
  - type: table
    name: Broken
    order: [formula.missing]
`);
		expect(missing.valid).toBe(false);
		expect(missing.diagnostics).toContainEqual(expect.objectContaining({ code: "BASE_FORMULA_UNDEFINED" }));
	});

	test("requires named, typed views and string expressions", () => {
		const result = validateBase("formulas:\n  total: 42\nviews:\n  - type: table\n");
		expect(result.valid).toBe(false);
		expect(result.diagnostics.map((item) => item.code)).toEqual([
			"BASE_EXPRESSION_TYPE",
			"BASE_VIEW_NAME",
		]);
	});
});

describe("note validation", () => {
	test("reports malformed frontmatter and invalid policy fields", () => {
		const malformed = validateNote("---\nclass: c1\nclass: c2\n---\n", "Note.md", null);
		expect(malformed.valid).toBe(false);
		expect(malformed.diagnostics.some((item) => item.code === "DUPLICATE_KEY")).toBe(true);
		expect(malformed.diagnostics.some((item) => item.code === "ACCESS_POLICY_INVALID")).toBe(true);
	});

	test("warns, but does not reject, when frontmatter is absent", () => {
		const result = validateNote("# Draft\n", "Note.md", null);
		expect(result.valid).toBe(true);
		expect(result.diagnostics).toContainEqual(expect.objectContaining({ code: "FRONTMATTER_MISSING", severity: "warning" }));
	});
});
