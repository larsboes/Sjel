import { spawnSync } from "node:child_process";
import { parseDocument } from "yaml";
import { AGENT_ACCESS_LEVELS, DATA_CLASSES, type AgentAccess, type Classification, type DataClass } from "./contracts.ts";

interface ParsedFrontmatter {
	declaredClass?: string;
	agentAccess?: string;
	sensitivity?: string;
	error?: string;
}

function frontmatterBlock(markdown: string): string | null {
	if (!markdown.startsWith("---\n") && !markdown.startsWith("---\r\n")) return null;
	const lines = markdown.split(/\r?\n/);
	for (let index = 1; index < lines.length; index += 1) {
		if (lines[index] === "---" || lines[index] === "...") return lines.slice(1, index).join("\n");
	}
	return null;
}

export function parseAccessFrontmatter(markdown: string): ParsedFrontmatter {
	const block = frontmatterBlock(markdown);
	if (block === null) return {};

	const document = parseDocument(block, { uniqueKeys: true });
	if (document.errors.length > 0) return { error: document.errors[0]?.message ?? "invalid YAML frontmatter" };
	const value = document.toJS();
	if (value === null || typeof value !== "object" || Array.isArray(value)) {
		return { error: "frontmatter must be a YAML mapping" };
	}
	const fields = value as Record<string, unknown>;
	const declaredClass = fields.class;
	const agentAccess = fields.agent_access;
	const sensitivity = fields.sensitivity;
	if (declaredClass !== undefined && typeof declaredClass !== "string") {
		return { error: "frontmatter class must be a string" };
	}
	if (agentAccess !== undefined && typeof agentAccess !== "string") {
		return { error: "frontmatter agent_access must be a string" };
	}
	if (sensitivity !== undefined && typeof sensitivity !== "string") {
		return { error: "frontmatter sensitivity must be a string" };
	}
	return { declaredClass, agentAccess, sensitivity };
}

function canonicalClass(path: string, declaredClass: string | undefined, classifierPath: string): Pick<Classification, "dataClass" | "rationale"> | null {
	const args = ["classify", "--id", path.replaceAll("\\", "/"), "--json"];
	if (declaredClass !== undefined) args.push("--declared", declaredClass);
	const result = spawnSync(classifierPath, args, { encoding: "utf8", timeout: 5_000, maxBuffer: 16_384 });
	if (result.error || result.status !== 0) return null;
	try {
		const value = JSON.parse(result.stdout) as { value?: unknown; rationale?: unknown };
		if (typeof value.value !== "string" || !DATA_CLASSES.includes(value.value as DataClass) || typeof value.rationale !== "string") return null;
		return { dataClass: value.value as DataClass, rationale: value.rationale };
	} catch {
		return null;
	}
}

/**
 * Folder defaults come from Sjel's canonical Rust classifier when its local
 * binary is configured. Without that binary an undeclared note uses C2, which
 * blocks cloud reads rather than copying the folder rules into TypeScript.
 */
export function classifyNote(markdown: string, path = "", classifierPath: string | null = null): Classification {
	const parsed = parseAccessFrontmatter(markdown);
	if (parsed.error) {
		return {
			dataClass: "c3",
			agentAccess: "confidential",
			rationale: `Frontmatter could not be classified: ${parsed.error}`,
			method: "conservative-fallback",
			valid: false,
		};
	}

	let dataClass: DataClass = "c2";
	let method: Classification["method"] = "conservative-fallback";
	const reasons: string[] = [];
	if (classifierPath && path) {
		const canonical = canonicalClass(path, parsed.declaredClass, classifierPath);
		if (!canonical) {
			return {
				dataClass: "c3",
				agentAccess: "confidential",
				rationale: "The configured canonical vault classifier failed.",
				method,
				valid: false,
			};
		}
		dataClass = canonical.dataClass;
		method = "canonical";
		reasons.push(canonical.rationale);
	} else if (parsed.declaredClass !== undefined) {
		const declared = parsed.declaredClass.trim().toLowerCase();
		if (!DATA_CLASSES.includes(declared as DataClass)) {
			return {
				dataClass: "c3",
				agentAccess: "confidential",
				rationale: `Unknown class value ${JSON.stringify(parsed.declaredClass)}.`,
				method: "conservative-fallback",
				valid: false,
			};
		}
		dataClass = declared as DataClass;
		method = "frontmatter";
		reasons.push(`Frontmatter declares ${dataClass}.`);
	} else {
		reasons.push("No canonical folder classification was available, so the extension used C2.");
	}

	let agentAccess: AgentAccess = "standard";
	if (parsed.sensitivity !== undefined) {
		const legacy = parsed.sensitivity.trim().toLowerCase();
		if (legacy === "internal") {
			if (agentAccess === "standard") agentAccess = "internal";
			reasons.push("Legacy sensitivity restricts agent access to internal.");
		} else if (legacy === "restricted") {
			agentAccess = "confidential";
			reasons.push("Legacy sensitivity restricts agent access to confidential.");
		} else if (legacy !== "public") {
			return {
				dataClass: "c3",
				agentAccess: "confidential",
				rationale: `Unknown sensitivity value ${JSON.stringify(parsed.sensitivity)}.`,
				method: "conservative-fallback",
				valid: false,
			};
		}
	}
	if (parsed.agentAccess !== undefined) {
		const declared = parsed.agentAccess.trim().toLowerCase();
		if (!AGENT_ACCESS_LEVELS.includes(declared as AgentAccess) || declared === "standard") {
			return {
				dataClass: "c3",
				agentAccess: "confidential",
				rationale: `agent_access must be internal or confidential; omit the field for standard access (got ${JSON.stringify(parsed.agentAccess)}).`,
				method: "conservative-fallback",
				valid: false,
			};
		}
		if (declared === "confidential" || agentAccess === "standard") agentAccess = declared as AgentAccess;
		reasons.push(`Frontmatter restricts agent access to ${declared}.`);
	}

	return { dataClass, agentAccess, rationale: reasons.join(" "), method, valid: true };
}
