import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { AuditLog } from "./audit.ts";
import { createTransport, ObsidianClient, vaultPath } from "./client.ts";
import { SessionClearance } from "./clearance.ts";
import { loadConfig } from "./config.ts";
import type { AccessConfig, AccessDecision, ModelIdentity, PatchInstruction } from "./contracts.ts";
import { GovernedObsidian } from "./governed.ts";
import { loadKnowledgeContract, loadKnowledgeSettings, NO_CONTRACT_MESSAGE } from "./knowledge-contract.ts";
import { validateBase, validateNote, type Diagnostic } from "./validate.ts";

interface Runtime {
	config: AccessConfig;
	governed: GovernedObsidian;
}

function modelIdentity(ctx: ExtensionContext): ModelIdentity {
	if (!ctx.model) throw new Error("Obsidian access requires an active model");
	return { provider: ctx.model.provider, id: ctx.model.id };
}

function createClient(config: AccessConfig): ObsidianClient {
	const apiKey = process.env.OBSIDIAN_API_KEY;
	if (!apiKey) throw new Error("OBSIDIAN_API_KEY is not available to the Obsidian access extension");
	return new ObsidianClient(createTransport(config, apiKey), config.maxCollectionItems);
}

function runtime(onAdmit: (decision: AccessDecision) => void): Runtime {
	const config = loadConfig();
	const client = createClient(config);
	const audit = new AuditLog(config.auditPath);
	return { config, governed: new GovernedObsidian(client, config, onAdmit, (operation, path, model, decision) => audit.record(operation, path, model, decision)) };
}

async function currentKnowledgeContract() {
	const config = loadConfig();
	const client = createClient(config);
	return loadKnowledgeContract((path) => client.read(path));
}

function textResult(text: string, details: Record<string, unknown> = {}) {
	return { content: [{ type: "text" as const, text }], details };
}

function diagnosticsText(diagnostics: Diagnostic[]): string {
	if (diagnostics.length === 0) return "No syntax or reference errors found.";
	return diagnostics.map((item) => `${item.severity.toUpperCase()} ${item.code}: ${item.message}`).join("\n");
}

function denied(): never {
	throw new Error("Obsidian object was not found or is not available to the active model");
}

async function confirmMutation(ctx: ExtensionContext, message: string): Promise<boolean> {
	if (!ctx.hasUI) throw new Error("This Obsidian mutation requires interactive confirmation");
	return ctx.ui.confirm("Confirm Obsidian change", message);
}

function mutationResult(result: { path: string; classification: AccessDecision["classification"]; accessibleAfterWrite: boolean }) {
	if (!result.accessibleAfterWrite) return textResult("Obsidian change completed. The resulting note is not available to the active model.", { path: result.path });
	return textResult("Obsidian change completed.", {
		path: result.path,
		dataClass: result.classification.dataClass,
		agentAccess: result.classification.agentAccess,
	});
}

