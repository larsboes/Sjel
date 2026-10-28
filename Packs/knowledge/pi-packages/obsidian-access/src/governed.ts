import { basename, posix } from "node:path";
import type { AuditOperation } from "./audit.ts";
import type { AccessConfig, AccessDecision, Classification, ModelIdentity, PatchInstruction } from "./contracts.ts";
import { ObsidianClient, ObsidianHttpError } from "./client.ts";
import { classifyNote } from "./frontmatter.ts";
import { validateBase, type ValidationResult } from "./validate.ts";
import { applyPathRules, decideAccess } from "./policy.ts";

export interface GovernedRead {
	path: string;
	content: string;
	decision: AccessDecision;
}

export interface GovernedBaseRead extends GovernedRead {
	validation: ValidationResult;
}

export interface MutationResult {
	path: string;
	classification: Classification;
	accessibleAfterWrite: boolean;
}

export type ConfirmMutation = (message: string) => Promise<boolean>;

const CLASS_RANK = { c0: 0, c1: 1, c2: 2, c3: 3 } as const;
const ACCESS_RANK = { standard: 0, internal: 1, confidential: 2 } as const;

function assertMarkdownPath(path: string): void {
	if (!path.toLowerCase().endsWith(".md")) throw new Error("Obsidian text mutations require a .md path");
}

function isDowngrade(before: Classification, after: Classification): boolean {
	return CLASS_RANK[after.dataClass] < CLASS_RANK[before.dataClass] || ACCESS_RANK[after.agentAccess] < ACCESS_RANK[before.agentAccess];
}

export class GovernedObsidian {
	constructor(
		private readonly client: ObsidianClient,
		private readonly config: AccessConfig,
		private readonly onAdmit?: (decision: AccessDecision) => void,
		private readonly audit?: (operation: AuditOperation, path: string, model: ModelIdentity, decision: AccessDecision) => void,
	) {}

	private classify(content: string, path: string): Classification {
		return applyPathRules(classifyNote(content, path, this.config.classifierPath), path, this.config.paths);
	}

	private pathDecision(path: string, model: ModelIdentity): AccessDecision {
		return decideAccess(this.config, model, this.classify("", path));
	}

	private async inspect(path: string, model: ModelIdentity, signal?: AbortSignal, operation: AuditOperation = "read"): Promise<GovernedRead | null> {
		assertMarkdownPath(path);
		const content = await this.client.read(path, signal);
		const decision = decideAccess(this.config, model, this.classify(content, path));
		this.audit?.(operation, path, model, decision);
		return decision.allowed ? { path, content, decision } : null;
	}

	async read(path: string, model: ModelIdentity, signal?: AbortSignal): Promise<GovernedRead | null> {
		const result = await this.inspect(path, model, signal);
		if (result) this.onAdmit?.(result.decision);
		return result;
	}

	async readBase(path: string, model: ModelIdentity, signal?: AbortSignal): Promise<GovernedBaseRead | null> {
		const decision = this.pathDecision(path, model);
		this.audit?.("read", path, model, decision);
		if (!decision.allowed) return null;
		const content = await this.client.readBase(path, signal);
		this.onAdmit?.(decision);
		return { path, content, decision, validation: validateBase(content) };
	}

	async targetedRead(
		path: string,
		targetType: "heading" | "block" | "frontmatter",
		target: string | string[],
		scope: "content" | "marker" | "markerAndContent",
		model: ModelIdentity,
		signal?: AbortSignal,
	): Promise<GovernedRead | null> {
		const note = await this.inspect(path, model, signal);
		if (!note) return null;
		const content = await this.client.readTarget(path, targetType, target, scope, signal);
		this.onAdmit?.(note.decision);
		return { ...note, content };
	}

	async active(model: ModelIdentity, signal?: AbortSignal): Promise<GovernedRead | null> {
		const note = await this.client.metadata(null, signal);
		const decision = decideAccess(this.config, model, this.classify(note.content, note.path));
		if (!decision.allowed) return null;
		this.onAdmit?.(decision);
		return { path: note.path, content: note.content, decision };
	}

