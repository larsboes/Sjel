import { describe, expect, test } from "bun:test";
import { ObsidianClient } from "../src/client.ts";
import type { AccessConfig, RestTransport } from "../src/contracts.ts";
import { GovernedObsidian, outline } from "../src/governed.ts";

const config: AccessConfig = {
	version: 1,
	defaultMaxClass: "c0",
	defaultAccess: ["standard"],
	models: [{ pattern: "local/*", maxClass: "c2", access: ["standard", "internal", "confidential"], processing: "local", c2Approved: true }],
	paths: [],
	transport: "fetch",
	baseUrl: "http://localhost",
	maxResponseBytes: 50_000,
	requestTimeoutSeconds: 120,
	maxCollectionItems: 50,
	classifierPath: null,
	egressTools: ["web_search", "fetch_content"],
	auditPath: null,
};

function fixtureTransport(): RestTransport {
	const notes: Record<string, string> = {
		"/vault/open.md": "---\nclass: c1\n---\n# Open\nvisible token",
		"/vault/internal.md": "---\nclass: c1\nagent_access: internal\n---\n# Internal\ninternal token",
		"/vault/secret.md": "---\nclass: c3\n---\n# Secret\nsecret token",
	};
	return async ({ path, headers }) => {
		if (headers?.Accept === "application/vnd.olrapi.note+json" && (path === "/vault/open.md" || path === "/active/")) {
			return {
				status: 200,
				contentType: "application/json",
				body: JSON.stringify({
					path: "open.md",
					content: notes["/vault/open.md"],
					tags: ["visible"],
					frontmatter: { class: "c1" },
					stat: { ctime: 1, mtime: 2, size: 3 },
					links: [],
					backlinks: ["open.md", "internal.md", "secret.md"],
					unresolvedLinks: [],
				}),
			};
		}
		if (path === "/vault/") {
			return { status: 200, contentType: "application/json", body: JSON.stringify({ files: ["open.md", "internal.md", "secret.md"] }) };
		}
		if (path === "/search/simple/") {
			return {
				status: 200,
				contentType: "application/json",
				body: JSON.stringify([
					{ filename: "open.md", matches: [{ match: "visible token" }] },
					{ filename: "internal.md", matches: [{ match: "internal token" }] },
					{ filename: "secret.md", matches: [{ match: "secret token" }] },
				]),
			};
		}
		return notes[path]
			? { status: 200, contentType: "text/markdown", body: notes[path] }
			: { status: 404, contentType: "application/json", body: "{}" };
	};
}

describe("collection filtering", () => {
	test("a C1-only cloud model never sees internal or C3 names", async () => {
		const governed = new GovernedObsidian(new ObsidianClient(fixtureTransport(), 50), {
			...config,
			models: [{ pattern: "cloud/*", maxClass: "c1", access: ["standard"], processing: "external", c2Approved: false }],
		});
		const model = { provider: "cloud", id: "reviewed" };
		expect(await governed.list("", model)).toEqual(["open.md"]);
		expect(JSON.stringify(await governed.search("token", model, 100))).not.toMatch(/internal|secret/);
	});

	test("a confidential local model still cannot receive C3", async () => {
		const governed = new GovernedObsidian(new ObsidianClient(fixtureTransport(), 50), config);
		const model = { provider: "local", id: "qwen" };
		expect(await governed.list("", model)).toEqual(["open.md", "internal.md"]);
		expect(await governed.read("secret.md", model)).toBeNull();
	});

	test("backlinks are independently filtered before their paths are returned", async () => {
		const governed = new GovernedObsidian(new ObsidianClient(fixtureTransport(), 50), {
			...config,
			models: [{ pattern: "cloud/*", maxClass: "c1", access: ["standard"], processing: "external", c2Approved: false }],
		});
		const model = { provider: "cloud", id: "reviewed" };
		expect(await governed.backlinks("open.md", model)).toEqual(["open.md"]);
		expect(await governed.tags("open.md", model)).toEqual(["visible"]);
		expect(await governed.properties("open.md", model)).toEqual({ class: "c1" });
		expect((await governed.active(model))?.path).toBe("open.md");
	});

	test("Base reads return YAML only after path admission and include validation", async () => {
		const client = new ObsidianClient(async () => ({
			status: 200,
			contentType: "application/octet-stream",
			body: "views:\n  - type: table\n    name: Tasks\n",
		}), 50);
		const result = await new GovernedObsidian(client, config).readBase("Resources/Bases/Tasks.base", { provider: "local", id: "qwen" });
		expect(result?.validation).toEqual({ valid: true, diagnostics: [] });
		expect(result?.content).toContain("name: Tasks");
	});
});

