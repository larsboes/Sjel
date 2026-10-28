import { describe, expect, test } from "bun:test";
import { ObsidianClient, ObsidianHttpError, vaultPath } from "../src/client.ts";
import type { RestRequest, RestTransport } from "../src/contracts.ts";

function transport(response: { status: number; body: string; contentType?: string }, seen?: RestRequest[]): RestTransport {
	return async (request) => {
		seen?.push(request);
		return { contentType: response.contentType ?? "application/json", ...response };
	};
}

describe("vault path encoding", () => {
	test("encodes each segment without encoding separators", () => {
		expect(vaultPath.encode("Projects/Sjel plan #1?.md")).toBe("Projects/Sjel%20plan%20%231%3F.md");
	});

	test("rejects traversal and empty interior segments", () => {
		expect(() => vaultPath.encode("Projects/../Secret.md")).toThrow("traversal");
		expect(() => vaultPath.encode("Projects//Note.md")).toThrow("traversal");
	});
});

describe("REST response contracts", () => {
	test("read uses the encoded vault endpoint", async () => {
		const seen: RestRequest[] = [];
		const client = new ObsidianClient(transport({ status: 200, body: "# Note", contentType: "text/markdown" }, seen), 50);
		expect(await client.read("Folder/My Note.md")).toBe("# Note");
		expect(seen[0]?.path).toBe("/vault/Folder/My%20Note.md");
	});

	test("read rejects a non-text response", async () => {
		const client = new ObsidianClient(transport({ status: 200, body: "binary", contentType: "application/octet-stream" }), 50);
		await expect(client.read("file.md")).rejects.toThrow("was not text");
	});

	test("Base reads admit the plugin's unknown-extension MIME marker only for .base paths", async () => {
		const seen: RestRequest[] = [];
		const client = new ObsidianClient(transport({ status: 200, body: "views: []", contentType: "false" }, seen), 50);
		expect(await client.readBase("Resources/Bases/Tasks.base")).toBe("views: []");
		expect(seen[0]?.headers?.Accept).toContain("text/yaml");
		await expect(client.readBase("attachment.pdf")).rejects.toThrow("requires a .base path");
	});

	test("list validates and caps the files array", async () => {
		const client = new ObsidianClient(transport({ status: 200, body: JSON.stringify({ files: ["a.md", "b.md", "c.md"] }) }), 2);
		expect(await client.list()).toEqual(["a.md", "b.md"]);
	});

	test("search posts to the simple-search endpoint and validates the response shape", async () => {
		const seen: RestRequest[] = [];
		const client = new ObsidianClient(transport({ status: 200, body: "{}" }, seen), 10);
		await expect(client.search("query")).rejects.toThrow("not an array");
		expect(seen[0]).toMatchObject({ method: "POST", path: "/search/simple/", query: { query: "query", contextLength: 100 } });
	});

	test("create sends Markdown with PUT", async () => {
		const seen: RestRequest[] = [];
		const client = new ObsidianClient(transport({ status: 204, body: "" }, seen), 10);
		await client.create("New note.md", "# New");
		expect(seen[0]).toMatchObject({
			method: "PUT",
			path: "/vault/New%20note.md",
			headers: { "Content-Type": "text/markdown; charset=utf-8" },
			body: "# New",
		});
	});

	test("exclusive create stages then moves without overwrite and cleans up a collision", async () => {
		const seen: RestRequest[] = [];
		const client = new ObsidianClient(async (request) => {
			seen.push(request);
			if (request.method === "MOVE") return { status: 409, contentType: "application/json", body: "destination exists" };
			return { status: 204, contentType: "", body: "" };
		}, 10);
		await expect(client.createExclusive("existing.md", "content")).rejects.toThrow("conflicted");
		expect(seen.map((request) => request.method)).toEqual(["PUT", "MOVE", "DELETE"]);
		expect(seen[1]?.headers?.Destination).toBe("existing.md");
		expect(seen[2]?.query).toEqual({ permanent: "true" });
	});

	test("patch sends a versioned markdown-patch 2 instruction", async () => {
		const seen: RestRequest[] = [];
		const client = new ObsidianClient(transport({ status: 200, body: "updated" }, seen), 10);
		expect(await client.patch("a.md", {
			targetType: "heading",
			target: ["Log"],
			operation: "append",
			content: "entry",
			ifMatch: "v1",
		})).toBe("updated");
		expect(seen[0]?.method).toBe("PATCH");
		expect(seen[0]?.headers?.["Content-Type"]).toBe("application/vnd.olrapi.patch-instruction+json");
		expect(JSON.parse(seen[0]?.body ?? "{}")).toMatchObject({ target: ["Log"], operation: "append", ifMatch: "v1" });
	});

	test("precondition failures are reported as conflicts without a response body", async () => {
		const client = new ObsidianClient(transport({ status: 412, body: "current note contents" }), 10);
		await expect(client.patch("a.md", {
			targetType: "heading",
			target: ["Log"],
			operation: "append",
			content: "entry",
			ifMatch: "stale",
		})).rejects.toThrow("conflicted");
	});

	test("HTTP failures do not repeat a server body", async () => {
		const client = new ObsidianClient(transport({ status: 500, body: "sensitive server detail" }), 10);
		try {
			await client.read("a.md");
			throw new Error("expected failure");
		} catch (error) {
			expect(error).toBeInstanceOf(ObsidianHttpError);
			expect(String(error)).not.toContain("sensitive server detail");
			expect(String(error)).toContain("HTTP 500");
		}
	});
});