	async backlinks(path: string, model: ModelIdentity, signal?: AbortSignal): Promise<string[] | null> {
		const source = await this.inspect(path, model, signal);
		if (!source) return null;
		const metadata = await this.client.metadata(path, signal);
		const admitted: string[] = [];
		for (const candidate of metadata.backlinks.slice(0, this.config.maxCollectionItems)) {
			try {
				const note = await this.inspect(candidate, model, signal);
				if (!note) continue;
				admitted.push(candidate);
				this.onAdmit?.(note.decision);
			} catch (error) {
				if (!(error instanceof ObsidianHttpError) || error.status !== 404) throw error;
			}
		}
		this.onAdmit?.(source.decision);
		return admitted;
	}

	async tags(path: string, model: ModelIdentity, signal?: AbortSignal): Promise<string[] | null> {
		const note = await this.inspect(path, model, signal);
		if (!note) return null;
		const metadata = await this.client.metadata(path, signal);
		this.onAdmit?.(note.decision);
		return metadata.tags.slice(0, this.config.maxCollectionItems);
	}

	async properties(path: string, model: ModelIdentity, signal?: AbortSignal): Promise<Record<string, unknown> | null> {
		const note = await this.inspect(path, model, signal);
		if (!note) return null;
		const metadata = await this.client.metadata(path, signal);
		this.onAdmit?.(note.decision);
		return metadata.frontmatter;
	}

	async outline(path: string, model: ModelIdentity, signal?: AbortSignal): Promise<{ headings: string[]; frontmatterFields: string[] } | null> {
		const note = await this.inspect(path, model, signal);
		if (!note) return null;
		const map = await this.client.documentMap(path, signal);
		this.onAdmit?.(note.decision);
		return { headings: outline(note.content), frontmatterFields: map.frontmatterFields };
	}

	async list(path: string, model: ModelIdentity, signal?: AbortSignal): Promise<string[]> {
		const entries = await this.client.list(path, signal);
		const admitted: string[] = [];
		for (const entry of entries) {
			const candidate = path ? posix.join(path.replaceAll("\\", "/"), entry) : entry;
			if (entry.endsWith("/")) {
				const decision = this.pathDecision(`${candidate}/`, model);
				this.audit?.("list", candidate, model, decision);
				if (decision.allowed) admitted.push(entry);
				continue;
			}
			try {
				if (entry.toLowerCase().endsWith(".md")) {
					if (await this.read(candidate, model, signal)) admitted.push(entry);
				} else if (entry.toLowerCase().endsWith(".base")) {
					if (await this.readBase(candidate, model, signal)) admitted.push(entry);
				} else {
					const decision = this.pathDecision(candidate, model);
					this.audit?.("list", candidate, model, decision);
					if (decision.allowed) admitted.push(entry);
				}
			} catch (error) {
				// A concurrently removed entry stays invisible. Authentication and transport
				// failures stop the listing rather than becoming a plausible empty result.
				if (!(error instanceof ObsidianHttpError) || error.status !== 404) throw error;
			}
		}
		return admitted;
	}

	async search(query: string, model: ModelIdentity, contextLength: number, signal?: AbortSignal): Promise<unknown[]> {
		const results = await this.client.search(query, contextLength, signal);
		const admitted: unknown[] = [];
		for (const result of results) {
			const path = searchResultPath(result);
			if (path === null) continue;
			try {
				if (path.toLowerCase().endsWith(".md")) {
					if (await this.read(path, model, signal)) admitted.push(result);
				} else if (path.toLowerCase().endsWith(".base")) {
					if (await this.readBase(path, model, signal)) admitted.push(result);
				}
			} catch (error) {
				// Search follows the same concurrent-delete rule as list.
				if (!(error instanceof ObsidianHttpError) || error.status !== 404) throw error;
			}
		}
		return admitted;
	}

	async create(path: string, content: string, model: ModelIdentity, signal?: AbortSignal): Promise<MutationResult> {
		assertMarkdownPath(path);
		try {
			const existing = await this.inspect(path, model, signal);
			if (!existing) throw new Error("Obsidian object was not found or is not available to the active model");
			throw new Error("Obsidian note already exists; use obsidian_write to replace it");
		} catch (error) {
			if (!(error instanceof ObsidianHttpError) || error.status !== 404) throw error;
		}
		const classification = this.classify(content, path);
		const decision = decideAccess(this.config, model, classification);
		this.audit?.("create", path, model, decision);
		if (!decision.allowed) throw new Error("The active model may not create a note with the requested classification");
		await this.client.createExclusive(path, content, signal);
		this.onAdmit?.(decision);
		return { path, classification, accessibleAfterWrite: true };
	}

