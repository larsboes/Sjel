import { expect, test } from "bun:test";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { AuditLog } from "../src/audit.ts";

const decision = {
	allowed: true,
	model: "local/qwen",
	classification: {
		dataClass: "c2" as const,
		agentAccess: "internal" as const,
		rationale: "fixture",
		method: "canonical" as const,
		valid: true,
	},
	reason: "allowed",
};

test("audit rows contain a digest and policy decision but not vault paths or content", () => {
	const path = join(mkdtempSync(join(tmpdir(), "obsidian-audit-")), "audit.jsonl");
	new AuditLog(path).record("read", "Atlas/People/Protected Name.md", { provider: "local", id: "qwen" }, decision);
	const written = readFileSync(path, "utf8");
	expect(written).not.toContain("Protected Name");
	expect(written).not.toContain("Atlas/People");
	expect(JSON.parse(written)).toMatchObject({
		model: "local/qwen",
		operation: "read",
		allowed: true,
		dataClass: "c2",
		agentAccess: "internal",
	});
	expect(JSON.parse(written).pathDigest).toHaveLength(64);
});
