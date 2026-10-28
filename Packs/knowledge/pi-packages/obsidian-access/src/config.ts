import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { parse } from "smol-toml";
import {
	AGENT_ACCESS_LEVELS,
	DATA_CLASSES,
	type AccessConfig,
	type AgentAccess,
	type DataClass,
	type ModelRule,
	type PathRule,
} from "./contracts.ts";

const DEFAULT_CONFIG: AccessConfig = {
	version: 1,
	defaultMaxClass: "c0",
	defaultAccess: ["standard"],
	models: [],
	paths: [],
	transport: "auto",
	baseUrl: "http://127.0.0.1:27123",
	maxResponseBytes: 500_000,
	requestTimeoutSeconds: 120,
	maxCollectionItems: 50,
	classifierPath: null,
	egressTools: ["web_search", "source_check", "fetch_content", "get_search_content"],
	auditPath: null,
};

function defaultAuditPath(): string | null {
	const overlay = process.env.SJEL_OVERLAY_ROOT ?? process.env.SJEL_PERSONAL_ROOT ?? process.env.AXON_PERSONAL_ROOT;
	return overlay ? join(overlay, "logs", "obsidian-access.jsonl") : null;
}

function defaultClassifierPath(): string | null {
	if (process.env.SJEL_VAULT_CLASSIFIER) return resolve(process.env.SJEL_VAULT_CLASSIFIER);
	const root = process.env.SJEL_ROOT;
	if (!root) return null;
	for (const profile of ["release", "debug"]) {
		const candidate = join(root, "target", profile, "vault");
		if (existsSync(candidate)) return candidate;
	}
	return null;
}

function configPath(): string | null {
	if (process.env.OBSIDIAN_ACCESS_CONFIG) return resolve(process.env.OBSIDIAN_ACCESS_CONFIG);
	const overlay = process.env.SJEL_OVERLAY_ROOT ?? process.env.SJEL_PERSONAL_ROOT ?? process.env.AXON_PERSONAL_ROOT;
	return overlay ? join(overlay, "config", "obsidian-access.toml") : null;
}

function classValue(value: unknown, label: string): Exclude<DataClass, "c3"> {
	if (typeof value !== "string" || !DATA_CLASSES.includes(value as DataClass) || value === "c3") {
		throw new Error(`${label} must be c0, c1, or c2; C3 never enters a prompt`);
	}
	return value as Exclude<DataClass, "c3">;
}

function accessValues(value: unknown, label: string): AgentAccess[] {
	if (!Array.isArray(value) || value.length === 0 || value.some((entry) => typeof entry !== "string")) {
		throw new Error(`${label} must be a non-empty string array`);
	}
	const values = value as string[];
	for (const entry of values) {
		if (!AGENT_ACCESS_LEVELS.includes(entry as AgentAccess)) {
			throw new Error(`${label} contains unknown level ${JSON.stringify(entry)}`);
		}
	}
	return [...new Set(values)] as AgentAccess[];
}