	async append(path: string, content: string, model: ModelIdentity, confirm: ConfirmMutation, signal?: AbortSignal): Promise<MutationResult> {
		assertMarkdownPath(path);
		if (!content) throw new Error("obsidian_append requires non-empty content");
		const map = await this.client.documentMap(path, signal);
		const current = await this.inspect(path, model, signal);
		if (!current) throw new Error("Obsidian object was not found or is not available to the active model");
		const proposed = this.classify(current.content + content, path);
		const decision = decideAccess(this.config, model, proposed);
		this.audit?.("append", path, model, decision);
		if (!decision.allowed) throw new Error("The active model may not write the resulting note classification");
		if (isDowngrade(current.decision.classification, proposed) && !(await confirm(`Append to \`${path}\` even though it would lower the note's class or agent access?`))) {
			throw new Error("Obsidian append cancelled");
		}
		const updated = await this.client.patch(path, {
			targetType: "heading",
			target: null,
			operation: "append",
			content,
			ifMatch: map.version,
		}, signal);
		const classification = this.classify(updated, path);
		const updatedDecision = decideAccess(this.config, model, classification);
		this.onAdmit?.(updatedDecision);
		return { path, classification, accessibleAfterWrite: updatedDecision.allowed };
	}

	async write(path: string, content: string, model: ModelIdentity, confirm: ConfirmMutation, signal?: AbortSignal): Promise<MutationResult> {
		assertMarkdownPath(path);
		const map = await this.client.documentMap(path, signal);
		const current = await this.inspect(path, model, signal);
		if (!current) throw new Error("Obsidian object was not found or is not available to the active model");
		const classification = this.classify(content, path);
		const decision = decideAccess(this.config, model, classification);
		this.audit?.("write", path, model, decision);
		if (!decision.allowed) throw new Error("The active model may not write the resulting note classification");
		const downgrade = isDowngrade(current.decision.classification, classification);
		const message = downgrade
			? `Replace all contents of \`${path}\` even though this would lower its class or agent access?`
			: `Replace all contents of \`${path}\`?`;
		if (!(await confirm(message))) throw new Error("Obsidian write cancelled");
		const updated = await this.client.patch(path, {
			targetType: "heading",
			target: null,
			operation: "replace",
			content,
			ifMatch: map.version,
		}, signal);
		const updatedClassification = this.classify(updated, path);
		const updatedDecision = decideAccess(this.config, model, updatedClassification);
		this.onAdmit?.(updatedDecision);
		return { path, classification: updatedClassification, accessibleAfterWrite: updatedDecision.allowed };
	}

	private async destinationOverwriteAllowed(path: string, overwrite: boolean, model: ModelIdentity, signal?: AbortSignal): Promise<void> {
		if (!overwrite) return;
		try {
			const destination = await this.inspect(path, model, signal);
			if (!destination) throw new Error("The overwrite destination is not available to the active model");
		} catch (error) {
			if (error instanceof ObsidianHttpError && error.status === 404) return;
			throw error;
		}
	}

	async copy(source: string, destination: string, overwrite: boolean, model: ModelIdentity, confirm: ConfirmMutation, signal?: AbortSignal): Promise<MutationResult> {
		assertMarkdownPath(source);
		assertMarkdownPath(destination);
		const note = await this.inspect(source, model, signal);
		if (!note) throw new Error("Obsidian object was not found or is not available to the active model");
		await this.destinationOverwriteAllowed(destination, overwrite, model, signal);
		const classification = this.classify(note.content, destination);
		const decision = decideAccess(this.config, model, classification);
		this.audit?.("copy", destination, model, decision);
		if (!decision.allowed) throw new Error("The destination classification is not available to the active model");
		if (overwrite && !(await confirm(`Copy \`${source}\` over the existing \`${destination}\`?`))) throw new Error("Obsidian copy cancelled");
		await this.client.copy(source, destination, overwrite, signal);
		return { path: destination, classification, accessibleAfterWrite: true };
	}

