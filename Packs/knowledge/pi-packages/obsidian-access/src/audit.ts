import { createHash } from "node:crypto";
import { appendFileSync, chmodSync, mkdirSync } from "node:fs";
import { dirname } from "node:path";
import type { AccessDecision, ModelIdentity } from "./contracts.ts";

export type AuditOperation = "read" | "list" | "search" | "create" | "append" | "write" | "patch" | "copy" | "move" | "delete";

export class AuditLog {
	constructor(private readonly path: string | null) {}

	record(operation: AuditOperation, path: string, model: ModelIdentity, decision: AccessDecision): void {
		if (!this.path) return;
		mkdirSync(dirname(this.path), { recursive: true, mode: 0o700 });
		const row = {
			timestamp: new Date().toISOString(),
			model: `${model.provider}/${model.id}`,
			operation,
			allowed: decision.allowed,
			dataClass: decision.classification.dataClass,
			agentAccess: decision.classification.agentAccess,
			pathDigest: createHash("sha256").update(path.replaceAll("\\", "/")).digest("hex"),
		};
		appendFileSync(this.path, `${JSON.stringify(row)}\n`, { encoding: "utf8", mode: 0o600 });
		chmodSync(this.path, 0o600);
	}
}
