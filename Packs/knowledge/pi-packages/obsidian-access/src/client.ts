import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import type { AccessConfig, DocumentMap, NoteMetadata, PatchInstruction, RestRequest, RestResponse, RestTransport } from "./contracts.ts";

const STATUS_MARKER = "\n__SJEL_OBSIDIAN_STATUS__:";

export class ObsidianHttpError extends Error {
	constructor(
		message: string,
		readonly status?: number,
	) {
		super(message);
		this.name = "ObsidianHttpError";
	}
}

function encodePath(path: string): string {
	const normalized = path.replaceAll("\\", "/").replace(/^\/+|\/+$/g, "");
	if (!normalized) return "";
	const segments = normalized.split("/");
	if (segments.some((segment) => segment === "." || segment === ".." || segment.length === 0)) {
		throw new Error("Obsidian paths must be vault-relative and cannot contain traversal segments");
	}
	return segments.map(encodeURIComponent).join("/");
}

function requestUrl(baseUrl: string, request: RestRequest): string {
	const url = new URL(request.path, `${baseUrl}/`);
	for (const [key, value] of Object.entries(request.query ?? {})) {
		if (value !== undefined) url.searchParams.set(key, String(value));
	}
	return url.toString();
}

function assertSize(text: string, maxBytes: number): void {
	const bytes = Buffer.byteLength(text);
	if (bytes > maxBytes) throw new ObsidianHttpError(`Obsidian response exceeded the ${maxBytes}-byte limit`);
}

function curlConfig(url: string, apiKey: string, request: RestRequest, timeoutSeconds: number): string {
	const quote = (value: string) =>
		value.replaceAll("\\", "\\\\").replaceAll('"', '\\"').replaceAll("\r", "\\r").replaceAll("\n", "\\n").replaceAll("\t", "\\t");
	const lines = [
		`url = "${quote(url)}"`,
		`request = "${request.method}"`,
		`header = "Authorization: Bearer ${quote(apiKey)}"`,
	];
	for (const [name, value] of Object.entries(request.headers ?? {})) lines.push(`header = "${quote(name)}: ${quote(value)}"`);
	if (request.body !== undefined) lines.push(`data-raw = "${quote(request.body)}"`);
	return [
		...lines,
		"silent",
		"show-error",
		"noproxy = \"*\"",
		`max-time = ${timeoutSeconds}`,
		`write-out = "${STATUS_MARKER.replace("\n", "\\n")}%{http_code}|%{content_type}"`,
		"",
	].join("\n");
}

