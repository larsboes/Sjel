import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { parse } from "smol-toml";

const MAX_CONTRACT_BYTES = 50_000;

export const NO_CONTRACT_MESSAGE =
	"No vault contract is configured for this machine. Follow the vault's existing conventions; writes are not gated on a contract.";

export interface KnowledgeSettings {
	profile: string | null;
	/** Vault-relative contract note, or null when this machine configures none. */
	contractPath: string | null;
}

export interface KnowledgeContract {
	profile: string | null;
	path: string;
	content: string;
	digest: string;
}

function overlayRoot(): string | null {
	const value = process.env.SJEL_OVERLAY_ROOT ?? process.env.SJEL_PERSONAL_ROOT ?? process.env.AXON_PERSONAL_ROOT;
	return value ? resolve(value) : null;
}

function knowledgeConfigPath(configPath?: string): string | null {
	if (configPath) return resolve(configPath);
	const overlay = overlayRoot();
	return overlay ? join(overlay, "config", "knowledge.toml") : null;
}

/** Reads `config/knowledge.toml`. A missing overlay or file means no profile and no contract. */
export function loadKnowledgeSettings(configPath?: string): KnowledgeSettings {
	const path = knowledgeConfigPath(configPath);
	if (path === null || !existsSync(path)) return { profile: null, contractPath: null };
	const raw = parse(readFileSync(path, "utf8")) as Record<string, unknown>;
	if (raw.version !== undefined && raw.version !== 1) throw new Error(`${path}: version must be 1`);
	if (raw.profile !== undefined && (typeof raw.profile !== "string" || !raw.profile.trim())) {
		throw new Error(`${path}: profile must be a non-empty string`);
	}
	const value = raw.contract_path;
	if (value === undefined) return { profile: (raw.profile as string | undefined) ?? null, contractPath: null };
	if (typeof value !== "string" || !value.trim()) throw new Error(`${path}: contract_path must be a non-empty string`);
	const contractPath = value.trim().replaceAll("\\", "/");
	if (contractPath.startsWith("/") || contractPath.split("/").includes("..") || !contractPath.toLowerCase().endsWith(".md")) {
		throw new Error(`${path}: contract_path must be a vault-relative .md path`);
	}
	return { profile: (raw.profile as string | undefined) ?? null, contractPath };
}

/** Returns null when no contract is configured; fails closed when a configured contract is unusable. */
export async function loadKnowledgeContract(
	readContract: (path: string) => Promise<string>,
	configPath?: string,
): Promise<KnowledgeContract | null> {
	const { profile, contractPath } = loadKnowledgeSettings(configPath);
	if (contractPath === null) return null;
	let content: string;
	try {
		content = await readContract(contractPath);
	} catch (error) {
		const reason = error instanceof Error ? error.message : String(error);
		throw new Error(`${contractPath}: configured vault contract could not be read: ${reason}`);
	}
	if (Buffer.byteLength(content) > MAX_CONTRACT_BYTES) {
		throw new Error(`${contractPath}: contract exceeds the ${MAX_CONTRACT_BYTES}-byte limit`);
	}
	if (!content.trim()) throw new Error(`${contractPath}: contract is empty`);
	return { profile, path: contractPath, content, digest: createHash("sha256").update(content).digest("hex") };
}
