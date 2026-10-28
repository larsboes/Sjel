import type { AccessConfig, AccessDecision, AgentAccess, Classification, DataClass, ModelIdentity } from "./contracts.ts";
import { decideAccess, highestAccess } from "./policy.ts";

export interface ClearanceValue {
	dataClass: Exclude<DataClass, "c3">;
	agentAccess: AgentAccess;
}

const CLASS_RANK: Record<ClearanceValue["dataClass"], number> = { c0: 0, c1: 1, c2: 2 };

export class SessionClearance {
	private current: ClearanceValue | null = null;

	get value(): ClearanceValue | null {
		return this.current ? { ...this.current } : null;
	}

	reset(): void {
		this.current = null;
	}

	restore(value: unknown): boolean {
		if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
		const candidate = value as { dataClass?: unknown; agentAccess?: unknown };
		if (candidate.dataClass !== "c0" && candidate.dataClass !== "c1" && candidate.dataClass !== "c2") return false;
		if (candidate.agentAccess !== "standard" && candidate.agentAccess !== "internal" && candidate.agentAccess !== "confidential") return false;
		this.current = { dataClass: candidate.dataClass, agentAccess: candidate.agentAccess };
		return true;
	}

	raise(classification: Classification): boolean {
		if (classification.dataClass === "c3") return false;
		const next: ClearanceValue = {
			dataClass:
				this.current && CLASS_RANK[this.current.dataClass] >= CLASS_RANK[classification.dataClass]
					? this.current.dataClass
					: classification.dataClass,
			agentAccess: this.current ? highestAccess(this.current.agentAccess, classification.agentAccess) : classification.agentAccess,
		};
		if (this.current?.dataClass === next.dataClass && this.current.agentAccess === next.agentAccess) return false;
		this.current = next;
		return true;
	}

	decide(config: AccessConfig, model: ModelIdentity): AccessDecision | null {
		if (!this.current) return null;
		return decideAccess(config, model, {
			...this.current,
			rationale: "This session previously received Obsidian content at this clearance.",
			method: "conservative-fallback",
			valid: true,
		});
	}
}