export function loadConfig(path = configPath()): AccessConfig {
	if (path === null || !existsSync(path)) {
		return { ...structuredClone(DEFAULT_CONFIG), classifierPath: defaultClassifierPath(), auditPath: defaultAuditPath() };
	}
	const raw = parse(readFileSync(path, "utf8")) as Record<string, unknown>;
	if (raw.version !== 1) throw new Error(`${path}: version must be 1`);

	const transport = raw.transport ?? DEFAULT_CONFIG.transport;
	if (transport !== "auto" && transport !== "fetch" && transport !== "curl-exe") {
		throw new Error(`${path}: transport must be auto, fetch, or curl-exe`);
	}
	const modelsRaw = raw.models ?? [];
	if (!Array.isArray(modelsRaw)) throw new Error(`${path}: [[models]] must be an array`);
	const models: ModelRule[] = modelsRaw.map((entry, index) => {
		if (entry === null || typeof entry !== "object" || Array.isArray(entry)) {
			throw new Error(`${path}: models[${index}] must be a table`);
		}
		const row = entry as Record<string, unknown>;
		if (typeof row.pattern !== "string" || !row.pattern.includes("/")) {
			throw new Error(`${path}: models[${index}].pattern must be provider/model`);
		}
		const maxClass = classValue(row.max_class, `${path}: models[${index}].max_class`);
		const processing = row.processing ?? "external";
		if (processing !== "local" && processing !== "trusted-service" && processing !== "external") {
			throw new Error(`${path}: models[${index}].processing must be local, trusted-service, or external`);
		}
		const c2Approved = row.c2_approved ?? false;
		if (typeof c2Approved !== "boolean") throw new Error(`${path}: models[${index}].c2_approved must be true or false`);
		if (maxClass === "c2" && !c2Approved) {
			throw new Error(`${path}: models[${index}] grants C2 without explicit c2_approved = true`);
		}
		return {
			pattern: row.pattern,
			maxClass,
			access: accessValues(row.access, `${path}: models[${index}].access`),
			processing,
			c2Approved,
		};
	});

	const pathsRaw = raw.paths ?? [];
	if (!Array.isArray(pathsRaw)) throw new Error(`${path}: [[paths]] must be an array`);
	const paths: PathRule[] = pathsRaw.map((entry, index) => {
		if (entry === null || typeof entry !== "object" || Array.isArray(entry)) {
			throw new Error(`${path}: paths[${index}] must be a table`);
		}
		const row = entry as Record<string, unknown>;
		if (typeof row.pattern !== "string" || !row.pattern.trim()) {
			throw new Error(`${path}: paths[${index}].pattern must be a non-empty vault-relative glob`);
		}
		if (row.pattern.startsWith("/") || row.pattern.replaceAll("\\", "/").split("/").includes("..")) {
			throw new Error(`${path}: paths[${index}].pattern must stay vault-relative`);
		}
		if (row.access !== "internal" && row.access !== "confidential") {
			throw new Error(`${path}: paths[${index}].access must be internal or confidential`);
		}
		return { pattern: row.pattern.replaceAll("\\", "/"), access: row.access };
	});

	const baseUrl = raw.base_url ?? DEFAULT_CONFIG.baseUrl;
	if (typeof baseUrl !== "string" || !/^https?:\/\//.test(baseUrl)) {
		throw new Error(`${path}: base_url must be an HTTP(S) URL`);
	}
	const classifierValue = raw.classifier_path;
	if (classifierValue !== undefined && typeof classifierValue !== "string") {
		throw new Error(`${path}: classifier_path must be a string`);
	}
	const classifierPath = classifierValue === undefined ? defaultClassifierPath() : resolve(classifierValue);
	if (classifierPath !== null && !existsSync(classifierPath)) throw new Error(`${path}: classifier_path does not exist`);

	const auditValue = raw.audit_path;
	if (auditValue !== undefined && typeof auditValue !== "string") throw new Error(`${path}: audit_path must be a string`);
	const auditPath = auditValue === undefined ? defaultAuditPath() : resolve(auditValue);

	const egressValue = raw.egress_tools ?? DEFAULT_CONFIG.egressTools;
	if (!Array.isArray(egressValue) || egressValue.some((entry) => typeof entry !== "string" || !entry)) {
		throw new Error(`${path}: egress_tools must be a string array`);
	}
	const egressTools = [...new Set(egressValue as string[])];

	const maxResponseBytes = raw.max_response_bytes ?? DEFAULT_CONFIG.maxResponseBytes;
	const requestTimeoutSeconds = raw.request_timeout_seconds ?? DEFAULT_CONFIG.requestTimeoutSeconds;
	const maxCollectionItems = raw.max_collection_items ?? DEFAULT_CONFIG.maxCollectionItems;
	if (!Number.isInteger(maxResponseBytes) || Number(maxResponseBytes) < 1_024 || Number(maxResponseBytes) > 1_000_000) {
		throw new Error(`${path}: max_response_bytes must be an integer from 1024 to 1000000`);
	}
	if (!Number.isInteger(requestTimeoutSeconds) || Number(requestTimeoutSeconds) < 1 || Number(requestTimeoutSeconds) > 300) {
		throw new Error(`${path}: request_timeout_seconds must be an integer from 1 to 300`);
	}
	if (!Number.isInteger(maxCollectionItems) || Number(maxCollectionItems) < 1 || Number(maxCollectionItems) > 200) {
		throw new Error(`${path}: max_collection_items must be an integer from 1 to 200`);
	}

	return {
		version: 1,
		defaultMaxClass: classValue(raw.default_max_class ?? DEFAULT_CONFIG.defaultMaxClass, `${path}: default_max_class`),
		defaultAccess: accessValues(raw.default_access ?? DEFAULT_CONFIG.defaultAccess, `${path}: default_access`),
		models,
		paths,
		transport,
		baseUrl: baseUrl.replace(/\/$/, ""),
		maxResponseBytes: Number(maxResponseBytes),
		requestTimeoutSeconds: Number(requestTimeoutSeconds),
		maxCollectionItems: Number(maxCollectionItems),
		classifierPath,
		egressTools,
		auditPath,
	};
}