	async move(source: string, destination: string, overwrite: boolean, model: ModelIdentity, confirm: ConfirmMutation, signal?: AbortSignal): Promise<MutationResult> {
		assertMarkdownPath(source);
		assertMarkdownPath(destination);
		const note = await this.inspect(source, model, signal);
		if (!note) throw new Error("Obsidian object was not found or is not available to the active model");
		await this.destinationOverwriteAllowed(destination, overwrite, model, signal);
		const classification = this.classify(note.content, destination);
		const decision = decideAccess(this.config, model, classification);
		this.audit?.("move", destination, model, decision);
		if (!decision.allowed) throw new Error("The destination classification is not available to the active model");
		const action = overwrite
			? `Move \`${source}\` to \`${destination}\` and overwrite the existing destination?`
			: `Move \`${source}\` to \`${destination}\`?`;
		if (!(await confirm(action))) throw new Error("Obsidian move cancelled");
		await this.client.move(source, destination, overwrite, signal);
		return { path: destination, classification, accessibleAfterWrite: true };
	}

	async delete(path: string, permanent: boolean, model: ModelIdentity, confirm: ConfirmMutation, signal?: AbortSignal): Promise<void> {
		assertMarkdownPath(path);
		const note = await this.inspect(path, model, signal);
		if (!note) throw new Error("Obsidian object was not found or is not available to the active model");
		const action = permanent
			? `Permanently delete \`${path}\`? This bypasses the system trash.`
			: `Move \`${path}\` to the system trash?`;
		if (!(await confirm(action))) throw new Error("Obsidian delete cancelled");
		await this.client.delete(path, permanent, signal);
		this.audit?.("delete", path, model, note.decision);
	}

	async patch(path: string, instruction: Omit<PatchInstruction, "ifMatch">, model: ModelIdentity, confirm: ConfirmMutation, signal?: AbortSignal): Promise<MutationResult> {
		assertMarkdownPath(path);
		if (instruction.targetType === "heading" && !Array.isArray(instruction.target)) {
			throw new Error("A heading patch target must be a heading-path array");
		}
		if (instruction.targetType !== "heading" && typeof instruction.target !== "string") {
			throw new Error("A block or frontmatter patch target must be a string");
		}
		if (instruction.targetType === "heading" && (instruction.target === null || (Array.isArray(instruction.target) && instruction.target.length === 0))) {
			throw new Error("obsidian_patch does not target the document root; use obsidian_write or obsidian_append");
		}
		const map = await this.client.documentMap(path, signal);
		const current = await this.inspect(path, model, signal);
		if (!current) throw new Error("Obsidian object was not found or is not available to the active model");
		const policyField = instruction.targetType === "frontmatter" && (instruction.target === "class" || instruction.target === "agent_access");
		if ((instruction.operation === "replace" || instruction.operation === "delete" || policyField) &&
			!(await confirm(`Apply ${instruction.operation} to the selected ${instruction.targetType} target in \`${path}\`?`))) {
			throw new Error("Obsidian patch cancelled");
		}
		const updated = await this.client.patch(path, { ...instruction, ifMatch: map.version }, signal);
		const classification = this.classify(updated, path);
		const decision = decideAccess(this.config, model, classification);
		this.audit?.("patch", path, model, decision);
		if (decision.allowed) this.onAdmit?.(decision);
		return { path, classification, accessibleAfterWrite: decision.allowed };
	}
}

function searchResultPath(result: unknown): string | null {
	if (result === null || typeof result !== "object" || Array.isArray(result)) return null;
	const candidate = result as { filename?: unknown; path?: unknown };
	if (typeof candidate.filename === "string") return candidate.filename;
	if (typeof candidate.path === "string") return candidate.path;
	return null;
}

export function outline(markdown: string): string[] {
	const headings: string[] = [];
	let fenced = false;
	for (const line of markdown.split(/\r?\n/)) {
		if (/^\s*(```|~~~)/.test(line)) {
			fenced = !fenced;
			continue;
		}
		if (fenced) continue;
		const match = /^(#{1,6})\s+(.+?)\s*#*\s*$/.exec(line);
		if (match) headings.push(`${match[1]} ${match[2]}`);
	}
	return headings;
}

export function displayName(path: string): string {
	return basename(path.replaceAll("\\", "/"));
}