export default function obsidianAccess(pi: ExtensionAPI) {
	const clearance = new SessionClearance();
	const removedEgress = new Set<string>();
	let loadedContractDigest: string | null = null;

	function requireContract(): void {
		if (loadedContractDigest) return;
		// A malformed knowledge.toml throws here, so the gate fails closed.
		if (loadKnowledgeSettings().contractPath === null) return;
		throw new Error("Load obsidian_contract before mutating the vault in this session");
	}

	function isEgressRestricted(): boolean {
		const current = clearance.value;
		return current !== null && (current.dataClass === "c2" || current.agentAccess === "confidential");
	}

	function restrictEgress(): void {
		if (!isEgressRestricted()) return;
		const blocked = new Set(loadConfig().egressTools);
		const active = pi.getActiveTools();
		for (const name of active) if (blocked.has(name)) removedEgress.add(name);
		pi.setActiveTools(active.filter((name) => !blocked.has(name)));
	}

	function restoreEgress(): void {
		if (removedEgress.size === 0) return;
		pi.setActiveTools([...new Set([...pi.getActiveTools(), ...removedEgress])]);
		removedEgress.clear();
	}

	function recordClearance(decision: AccessDecision): void {
		if (!decision.allowed || !clearance.raise(decision.classification)) return;
		pi.appendEntry("obsidian-access-clearance", clearance.value);
		restrictEgress();
	}

	function currentRuntime(): Runtime {
		return runtime(recordClearance);
	}

	function modelMeetsClearance(model: ModelIdentity): boolean {
		if (!clearance.value) return true;
		try {
			return clearance.decide(loadConfig(), model)?.allowed ?? true;
		} catch {
			// A missing or malformed policy must not make a contaminated session fail open.
			return false;
		}
	}

	function showClearance(ctx: ExtensionContext, model: ModelIdentity): void {
		const current = clearance.value;
		if (!current) {
			ctx.ui.setStatus("obsidian-clearance", undefined);
			return;
		}
		if (modelMeetsClearance(model)) {
			ctx.ui.setStatus("obsidian-clearance", `Obsidian ${current.dataClass}/${current.agentAccess}`);
			return;
		}
		ctx.ui.setStatus("obsidian-clearance", "Obsidian model blocked");
		if (ctx.hasUI) ctx.ui.notify("This model is below the Obsidian clearance already present in the session. Switch back or start a new session.", "error");
	}

	pi.on("session_start", async (_event, ctx) => {
		restoreEgress();
		clearance.reset();
		loadedContractDigest = null;
		let configuredContractDigest: string | null = null;
		try {
			configuredContractDigest = (await currentKnowledgeContract())?.digest ?? null;
		} catch {
			configuredContractDigest = null;
		}
		for (const entry of ctx.sessionManager.getBranch()) {
			if (entry.type === "custom" && entry.customType === "obsidian-access-clearance") clearance.restore(entry.data);
			if (entry.type === "custom" && entry.customType === "obsidian-contract-loaded") {
				const data = entry.data as { digest?: unknown };
				if (typeof data.digest === "string" && data.digest === configuredContractDigest) loadedContractDigest = data.digest;
			}
		}
		restrictEgress();
		if (ctx.model) showClearance(ctx, { provider: ctx.model.provider, id: ctx.model.id });
	});

	pi.on("model_select", (event, ctx) => {
		showClearance(ctx, { provider: event.model.provider, id: event.model.id });
	});

	pi.on("input", (_event, ctx) => {
		if (modelMeetsClearance(modelIdentity(ctx))) return { action: "continue" as const };
		if (ctx.hasUI) ctx.ui.notify("Turn blocked: the active model is below this session's Obsidian clearance.", "error");
		return { action: "handled" as const };
	});

	pi.on("tool_call", (event) => {
		if (!isEgressRestricted()) return;
		let egressTools: string[];
		try {
			egressTools = loadConfig().egressTools;
		} catch {
			egressTools = ["web_search", "source_check", "fetch_content", "get_search_content"];
		}
		if (egressTools.includes(event.toolName)) {
			return { block: true, reason: "This Obsidian session contains C2 or confidential data; configured egress tools are disabled." };
		}
	});

	pi.registerTool({
		name: "obsidian_contract",
		label: "Obsidian Contract",
		description:
			"Load the vault-writing contract configured for this machine from Obsidian. Use before any vault mutation. Reports when no contract is configured.",
		promptSnippet: "Load the vault-writing contract configured for this machine",
		promptGuidelines: ["Call obsidian_contract before creating, editing, moving, or deleting vault notes in a session."],
		parameters: Type.Object({}),
		async execute() {
			const contract = await currentKnowledgeContract();
			if (contract === null) return textResult(NO_CONTRACT_MESSAGE, { configured: false });
			loadedContractDigest = contract.digest;
			pi.appendEntry("obsidian-contract-loaded", { profile: contract.profile, path: contract.path, digest: contract.digest });
			return textResult(contract.content, { configured: true, profile: contract.profile, path: contract.path });
		},
	});

	pi.registerTool({
		name: "obsidian_note_validate",
		label: "Validate Obsidian Note",
		description: "Validate Markdown frontmatter syntax and access-policy fields without writing to the vault.",
		promptSnippet: "Validate an Obsidian Markdown draft",
		parameters: Type.Object({
			content: Type.String({ description: "Complete Markdown content to validate" }),
			path: Type.Optional(Type.String({ description: "Prospective vault-relative .md path used for classification" })),
		}),
		async execute(_id, params) {
			if (params.path !== undefined) {
				if (!params.path.toLowerCase().endsWith(".md")) throw new Error("obsidian_note_validate path must end in .md");
				vaultPath.encode(params.path);
			}
			const config = loadConfig();
			const result = validateNote(params.content, params.path ?? "Draft.md", config.classifierPath, config.paths);
			return textResult(diagnosticsText(result.diagnostics), { ...result });
		},
	});

	pi.registerTool({
		name: "obsidian_base_validate",
		label: "Validate Obsidian Base",
		description: "Validate a .base YAML draft, including view shape and formula references, without writing to the vault.",
		promptSnippet: "Validate an Obsidian Base draft",
		parameters: Type.Object({ content: Type.String({ description: "Complete .base YAML content to validate" }) }),
		async execute(_id, params) {
			const result = validateBase(params.content);
			return textResult(diagnosticsText(result.diagnostics), { ...result });
		},
	});

	pi.registerTool({
		name: "obsidian_base_read",
		label: "Read Obsidian Base",
		description: "Read and validate one policy-admitted Obsidian .base file.",
		promptSnippet: "Read a policy-admitted Obsidian Base",
		parameters: Type.Object({ path: Type.String({ description: "Vault-relative .base path" }) }),
		async execute(_id, params, signal, _update, ctx) {
			const result = await currentRuntime().governed.readBase(params.path, modelIdentity(ctx), signal);
			if (!result) denied();
			const validation = diagnosticsText(result.validation.diagnostics);
			return textResult(`${validation}\n\n${result.content}`, {
				path: result.path,
				dataClass: result.decision.classification.dataClass,
				agentAccess: result.decision.classification.agentAccess,
				validation: result.validation,
			});
		},
	});

	pi.registerTool({
		name: "obsidian_read",
		label: "Obsidian Read",
		description: "Read one policy-admitted UTF-8 note from Obsidian. Restricted notes appear unavailable.",
		promptSnippet: "Read a policy-admitted Obsidian note by vault-relative path",
		promptGuidelines: [
			"Use obsidian_read instead of filesystem reads or the raw REST API for Obsidian notes.",
		],
		parameters: Type.Object({
			path: Type.String({ description: "Vault-relative note path, for example Projects/Sjel/ISA.md" }),
			targetType: Type.Optional(Type.Union([Type.Literal("heading"), Type.Literal("block"), Type.Literal("frontmatter")])),
			target: Type.Optional(Type.Union([Type.String(), Type.Array(Type.String(), { minItems: 1 })])),
			scope: Type.Optional(Type.Union([Type.Literal("content"), Type.Literal("marker"), Type.Literal("markerAndContent")])),
		}),
		async execute(_id, params, signal, _update, ctx) {
			if ((params.targetType === undefined) !== (params.target === undefined)) {
				throw new Error("targetType and target must be supplied together");
			}
			const result = params.targetType && params.target !== undefined
				? await currentRuntime().governed.targetedRead(params.path, params.targetType, params.target, params.scope ?? "content", modelIdentity(ctx), signal)
				: await currentRuntime().governed.read(params.path, modelIdentity(ctx), signal);
			if (!result) denied();
			return textResult(result.content, {
				path: result.path,
				dataClass: result.decision.classification.dataClass,
				agentAccess: result.decision.classification.agentAccess,
			});
		},
	});

	pi.registerTool({
		name: "obsidian_outline",
		label: "Obsidian Outline",
		description: "List headings from one policy-admitted Obsidian note without returning its body.",
		promptSnippet: "List headings from a policy-admitted Obsidian note",
		parameters: Type.Object({ path: Type.String({ description: "Vault-relative note path" }) }),
		async execute(_id, params, signal, _update, ctx) {
			const result = await currentRuntime().governed.outline(params.path, modelIdentity(ctx), signal);
			if (!result) denied();
			const lines = [...result.headings, ...result.frontmatterFields.map((field) => `frontmatter: ${field}`)];
			return textResult(lines.length ? lines.join("\n") : "No headings or frontmatter fields.", {
				path: params.path,
				headings: result.headings,
				frontmatterFields: result.frontmatterFields,
			});
		},
	});

	pi.registerTool({
		name: "obsidian_active_file",
		label: "Obsidian Active File",
		description: "Read the currently active Obsidian note if it is available to the active model.",
		promptSnippet: "Read the policy-admitted note currently active in Obsidian",
		parameters: Type.Object({}),
		async execute(_id, _params, signal, _update, ctx) {
			const result = await currentRuntime().governed.active(modelIdentity(ctx), signal);
			if (!result) denied();
			return textResult(result.content, { path: result.path });
		},
	});

	pi.registerTool({
		name: "obsidian_backlinks",
		label: "Obsidian Backlinks",
		description: "List backlinks whose source notes are independently available to the active model.",
		promptSnippet: "List policy-admitted backlinks to an Obsidian note",
		parameters: Type.Object({ path: Type.String({ description: "Vault-relative note path" }) }),
		async execute(_id, params, signal, _update, ctx) {
			const backlinks = await currentRuntime().governed.backlinks(params.path, modelIdentity(ctx), signal);
			if (!backlinks) denied();
			return textResult(backlinks.length ? backlinks.join("\n") : "No available backlinks.", { count: backlinks.length });
		},
	});

	pi.registerTool({
		name: "obsidian_tags",
		label: "Obsidian Tags",
		description: "List tags on one policy-admitted note. It does not expose vault-wide counts from denied notes.",
		promptSnippet: "List tags on a policy-admitted Obsidian note",
		parameters: Type.Object({ path: Type.String({ description: "Vault-relative note path" }) }),
		async execute(_id, params, signal, _update, ctx) {
			const tags = await currentRuntime().governed.tags(params.path, modelIdentity(ctx), signal);
			if (!tags) denied();
			return textResult(tags.length ? tags.join("\n") : "No tags.", { count: tags.length });
		},
	});

	pi.registerTool({
		name: "obsidian_properties",
		label: "Obsidian Properties",
		description: "Return frontmatter properties from one policy-admitted note without returning its body.",
		promptSnippet: "Read frontmatter properties from a policy-admitted note",
		parameters: Type.Object({ path: Type.String({ description: "Vault-relative .md path" }) }),
		async execute(_id, params, signal, _update, ctx) {
			const properties = await currentRuntime().governed.properties(params.path, modelIdentity(ctx), signal);
			if (!properties) denied();
			return textResult(JSON.stringify(properties, null, 2), { count: Object.keys(properties).length });
		},
	});

	pi.registerTool({
		name: "obsidian_list",
		label: "Obsidian List",
		description: "List only vault entries whose names are available to the active model.",
		promptSnippet: "List policy-admitted entries in an Obsidian directory",
		parameters: Type.Object({
			path: Type.Optional(Type.String({ description: "Vault-relative directory; omit for the root" })),
		}),
		async execute(_id, params, signal, _update, ctx) {
			const entries = await currentRuntime().governed.list(params.path ?? "", modelIdentity(ctx), signal);
			return textResult(entries.length ? entries.join("\n") : "No available entries.", { count: entries.length });
		},
	});

	pi.registerTool({
		name: "obsidian_search",
		label: "Obsidian Search",
		description: "Search Obsidian and remove every result not available to the active model before returning paths or snippets.",
		promptSnippet: "Search policy-admitted Obsidian notes",
		parameters: Type.Object({
			query: Type.String({ description: "Obsidian simple-search query" }),
			contextLength: Type.Optional(Type.Integer({ minimum: 0, maximum: 500, description: "Snippet characters around a match" })),
		}),
		async execute(_id, params, signal, _update, ctx) {
			const results = await currentRuntime().governed.search(params.query, modelIdentity(ctx), params.contextLength ?? 100, signal);
			return textResult(JSON.stringify(results, null, 2), { count: results.length });
		},
	});

	pi.registerTool({
		name: "obsidian_create",
		label: "Obsidian Create",
		description: "Create a new Markdown note without overwriting an existing vault object. The proposed note must be available to the active model.",
		promptSnippet: "Create a new policy-admitted Obsidian Markdown note",
		parameters: Type.Object({
			path: Type.String({ description: "Vault-relative .md path" }),
			content: Type.String({ description: "Complete Markdown content, including frontmatter when needed" }),
		}),
		async execute(_id, params, signal, _update, ctx) {
			requireContract();
			const result = await currentRuntime().governed.create(params.path, params.content, modelIdentity(ctx), signal);
			return mutationResult(result);
		},
	});

	pi.registerTool({
		name: "obsidian_append",
		label: "Obsidian Append",
		description: "Append Markdown to an existing policy-admitted note with an optimistic concurrency check.",
		promptSnippet: "Append Markdown to a policy-admitted Obsidian note",
		parameters: Type.Object({
			path: Type.String({ description: "Vault-relative .md path" }),
			content: Type.String({ minLength: 1, description: "Markdown to append" }),
		}),
		async execute(_id, params, signal, _update, ctx) {
			requireContract();
			const result = await currentRuntime().governed.append(
				params.path,
				params.content,
				modelIdentity(ctx),
				(message) => confirmMutation(ctx, message),
				signal,
			);
			return mutationResult(result);
		},
	});

	pi.registerTool({
		name: "obsidian_write",
		label: "Obsidian Write",
		description: "Replace the complete contents of an existing policy-admitted Markdown note. Always requires confirmation and rejects concurrent changes.",
		promptSnippet: "Replace an entire policy-admitted Obsidian note after confirmation",
		parameters: Type.Object({
			path: Type.String({ description: "Vault-relative .md path" }),
			content: Type.String({ description: "Complete replacement Markdown" }),
		}),
		async execute(_id, params, signal, _update, ctx) {
			requireContract();
			const result = await currentRuntime().governed.write(
				params.path,
				params.content,
				modelIdentity(ctx),
				(message) => confirmMutation(ctx, message),
				signal,
			);
			return mutationResult(result);
		},
	});

	pi.registerTool({
		name: "obsidian_patch",
		label: "Obsidian Patch",
		description: "Apply one markdown-patch 2.0 edit to a heading, block, or frontmatter field. Replacements, deletions, and policy-field edits require confirmation.",
		promptSnippet: "Edit a heading, block, or frontmatter field in an Obsidian note",
		parameters: Type.Object({
			path: Type.String({ description: "Vault-relative .md path" }),
			targetType: Type.Union([Type.Literal("heading"), Type.Literal("block"), Type.Literal("frontmatter")]),
			target: Type.Union([Type.String(), Type.Array(Type.String(), { minItems: 1 })], { description: "Heading path array, block id, or frontmatter key" }),
			operation: Type.Union([Type.Literal("replace"), Type.Literal("prepend"), Type.Literal("append"), Type.Literal("delete")]),
			scope: Type.Optional(Type.Union([Type.Literal("content"), Type.Literal("marker"), Type.Literal("markerAndContent")])),
			content: Type.Optional(Type.String({ description: "Markdown/text payload" })),
			value: Type.Optional(Type.Any({ description: "Structured frontmatter or table payload" })),
			within: Type.Optional(Type.Integer({ description: "Heading body-block index; negative values count from the end" })),
			createTargetIfMissing: Type.Optional(Type.Boolean()),
		}),
		async execute(_id, params, signal, _update, ctx) {
			requireContract();
			const instruction: Omit<PatchInstruction, "ifMatch"> = {
				targetType: params.targetType,
				target: params.target,
				operation: params.operation,
				scope: params.scope,
				content: params.content,
				value: params.value,
				within: params.within,
				createTargetIfMissing: params.createTargetIfMissing,
			};
			const result = await currentRuntime().governed.patch(
				params.path,
				instruction,
				modelIdentity(ctx),
				(message) => confirmMutation(ctx, message),
				signal,
			);
			return mutationResult(result);
		},
	});

	pi.registerTool({
		name: "obsidian_copy",
		label: "Obsidian Copy",
		description: "Copy a policy-admitted Markdown note. Existing destinations are preserved unless overwrite is explicitly requested and confirmed.",
		promptSnippet: "Copy a policy-admitted Obsidian note",
		parameters: Type.Object({
			source: Type.String({ description: "Existing vault-relative .md path" }),
			destination: Type.String({ description: "New vault-relative .md path" }),
			overwrite: Type.Optional(Type.Boolean({ default: false })),
		}),
		async execute(_id, params, signal, _update, ctx) {
			requireContract();
			const result = await currentRuntime().governed.copy(
				params.source,
				params.destination,
				params.overwrite ?? false,
				modelIdentity(ctx),
				(message) => confirmMutation(ctx, message),
				signal,
			);
			return mutationResult(result);
		},
	});

	pi.registerTool({
		name: "obsidian_move",
		label: "Obsidian Move",
		description: "Move a policy-admitted Markdown note after confirmation. The destination must remain available under its folder classification.",
		promptSnippet: "Move a policy-admitted Obsidian note after confirmation",
		parameters: Type.Object({
			source: Type.String({ description: "Existing vault-relative .md path" }),
			destination: Type.String({ description: "New vault-relative .md path" }),
			overwrite: Type.Optional(Type.Boolean({ default: false })),
		}),
		async execute(_id, params, signal, _update, ctx) {
			requireContract();
			const result = await currentRuntime().governed.move(
				params.source,
				params.destination,
				params.overwrite ?? false,
				modelIdentity(ctx),
				(message) => confirmMutation(ctx, message),
				signal,
			);
			return mutationResult(result);
		},
	});

	pi.registerTool({
		name: "obsidian_delete",
		label: "Obsidian Delete",
		description: "Delete a policy-admitted Markdown note after confirmation. The default uses the system trash.",
		promptSnippet: "Trash or permanently delete a policy-admitted Obsidian note after confirmation",
		parameters: Type.Object({
			path: Type.String({ description: "Vault-relative .md path" }),
			permanent: Type.Optional(Type.Boolean({ default: false, description: "Bypass the system trash" })),
		}),
		async execute(_id, params, signal, _update, ctx) {
			requireContract();
			await currentRuntime().governed.delete(
				params.path,
				params.permanent ?? false,
				modelIdentity(ctx),
				(message) => confirmMutation(ctx, message),
				signal,
			);
			return textResult("Obsidian note deleted.", { path: params.path, permanent: params.permanent ?? false });
		},
	});

	pi.registerTool({
		name: "obsidian_access_explain",
		label: "Obsidian Access Explain",
		description: "Check whether the active model may read a supplied Obsidian path without returning protected content.",
		parameters: Type.Object({ path: Type.String({ description: "Vault-relative note path supplied by the user" }) }),
		async execute(_id, params, signal, _update, ctx) {
			const result = params.path.toLowerCase().endsWith(".base")
				? await currentRuntime().governed.readBase(params.path, modelIdentity(ctx), signal)
				: await currentRuntime().governed.read(params.path, modelIdentity(ctx), signal);
			if (!result) return textResult("Access denied or object not found.", { allowed: false });
			return textResult(
				`Access allowed: ${result.decision.classification.dataClass}, ${result.decision.classification.agentAccess}.`,
				{ allowed: true, dataClass: result.decision.classification.dataClass, agentAccess: result.decision.classification.agentAccess },
			);
		},
	});
}
