import { parseDocument } from "yaml";
import type { Classification, PathRule } from "./contracts.ts";
import { classifyNote } from "./frontmatter.ts";
import { applyPathRules } from "./policy.ts";

export interface Diagnostic {
	severity: "error" | "warning";
	code: string;
	message: string;
}

export interface ValidationResult {
	valid: boolean;
	diagnostics: Diagnostic[];
}

export interface NoteValidationResult extends ValidationResult {
	classification: Classification;
}

function yamlDocument(content: string) {
	return parseDocument(content, { uniqueKeys: true, prettyErrors: true });
}

function yamlDiagnostics(content: string): { value: unknown; diagnostics: Diagnostic[] } {
	const document = yamlDocument(content);
	const diagnostics: Diagnostic[] = [
		...document.errors.map((error) => ({ severity: "error" as const, code: error.code, message: error.message })),
		...document.warnings.map((warning) => ({ severity: "warning" as const, code: warning.code, message: warning.message })),
	];
	return { value: document.errors.length === 0 ? document.toJS() : null, diagnostics };
}

function mapping(value: unknown): value is Record<string, unknown> {
	return value !== null && typeof value === "object" && !Array.isArray(value);
}

function collectFormulaReferences(value: unknown, references: Set<string>): void {
	if (typeof value === "string") {
		for (const match of value.matchAll(/\bformula\.([A-Za-z_][A-Za-z0-9_-]*)\b/g)) references.add(match[1] ?? "");
		return;
	}
	if (Array.isArray(value)) {
		for (const item of value) collectFormulaReferences(item, references);
		return;
	}
	if (mapping(value)) {
		for (const [key, item] of Object.entries(value)) {
			collectFormulaReferences(key, references);
			collectFormulaReferences(item, references);
		}
	}
}

function expectMapping(value: unknown, key: string, diagnostics: Diagnostic[]): Record<string, unknown> | null {
	if (value === undefined) return {};
	if (mapping(value)) return value;
	diagnostics.push({ severity: "error", code: "BASE_SECTION_TYPE", message: `${key} must be a YAML mapping` });
	return null;
}

export function validateBase(content: string): ValidationResult {
	const parsed = yamlDiagnostics(content);
	const diagnostics = [...parsed.diagnostics];
	if (diagnostics.some((item) => item.severity === "error")) return { valid: false, diagnostics };
	if (!mapping(parsed.value)) {
		diagnostics.push({ severity: "error", code: "BASE_ROOT_TYPE", message: "A Base must be a YAML mapping" });
		return { valid: false, diagnostics };
	}

	const formulas = expectMapping(parsed.value.formulas, "formulas", diagnostics);
	const properties = expectMapping(parsed.value.properties, "properties", diagnostics);
	const summaries = expectMapping(parsed.value.summaries, "summaries", diagnostics);
	for (const [section, values] of [["formulas", formulas], ["summaries", summaries]] as const) {
		for (const [name, expression] of Object.entries(values ?? {})) {
			if (typeof expression !== "string") {
				diagnostics.push({ severity: "error", code: "BASE_EXPRESSION_TYPE", message: `${section}.${name} must be a string expression` });
			}
		}
	}
	for (const [name, options] of Object.entries(properties ?? {})) {
		if (!mapping(options)) diagnostics.push({ severity: "error", code: "BASE_PROPERTY_TYPE", message: `properties.${name} must be a mapping` });
	}

	const views = parsed.value.views;
	if (!Array.isArray(views) || views.length === 0) {
		diagnostics.push({ severity: "error", code: "BASE_VIEWS_REQUIRED", message: "views must be a non-empty YAML list" });
	} else {
		views.forEach((view, index) => {
			if (!mapping(view)) {
				diagnostics.push({ severity: "error", code: "BASE_VIEW_TYPE", message: `views[${index}] must be a mapping` });
				return;
			}
			if (typeof view.type !== "string" || !view.type.trim()) {
				diagnostics.push({ severity: "error", code: "BASE_VIEW_LAYOUT", message: `views[${index}].type must name a layout` });
			}
			if (typeof view.name !== "string" || !view.name.trim()) {
				diagnostics.push({ severity: "error", code: "BASE_VIEW_NAME", message: `views[${index}].name must be a non-empty string` });
			}
			if (view.order !== undefined && (!Array.isArray(view.order) || view.order.some((item) => typeof item !== "string"))) {
				diagnostics.push({ severity: "error", code: "BASE_VIEW_ORDER", message: `views[${index}].order must be a list of property names` });
			}
		});
	}

	const references = new Set<string>();
	collectFormulaReferences(parsed.value, references);
	for (const reference of references) {
		if (!Object.hasOwn(formulas ?? {}, reference)) {
			diagnostics.push({ severity: "error", code: "BASE_FORMULA_UNDEFINED", message: `formula.${reference} is referenced but not defined` });
		}
	}
	return { valid: !diagnostics.some((item) => item.severity === "error"), diagnostics };
}

function frontmatter(markdown: string): string | null {
	if (!markdown.startsWith("---\n") && !markdown.startsWith("---\r\n")) return null;
	const lines = markdown.split(/\r?\n/);
	for (let index = 1; index < lines.length; index += 1) {
		if (lines[index] === "---" || lines[index] === "...") return lines.slice(1, index).join("\n");
	}
	return null;
}

export function validateNote(markdown: string, path: string, classifierPath: string | null, pathRules: PathRule[] = []): NoteValidationResult {
	const diagnostics: Diagnostic[] = [];
	if (markdown.startsWith("---") && frontmatter(markdown) === null) {
		diagnostics.push({ severity: "error", code: "FRONTMATTER_UNCLOSED", message: "YAML frontmatter has no closing delimiter" });
	} else {
		const block = frontmatter(markdown);
		if (block === null) {
			diagnostics.push({ severity: "warning", code: "FRONTMATTER_MISSING", message: "The note has no YAML frontmatter" });
		} else {
			const parsed = yamlDiagnostics(block);
			diagnostics.push(...parsed.diagnostics);
			if (parsed.diagnostics.every((item) => item.severity !== "error") && !mapping(parsed.value)) {
				diagnostics.push({ severity: "error", code: "FRONTMATTER_ROOT_TYPE", message: "Frontmatter must be a YAML mapping" });
			}
		}
	}
	const classification = applyPathRules(classifyNote(markdown, path, classifierPath), path, pathRules);
	if (!classification.valid) {
		diagnostics.push({ severity: "error", code: "ACCESS_POLICY_INVALID", message: classification.rationale });
	}
	return { valid: !diagnostics.some((item) => item.severity === "error"), diagnostics, classification };
}