export function createCurlExeTransport(config: AccessConfig, apiKey: string): RestTransport {
	return async (request) => {
		const url = requestUrl(config.baseUrl, request);
		if (request.body !== undefined) assertSize(request.body, config.maxResponseBytes);
		const child = spawn("curl.exe", ["--config", "-"], { stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
		let stdout = "";
		let stderr = "";
		let oversized = false;
		const abort = () => child.kill();
		request.signal?.addEventListener("abort", abort, { once: true });
		child.stdout.setEncoding("utf8");
		child.stderr.setEncoding("utf8");
		child.stdout.on("data", (chunk: string) => {
			stdout += chunk;
			if (Buffer.byteLength(stdout) > config.maxResponseBytes + 1_024) {
				oversized = true;
				child.kill();
			}
		});
		child.stderr.on("data", (chunk: string) => {
			stderr += chunk;
		});
		child.stdin.end(curlConfig(url, apiKey, request, config.requestTimeoutSeconds));
		const code = await new Promise<number | null>((resolve, reject) => {
			child.once("error", reject);
			child.once("close", resolve);
		});
		request.signal?.removeEventListener("abort", abort);
		if (request.signal?.aborted) throw new ObsidianHttpError("Obsidian request cancelled");
		if (oversized) throw new ObsidianHttpError(`Obsidian response exceeded the ${config.maxResponseBytes}-byte limit`);
		if (code !== 0) throw new ObsidianHttpError(`Obsidian transport failed: ${stderr.trim() || `curl.exe exited ${code}`}`);
		const marker = stdout.lastIndexOf(STATUS_MARKER);
		if (marker === -1) throw new ObsidianHttpError("Obsidian transport returned no HTTP status");
		const body = stdout.slice(0, marker);
		assertSize(body, config.maxResponseBytes);
		const trailer = stdout.slice(marker + STATUS_MARKER.length);
		const separator = trailer.indexOf("|");
		if (separator === -1) throw new ObsidianHttpError("Obsidian transport returned an invalid HTTP status trailer");
		const status = Number(trailer.slice(0, separator));
		const contentType = trailer.slice(separator + 1).trim();
		return { status, contentType, body };
	};
}

export function createFetchTransport(config: AccessConfig, apiKey: string): RestTransport {
	return async (request) => {
		if (request.body !== undefined) assertSize(request.body, config.maxResponseBytes);
		const controller = new AbortController();
		const timeout = setTimeout(() => controller.abort(), config.requestTimeoutSeconds * 1_000);
		const abort = () => controller.abort();
		request.signal?.addEventListener("abort", abort, { once: true });
		try {
			const response = await fetch(requestUrl(config.baseUrl, request), {
				method: request.method,
				headers: { Authorization: `Bearer ${apiKey}`, ...request.headers },
				body: request.body,
				signal: controller.signal,
			});
			const body = await response.text();
			assertSize(body, config.maxResponseBytes);
			return { status: response.status, contentType: response.headers.get("content-type") ?? "", body };
		} catch (error) {
			if (controller.signal.aborted) {
				throw new ObsidianHttpError(request.signal?.aborted ? "Obsidian request cancelled" : "Obsidian request timed out");
			}
			throw error;
		} finally {
			clearTimeout(timeout);
			request.signal?.removeEventListener("abort", abort);
		}
	};
}

export function createTransport(config: AccessConfig, apiKey: string): RestTransport {
	const selected = config.transport === "auto" ? (process.env.WSL_DISTRO_NAME ? "curl-exe" : "fetch") : config.transport;
	return selected === "curl-exe" ? createCurlExeTransport(config, apiKey) : createFetchTransport(config, apiKey);
}

export class ObsidianClient {
	constructor(
		private readonly transport: RestTransport,
		private readonly maxItems: number,
	) {}

	private async request(request: RestRequest): Promise<RestResponse> {
		const response = await this.transport(request);
		if (response.status === 404) throw new ObsidianHttpError("Obsidian object not found", 404);
		if (response.status === 401 || response.status === 403) throw new ObsidianHttpError("Obsidian authentication failed", response.status);
		if (response.status === 409 || response.status === 412) {
			throw new ObsidianHttpError("Obsidian write conflicted with the current note", response.status);
		}
		if (response.status < 200 || response.status >= 300) {
			throw new ObsidianHttpError(`Obsidian returned HTTP ${response.status}`, response.status);
		}
		return response;
	}

	private get(path: string, query: RestRequest["query"], signal?: AbortSignal, headers?: Record<string, string>): Promise<RestResponse> {
		return this.request({ method: "GET", path, query, headers, signal });
	}

	async read(path: string, signal?: AbortSignal): Promise<string> {
		const encoded = encodePath(path);
		if (!encoded) throw new Error("obsidian_read requires a file path");
		const response = await this.get(`/vault/${encoded}`, undefined, signal);
		if (response.contentType && !response.contentType.toLowerCase().startsWith("text/")) {
			throw new ObsidianHttpError("Obsidian note response was not text");
		}
		return response.body;
	}

	async readBase(path: string, signal?: AbortSignal): Promise<string> {
		if (!path.toLowerCase().endsWith(".base")) throw new Error("obsidian_base_read requires a .base path");
		const encoded = encodePath(path);
		const response = await this.get(`/vault/${encoded}`, undefined, signal, { Accept: "text/yaml, application/yaml, text/plain" });
		const contentType = response.contentType.toLowerCase().split(";", 1)[0]?.trim() ?? "";
		const admittedTypes = new Set(["", "false", "text/plain", "text/yaml", "application/yaml", "application/x-yaml", "application/octet-stream"]);
		if (!contentType.startsWith("text/") && !admittedTypes.has(contentType)) {
			throw new ObsidianHttpError("Obsidian Base response was not YAML text");
		}
		return response.body;
	}

	async list(path = "", signal?: AbortSignal): Promise<string[]> {
		const encoded = encodePath(path);
		const endpoint = encoded ? `/vault/${encoded}/` : "/vault/";
		const response = await this.get(endpoint, undefined, signal);
		let value: unknown;
		try {
			value = JSON.parse(response.body);
		} catch {
			throw new ObsidianHttpError("Obsidian directory response was not JSON");
		}
		const files = (value as { files?: unknown })?.files;
		if (!Array.isArray(files) || files.some((entry) => typeof entry !== "string")) {
			throw new ObsidianHttpError("Obsidian directory response has no string files array");
		}
		return (files as string[]).slice(0, this.maxItems);
	}

	async readTarget(
		path: string,
		targetType: "heading" | "block" | "frontmatter",
		target: string | string[],
		scope: "content" | "marker" | "markerAndContent" = "content",
		signal?: AbortSignal,
	): Promise<string> {
		const encoded = encodePath(path);
		if (!encoded) throw new Error("A targeted read requires a file path");
		const parts = Array.isArray(target) ? target : [target];
		if (parts.length === 0 || parts.some((part) => !part)) throw new Error("A targeted read requires a non-empty target");
		const suffix = [targetType, ...parts].map(encodeURIComponent).join("/");
		const response = await this.get(`/vault/${encoded}/${suffix}`, undefined, signal, { "Target-Scope": scope });
		if (targetType !== "frontmatter") {
			if (response.contentType && !response.contentType.toLowerCase().startsWith("text/")) {
				throw new ObsidianHttpError("Obsidian targeted note response was not text");
			}
			return response.body;
		}
		try {
			return JSON.stringify(JSON.parse(response.body), null, 2);
		} catch {
			throw new ObsidianHttpError("Obsidian frontmatter target was not JSON");
		}
	}

	async metadata(path: string | null, signal?: AbortSignal): Promise<NoteMetadata> {
		const endpoint = path === null ? "/active/" : `/vault/${encodePath(path)}`;
		const response = await this.get(endpoint, undefined, signal, { Accept: "application/vnd.olrapi.note+json" });
		let value: unknown;
		try {
			value = JSON.parse(response.body);
		} catch {
			throw new ObsidianHttpError("Obsidian note metadata was not JSON");
		}
		const note = value as Partial<NoteMetadata>;
		if (typeof note.path !== "string" || typeof note.content !== "string" || !Array.isArray(note.tags) ||
			!Array.isArray(note.backlinks) || !Array.isArray(note.links) || !Array.isArray(note.unresolvedLinks) ||
			note.frontmatter === null || typeof note.frontmatter !== "object" || Array.isArray(note.frontmatter)) {
			throw new ObsidianHttpError("Obsidian note metadata has an invalid shape");
		}
		for (const values of [note.tags, note.backlinks, note.links, note.unresolvedLinks]) {
			if (values.some((entry) => typeof entry !== "string")) throw new ObsidianHttpError("Obsidian note metadata contains a non-string list value");
		}
		return note as NoteMetadata;
	}

	async documentMap(path: string, signal?: AbortSignal): Promise<DocumentMap> {
		const encoded = encodePath(path);
		if (!encoded) throw new Error("A document map requires a file path");
		const response = await this.get(`/vault/${encoded}`, undefined, signal, { Accept: "application/vnd.olrapi.document-map+json" });
		let value: unknown;
		try {
			value = JSON.parse(response.body);
		} catch {
			throw new ObsidianHttpError("Obsidian document map was not JSON");
		}
		const map = value as Partial<DocumentMap>;
		if (typeof map?.version !== "string" || !Array.isArray(map.blocks) || !Array.isArray(map.frontmatterFields)) {
			throw new ObsidianHttpError("Obsidian document map has an invalid shape");
		}
		return map as DocumentMap;
	}

	async create(path: string, content: string, signal?: AbortSignal): Promise<void> {
		const encoded = encodePath(path);
		if (!encoded) throw new Error("obsidian_create requires a file path");
		await this.request({
			method: "PUT",
			path: `/vault/${encoded}`,
			headers: { "Content-Type": "text/markdown; charset=utf-8" },
			body: content,
			signal,
		});
	}

	async move(source: string, destination: string, overwrite = false, signal?: AbortSignal): Promise<void> {
		await this.relocate("MOVE", source, destination, overwrite, signal);
	}

	async copy(source: string, destination: string, overwrite = false, signal?: AbortSignal): Promise<void> {
		await this.relocate("COPY", source, destination, overwrite, signal);
	}

	private async relocate(method: "MOVE" | "COPY", source: string, destination: string, overwrite: boolean, signal?: AbortSignal): Promise<void> {
		const encodedSource = encodePath(source);
		const normalizedDestination = destination.replaceAll("\\", "/");
		encodePath(normalizedDestination);
		const encodedDestination = encodeURIComponent(normalizedDestination);
		if (!encodedSource || !encodedDestination) throw new Error("Obsidian relocation requires source and destination paths");
		await this.request({
			method,
			path: `/vault/${encodedSource}`,
			headers: { Destination: encodedDestination, "Allow-Overwrite": String(overwrite) },
			signal,
		});
	}

	async delete(path: string, permanent: boolean, signal?: AbortSignal): Promise<void> {
		const encoded = encodePath(path);
		if (!encoded) throw new Error("Obsidian delete requires a file path");
		await this.request({ method: "DELETE", path: `/vault/${encoded}`, query: { permanent: String(permanent) }, signal });
	}

	async createExclusive(path: string, content: string, signal?: AbortSignal): Promise<void> {
		const stagingPath = `Resources/Inbox/Sjel Access Staging ${randomUUID()}.md`;
		await this.create(stagingPath, content, signal);
		try {
			await this.move(stagingPath, path, false, signal);
		} catch (error) {
			try {
				await this.delete(stagingPath, true);
			} catch {
				throw new ObsidianHttpError("Obsidian create failed and its staging note could not be removed");
			}
			throw error;
		}
	}

	async patch(path: string, instruction: PatchInstruction, signal?: AbortSignal): Promise<string> {
		const encoded = encodePath(path);
		if (!encoded) throw new Error("obsidian_patch requires a file path");
		const response = await this.request({
			method: "PATCH",
			path: `/vault/${encoded}`,
			headers: { "Content-Type": "application/vnd.olrapi.patch-instruction+json" },
			body: JSON.stringify(instruction),
			signal,
		});
		return response.body;
	}

	async search(query: string, contextLength = 100, signal?: AbortSignal): Promise<unknown[]> {
		if (!query.trim()) throw new Error("obsidian_search requires a non-empty query");
		const response = await this.request({ method: "POST", path: "/search/simple/", query: { query, contextLength }, signal });
		let value: unknown;
		try {
			value = JSON.parse(response.body);
		} catch {
			throw new ObsidianHttpError("Obsidian search response was not JSON");
		}
		if (!Array.isArray(value)) throw new ObsidianHttpError("Obsidian search response was not an array");
		return value.slice(0, this.maxItems);
	}
}

export const vaultPath = { encode: encodePath };
