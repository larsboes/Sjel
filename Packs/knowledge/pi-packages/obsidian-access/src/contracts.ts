export const DATA_CLASSES = ["c0", "c1", "c2", "c3"] as const;
export type DataClass = (typeof DATA_CLASSES)[number];

export const AGENT_ACCESS_LEVELS = ["standard", "internal", "confidential"] as const;
export type AgentAccess = (typeof AGENT_ACCESS_LEVELS)[number];

export interface Classification {
	dataClass: DataClass;
	agentAccess: AgentAccess;
	rationale: string;
	method: "canonical" | "frontmatter" | "conservative-fallback";
	valid: boolean;
}

export interface PathRule {
	pattern: string;
	access: Exclude<AgentAccess, "standard">;
}

export type ProcessingBoundary = "local" | "trusted-service" | "external";

export interface ModelRule {
	pattern: string;
	maxClass: Exclude<DataClass, "c3">;
	access: AgentAccess[];
	processing: ProcessingBoundary;
	c2Approved: boolean;
}

export interface AccessConfig {
	version: 1;
	defaultMaxClass: Exclude<DataClass, "c3">;
	defaultAccess: AgentAccess[];
	models: ModelRule[];
	paths: PathRule[];
	transport: "auto" | "fetch" | "curl-exe";
	baseUrl: string;
	maxResponseBytes: number;
	requestTimeoutSeconds: number;
	maxCollectionItems: number;
	classifierPath: string | null;
	egressTools: string[];
	auditPath: string | null;
}

export interface ModelIdentity {
	provider: string;
	id: string;
}

export interface AccessDecision {
	allowed: boolean;
	model: string;
	classification: Classification;
	reason: string;
}

export interface RestResponse {
	status: number;
	contentType: string;
	body: string;
}

export interface RestRequest {
	method: "GET" | "PUT" | "PATCH" | "POST" | "DELETE" | "MOVE" | "COPY";
	path: string;
	query?: Record<string, string | number | undefined>;
	headers?: Record<string, string>;
	body?: string;
	signal?: AbortSignal;
}

export interface NoteMetadata {
	path: string;
	content: string;
	tags: string[];
	backlinks: string[];
	links: string[];
	unresolvedLinks: string[];
	frontmatter: Record<string, unknown>;
	stat: { ctime: number; mtime: number; size: number };
}

export interface DocumentMap {
	version: string;
	headings: unknown;
	blocks: string[];
	frontmatterFields: string[];
}

export interface PatchInstruction {
	targetType: "heading" | "block" | "frontmatter";
	target: string | string[] | null;
	operation: "replace" | "prepend" | "append" | "delete";
	scope?: "content" | "marker" | "markerAndContent";
	content?: string;
	value?: unknown;
	within?: number;
	createTargetIfMissing?: boolean;
	ifMatch: string;
}

export type RestTransport = (request: RestRequest) => Promise<RestResponse>;