describe("mutations", () => {
	const model = { provider: "local", id: "qwen" };

	function mutableNote(initial: string) {
		let note = initial;
		const requests: Array<{ method: string; body?: string }> = [];
		const mutableTransport: RestTransport = async (request) => {
			requests.push({ method: request.method, body: request.body });
			if (request.method === "GET" && request.headers?.Accept === "application/vnd.olrapi.document-map+json") {
				return { status: 200, contentType: "application/json", body: JSON.stringify({ version: "v1", headings: {}, blocks: [], frontmatterFields: ["class"] }) };
			}
			if (request.method === "GET") return { status: 200, contentType: "text/markdown", body: note };
			if (request.method === "COPY" || request.method === "MOVE" || request.method === "DELETE") {
				return { status: 204, contentType: "", body: "" };
			}
			if (request.method === "PATCH") {
				const instruction = JSON.parse(request.body ?? "{}") as { operation?: string; content?: string; ifMatch?: string };
				if (instruction.ifMatch !== "v1") return { status: 412, contentType: "application/json", body: "{}" };
				if (instruction.operation === "append") note += `\n${instruction.content ?? ""}`;
				if (instruction.operation === "replace") note = instruction.content ?? note;
				return { status: 200, contentType: "text/markdown", body: note };
			}
			return { status: 500, contentType: "application/json", body: "{}" };
		};
		return { governed: new GovernedObsidian(new ObsidianClient(mutableTransport, 50), config), requests, content: () => note };
	}

	test("create refuses overwrite and creates an absent admitted note", async () => {
		const seen: string[] = [];
		let exists = false;
		const client = new ObsidianClient(async (request) => {
			seen.push(request.method);
			if (request.method === "GET") {
				return exists
					? { status: 200, contentType: "text/markdown", body: "---\nclass: c1\n---\nexisting" }
					: { status: 404, contentType: "application/json", body: "{}" };
			}
			exists = true;
			return { status: 204, contentType: "", body: "" };
		}, 50);
		const governed = new GovernedObsidian(client, config);
		await governed.create("new.md", "---\nclass: c1\n---\nnew", model);
		expect(seen).toEqual(["GET", "PUT", "MOVE"]);
		await expect(governed.create("new.md", "---\nclass: c1\n---\nother", model)).rejects.toThrow("already exists");
		expect(seen.filter((method) => method === "PUT")).toHaveLength(1);
	});

	test("append uses the document version and keeps the original on conflict", async () => {
		const fixture = mutableNote("---\nclass: c1\n---\n# Log");
		await fixture.governed.append("log.md", "entry", model, async () => true);
		expect(fixture.content()).toContain("entry");
		const patch = fixture.requests.find((request) => request.method === "PATCH");
		expect(JSON.parse(patch?.body ?? "{}").ifMatch).toBe("v1");
	});

	test("whole-note replacement stops before PATCH when confirmation is refused", async () => {
		const fixture = mutableNote("---\nclass: c1\n---\noriginal");
		let prompt = "";
		await expect(fixture.governed.write("note.md", "---\nclass: c1\n---\nreplacement", model, async (message) => {
			prompt = message;
			return false;
		})).rejects.toThrow("cancelled");
		expect(prompt).toContain("`note.md`");
		expect(fixture.requests.some((request) => request.method === "PATCH")).toBe(false);
		expect(fixture.content()).toContain("original");
	});

	test("targeted replacement requires confirmation", async () => {
		const fixture = mutableNote("---\nclass: c1\n---\n# Log\nold");
		let prompt = "";
		await expect(fixture.governed.patch("note.md", {
			targetType: "heading",
			target: ["Log"],
			operation: "replace",
			content: "new",
		}, model, async (message) => {
			prompt = message;
			return false;
		})).rejects.toThrow("cancelled");
		expect(prompt).toContain("`note.md`");
		expect(fixture.requests.some((request) => request.method === "PATCH")).toBe(false);
	});

	test("copy is non-destructive while move and delete require confirmation", async () => {
		const fixture = mutableNote("---\nclass: c1\n---\nbody");
		const prompts: string[] = [];
		const refuse = async (message: string) => {
			prompts.push(message);
			return false;
		};
		await fixture.governed.copy("note.md", "copy.md", false, model, refuse);
		expect(fixture.requests.some((request) => request.method === "COPY")).toBe(true);
		await expect(fixture.governed.move("note.md", "moved.md", false, model, refuse)).rejects.toThrow("cancelled");
		await expect(fixture.governed.delete("note.md", false, model, refuse)).rejects.toThrow("cancelled");
		expect(prompts).toEqual([
			"Move `note.md` to `moved.md`?",
			"Move `note.md` to the system trash?",
		]);
		expect(fixture.requests.some((request) => request.method === "MOVE" || request.method === "DELETE")).toBe(false);
	});
});

describe("outline", () => {
	test("ignores headings inside fenced code", () => {
		expect(outline("# One\n```md\n# Not a heading\n```\n## Two\n")).toEqual(["# One", "## Two"]);
	});
});
