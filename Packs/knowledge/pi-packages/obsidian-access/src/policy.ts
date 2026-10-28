import type {
	AccessConfig,
	AccessDecision,
	AgentAccess,
	Classification,
	DataClass,
	ModelIdentity,
	ModelRule,
	PathRule,
} from "./contracts.ts";

const CLASS_RANK: Record<DataClass, number> = { c0: 0, c1: 1, c2: 2, c3: 3 };

function wildcard(pattern: string): RegExp {
	let expression = "";
	for (let index = 0; index < pattern.length; index += 1) {
		const character = pattern[index] ?? "";
		if (character !== "*") {
			expression += character.replace(/[.+?^${}()|[\]\\]/g, "\\$&");
			continue;
		}
		if (pattern[index + 1] === "*") {
			if (pattern[index + 2] === "/") {
				expression += "(?:.*/)?";
				index += 2;
			} else {
				expression += ".*";
				index += 1;
			}
		} else {
			expression += "[^/]*";
		}
	}
	return new RegExp(`^${expression}$`, "i");
}

export function ruleForModel(config: AccessConfig, model: ModelIdentity): ModelRule {
	const key = `${model.provider}/${model.id}`;
	return (
		config.models.find((candidate) => wildcard(candidate.pattern).test(key)) ?? {
			pattern: "<default>",
			maxClass: config.defaultMaxClass,
			access: config.defaultAccess,
			processing: "external",
			c2Approved: false,
		}
	);
}

export function applyPathRules(classification: Classification, path: string, rules: PathRule[]): Classification {
	const normalized = path.replaceAll("\\", "/").replace(/^\/+|\/+$/g, "");
	let agentAccess = classification.agentAccess;
	const matched: string[] = [];
	for (const rule of rules) {
		if (!wildcard(rule.pattern).test(normalized)) continue;
		agentAccess = highestAccess(agentAccess, rule.access);
		matched.push(rule.pattern);
	}
	if (matched.length === 0) return classification;
	return {
		...classification,
		agentAccess,
		rationale: `${classification.rationale} Path policy ${matched.join(", ")} restricts agent access to ${agentAccess}.`.trim(),
	};
}

export function decideAccess(config: AccessConfig, model: ModelIdentity, classification: Classification): AccessDecision {
	const modelName = `${model.provider}/${model.id}`;
	if (!classification.valid) {
		return { allowed: false, model: modelName, classification, reason: "The note classification is invalid." };
	}
	if (classification.dataClass === "c3") {
		return { allowed: false, model: modelName, classification, reason: "C3 content never enters a prompt." };
	}

	const rule = ruleForModel(config, model);
	if (classification.dataClass === "c2" && !rule.c2Approved) {
		return { allowed: false, model: modelName, classification, reason: "C2 requires an explicitly reviewed processing boundary." };
	}
	if (CLASS_RANK[classification.dataClass] > CLASS_RANK[rule.maxClass]) {
		return {
			allowed: false,
			model: modelName,
			classification,
			reason: `The model rule allows at most ${rule.maxClass}.`,
		};
	}
	if (!rule.access.includes(classification.agentAccess)) {
		return {
			allowed: false,
			model: modelName,
			classification,
			reason: `The model rule does not allow ${classification.agentAccess} agent access.`,
		};
	}
	return { allowed: true, model: modelName, classification, reason: `Allowed by model rule ${rule.pattern}.` };
}

export function highestAccess(left: AgentAccess, right: AgentAccess): AgentAccess {
	const rank: Record<AgentAccess, number> = { standard: 0, internal: 1, confidential: 2 };
	return rank[left] >= rank[right] ? left : right;
}
