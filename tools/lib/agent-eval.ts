import { createHash } from "node:crypto";

const MAX_TEXT = 256_000;
const MAX_FILES = 64;
const MAX_CALLS = 128;
const MAX_REPORT_TEXT = 1_000_000;
const MAX_SUITE_TEXT = 4_000_000;
const TOOLS = new Set(["read", "list", "search", "edit", "probe", "sandbox_python"]);

export interface ToolCall {
  tool: string;
  args: Record<string, unknown>;
}

export interface Replay {
  case_id: string;
  calls: ToolCall[];
  answer: string;
}

interface Claim {
  id: string;
  text: string;
  sources: string[];
  critical: boolean;
  evidence_kind?: string;
}

interface Policy {
  tools: string[];
  filesystem: string;
  network: "deny";
  subprocess: string;
  python_imports?: string;
  probe_max_attempts?: number;
}

export interface EvalCase {
  id: string;
  profile: string;
  scenario: string;
  prompt: string;
  input: {
    files: Record<string, { source_id: string; content: string; sha256: string }>;
    custom_tools?: unknown[];
  };
  policy: Policy;
  oracle: {
    claims: Claim[];
    checks: string[];
    critical_failures: string[];
    probe?: {
      responses: Record<string, number>;
      required_predictions: Record<string, { H1: number; H2: number }>;
      target_input: number;
      target_output: number;
    };
    [key: string]: unknown;
  };
  provenance?: Record<string, unknown>;
}

export interface Suite {
  schema_version: 1;
  split: "development";
  data_class: "c0";
  label_status: string;
  results_status: "not_run";
  cases: EvalCase[];
}

function object(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label}: expected an object`);
  const proto = Object.getPrototypeOf(value);
  if (proto !== Object.prototype && proto !== null) throw new Error(`${label}: expected a plain object`);
  return value as Record<string, unknown>;
}

function text(value: unknown, label: string, empty = false): string {
  if (typeof value !== "string" || (!empty && !value.length) || value.length > MAX_TEXT) {
    throw new Error(`${label}: expected ${empty ? "bounded" : "nonempty bounded"} text`);
  }
  return value;
}

function strings(value: unknown, label: string): string[] {
  if (!Array.isArray(value) || value.length > 128) throw new Error(`${label}: expected a bounded array`);
  return value.map((v) => text(v, label));
}

function relativePath(value: unknown): string {
  const path = text(value, "path");
  if (path.length > 512) throw new Error("path exceeds 512 characters");
  if (!/^[A-Za-z0-9_.\-/]+$/.test(path) || path.startsWith("/") || path.split("/").some((p) => !p || p === "." || p === "..")) {
    throw new Error("path must be a normalized relative file path");
  }
  return path;
}

/** Hash fixture bytes and virtual artifact contents without reading host files. */
export function digest(content: string): string {
  return createHash("sha256").update(content).digest("hex");
}

function validatePolicy(raw: unknown, id: string): void {
  const p = object(raw, "policy");
  const known = new Set(["tools", "filesystem", "network", "subprocess", "python_imports", "probe_max_attempts"]);
  for (const key of Object.keys(p)) if (!known.has(key)) throw new Error(`unknown policy field: ${key}`);
  const tools = strings(p.tools, "policy.tools");
  if (new Set(tools).size !== tools.length || tools.some((t) => !TOOLS.has(t))) throw new Error("unsupported or duplicate policy tool");
  if (p.network !== "deny") throw new Error("solver network policy must deny access");
  if (p.filesystem !== "read_only" && !(id === "C02" && p.filesystem === "only projection.py writable; all other inputs read-only")) {
    throw new Error("unsupported filesystem policy");
  }
  if (tools.includes("edit") !== (id === "C02" && p.filesystem !== "read_only")) throw new Error("edit policy does not match writable files");
  const pythonMode = id === "C02" ? "sandbox_python only; no child processes" : id === "R01" ? "sandbox_python arithmetic only; no child processes" : undefined;
  if (typeof p.subprocess !== "string" || (p.subprocess !== "deny" && (pythonMode === undefined || p.subprocess !== pythonMode))) throw new Error("unsupported subprocess policy");
  if (tools.includes("sandbox_python") !== (p.subprocess !== "deny")) throw new Error("Python tool does not match subprocess policy");
  if (tools.includes("sandbox_python") || p.python_imports !== undefined) {
    const expected = id === "C02" ? "standard library only; confined to the disposable workspace" : id === "R01" ? "standard library only; no filesystem mutation" : undefined;
    if (expected === undefined || p.python_imports !== expected) throw new Error("unsupported Python import policy");
  }
  if (tools.includes("probe")) {
    if (id !== "R04" || p.probe_max_attempts !== 1) throw new Error("unsupported probe policy");
  } else if (p.probe_max_attempts !== undefined) throw new Error("probe budget without probe tool");
}

/** Validate the development catalogue before exporting any solver inputs. Throws on unsupported contracts. */
export function validateSuite(value: unknown): Suite {
  const suite = object(value, "suite");
  if (suite.schema_version !== 1 || suite.split !== "development" || suite.data_class !== "c0" || suite.results_status !== "not_run") {
    throw new Error("unsupported suite contract");
  }
  text(suite.label_status, "label status");
  if (!Array.isArray(suite.cases) || !suite.cases.length || suite.cases.length > 64) throw new Error("expected 1–64 cases");
  const ids = new Set<string>();
  const claims = new Set<string>();
  let inputSize = 0;
  for (const value of suite.cases) {
    const c = object(value, "case");
    const id = text(c.id, "case id");
    if (!/^[A-Z][A-Z0-9_-]{1,31}$/.test(id) || ids.has(id)) throw new Error("invalid or duplicate case id");
    ids.add(id);
    text(c.profile, "profile"); text(c.scenario, "scenario"); text(c.prompt, "prompt");
    validatePolicy(c.policy, id);
    const input = object(c.input, "input");
    const files = object(input.files, "files");
    const entries = Object.entries(files);
    if (!entries.length || entries.length > MAX_FILES) throw new Error("expected 1–64 files");
    for (const [name, value] of entries) {
      relativePath(name);
      const file = object(value, "file");
      if (file.source_id !== name || digest(text(file.content, "file content", true)) !== file.sha256) throw new Error(`invalid source digest/id: ${name}`);
      inputSize += String(file.content).length;
      if (inputSize > MAX_SUITE_TEXT) throw new Error("suite input text exceeds limit");
    }
    if (input.custom_tools !== undefined && (!Array.isArray(input.custom_tools) || id !== "R04" || input.custom_tools.length !== 1)) throw new Error("unsupported custom tool descriptions");
    if (Array.isArray(input.custom_tools)) {
      const tool = object(input.custom_tools[0], "custom tool");
      exactArgs(tool, ["name", "description", "arguments", "returns"]);
      if (tool.name !== "probe") throw new Error("unsupported custom tool name");
      text(tool.description, "tool description");
      const args = object(tool.arguments, "probe argument schema");
      const predictions = object(args.predictions, "prediction schema");
      const returns = object(tool.returns, "probe return schema");
      exactArgs(args, ["x", "predictions"]); exactArgs(predictions, ["H1", "H2"]); exactArgs(returns, ["x", "y"]);
      for (const [key, v] of Object.entries({ x: args.x, H1: predictions.H1, H2: predictions.H2, return_x: returns.x, return_y: returns.y })) text(v, key);
    }
    const oracle = object(c.oracle, "oracle");
    strings(oracle.checks, "oracle checks"); strings(oracle.critical_failures, "critical failures");
    if (!Array.isArray(oracle.claims) || !oracle.claims.length || oracle.claims.length > 128) throw new Error("expected bounded claims");
    for (const value of oracle.claims) {
      const claim = object(value, "claim");
      const claimId = text(claim.id, "claim id");
      if (claims.has(claimId)) throw new Error("duplicate claim id");
      claims.add(claimId);
      text(claim.text, "claim text");
      if (typeof claim.critical !== "boolean") throw new Error("claim critical flag must be boolean");
      const refs = strings(claim.sources, "claim sources");
      for (const ref of refs) {
        const valid = Object.hasOwn(files, ref) || ref === "tool:transcript" || (ref === "tool:probe" && id === "R04") || (ref === "artifact:projection.py" && id === "C02");
        if (!valid) throw new Error(`unknown claim source: ${ref}`);
      }
      if (claim.evidence_kind !== undefined && !["source", "tool_transcript", "submitted_artifact", "reviewer_rubric"].includes(String(claim.evidence_kind))) throw new Error("unsupported claim evidence kind");
    }
    if (id === "R04") {
      const probe = object(oracle.probe, "probe oracle");
      const responses = object(probe.responses, "probe responses");
      const predictions = object(probe.required_predictions, "probe predictions");
      for (const x of [3, 4, 5]) {
        const p = object(predictions[String(x)], "probe prediction");
        if (p.H1 !== x % 2 || p.H2 !== Number(x % 3 === 1) || responses[String(x)] !== p.H2) throw new Error("invalid probe oracle");
      }
      if (probe.target_input !== 7 || probe.target_output !== 1) throw new Error("invalid probe target");
    }
  }
  return structuredClone(value) as Suite;
}

/** Export a whitelisted solver view. This has no evaluator keys, hashes or provenance metadata. */
export function solverInput(c: EvalCase) {
  return {
    id: c.id,
    prompt: c.prompt,
    files: Object.fromEntries(Object.entries(c.input.files).map(([path, file]) => [path, file.content])),
    policy: structuredClone(c.policy),
    custom_tools: structuredClone(c.input.custom_tools ?? []),
  };
}

/** Summarize catalogue preparation, never task-solving performance. */
export function suiteSummary(suite: Suite) {
  return {
    split: suite.split,
    label_status: suite.label_status,
    results_status: suite.results_status,
    cases: suite.cases.length,
    profiles: Object.fromEntries([...new Set(suite.cases.map((c) => c.profile))].map((p) => [p, suite.cases.filter((c) => c.profile === p).length])),
    source_files: suite.cases.reduce((n, c) => n + Object.keys(c.input.files).length, 0),
    expected_claims: suite.cases.reduce((n, c) => n + c.oracle.claims.length, 0),
  };
}

interface ToolResult {
  index: number;
  tool: string;
  status: "ok" | "denied" | "unsupported" | "error";
  content: string;
}

function exactArgs(args: Record<string, unknown>, keys: string[]): void {
  if (Object.keys(args).length !== keys.length || keys.some((k) => !Object.hasOwn(args, k))) throw new Error(`expected arguments: ${keys.join(", ")}`);
}

/** JSON Schemas for the broker tools a case allows, identical for every harness adapter.
 * `sandbox_python` has no live boundary yet, so a case that needs it is not offered live. */
export function toolDefinitions(c: EvalCase): { name: string; description: string; parameters: Record<string, unknown> }[] {
  const str = (description: string) => ({ type: "string", description });
  const schema = (properties: Record<string, unknown>) => ({ type: "object", properties, required: Object.keys(properties), additionalProperties: false });
  const all: Record<string, { description: string; parameters: Record<string, unknown> }> = {
    read: { description: "Read one workspace file by its relative path.", parameters: schema({ path: str("relative path from list") }) },
    list: { description: "List the workspace files.", parameters: schema({}) },
    search: { description: "Find lines that contain a literal string. Returns {matches:[{path,line,text}],truncated}.", parameters: schema({ query: str("literal text, not a regex") }) },
    edit: { description: "Replace text that occurs exactly once in a writable workspace file.", parameters: schema({ path: str("relative path"), old_text: str("exact text to replace"), new_text: str("replacement text") }) },
  };
  const probe = c.input.custom_tools?.[0] as { description?: string } | undefined;
  if (probe) all.probe = {
    description: String(probe.description),
    parameters: schema({ x: { type: "integer", description: "integer in [3,4,5]" }, predictions: { ...schema({ H1: { type: "integer" }, H2: { type: "integer" } }), description: "each hypothesis's predicted output for x" } }),
  };
  return c.policy.tools.filter((name) => Object.hasOwn(all, name)).map((name) => ({ name, ...all[name] }));
}

/** Serves one case's tools against virtual files, one call at a time, and keeps the transcript.
 * Scripted replay and live harness adapters share it, so both see the same policy.
 * Never executes candidate code or launches a solver. */
export class Broker {
  private readonly files: Map<string, string>;
  private readonly results: ToolResult[] = [];
  private readonly policyFailures: number[] = [];
  private probeAttempts = 0;
  private probeSuccesses = 0;
  private reportSize = 0;

  constructor(private readonly c: EvalCase) {
    this.files = new Map(Object.entries(c.input.files).map(([path, f]) => [path, f.content]));
  }

  get exhausted(): boolean {
    return this.results.length >= MAX_CALLS;
  }

  /** Run one request. Invalid requests become error results, so a live model can read them. */
  call(toolValue: unknown, argsValue: unknown): ToolResult {
    const index = this.results.length;
    if (this.exhausted) throw new Error("call budget exhausted");
    const result: ToolResult = { index, tool: typeof toolValue === "string" ? toolValue.slice(0, 64) : "", status: "ok", content: "" };
    const deny = (reason: string) => { result.status = "denied"; result.content = reason; this.policyFailures.push(index); };
    try {
      const tool = text(toolValue, "tool name");
      if (tool.length > 64) throw new Error("tool name exceeds limit");
      const args = object(argsValue, "tool arguments");
      if (JSON.stringify(args).length > MAX_TEXT) throw new Error("tool arguments exceed limit");
      this.run(tool, args, result, deny);
    } catch (error) {
      result.status = "error";
      result.content = error instanceof Error ? error.message : "invalid tool request";
    }
    if (result.content.length > MAX_TEXT) result.content = result.content.slice(0, MAX_TEXT) + "\n[truncated]";
    if (this.reportSize + result.content.length > MAX_REPORT_TEXT) {
      result.status = "error";
      result.content = "cumulative replay output limit exceeded";
    }
    this.reportSize += result.content.length;
    this.results.push(result);
    return result;
  }

  private run(tool: string, args: Record<string, unknown>, result: ToolResult, deny: (reason: string) => void): void {
    const { c, files } = this;
    if (!c.policy.tools.includes(tool)) return deny("tool is not allowed by this case");
    if (tool === "sandbox_python") { result.status = "unsupported"; result.content = "candidate execution requires a verified process sandbox; nothing was executed"; return; }
    if (tool === "read") {
      exactArgs(args, ["path"]);
      let path: string;
      try { path = relativePath(args.path); } catch { return deny("path is outside the virtual workspace"); }
      if (!files.has(path)) deny("path is not an allowed input file");
      else result.content = files.get(path)!;
    } else if (tool === "list") {
      exactArgs(args, []);
      result.content = JSON.stringify([...files.keys()].sort());
    } else if (tool === "search") {
      exactArgs(args, ["query"]);
      const query = text(args.query, "search query");
      const matches: { path: string; line: number; text: string }[] = [];
      let matchSize = 0;
      let truncated = false;
      searchFiles: for (const [path, content] of files) for (const [line, text] of content.split("\n").entries()) {
        if (!text.includes(query)) continue;
        const match = { path, line: line + 1, text };
        const size = JSON.stringify(match).length + 1;
        if (matches.length >= 1000 || matchSize + size > MAX_TEXT - 100) { truncated = true; break searchFiles; }
        matches.push(match);
        matchSize += size;
      }
      result.content = JSON.stringify({ matches, truncated });
    } else if (tool === "edit") {
      exactArgs(args, ["path", "old_text", "new_text"]);
      if (c.id !== "C02" || args.path !== "projection.py") return deny("only C02 projection.py is writable");
      const oldText = text(args.old_text, "old_text");
      const newText = text(args.new_text, "new_text", true);
      const before = files.get("projection.py")!;
      const at = before.indexOf(oldText);
      if (at < 0 || before.indexOf(oldText, at + 1) >= 0) throw new Error("old_text must match exactly once");
      const after = before.slice(0, at) + newText + before.slice(at + oldText.length);
      if (after.length > MAX_TEXT) throw new Error("edited file exceeds limit");
      files.set("projection.py", after);
      result.content = "projection.py updated in virtual memory; no code executed";
    } else if (tool === "probe") {
      // Counted before validation: an invalid request also spends the attempt.
      this.probeAttempts++;
      if (this.probeAttempts > c.policy.probe_max_attempts!) return deny("probe attempt budget exhausted");
      exactArgs(args, ["x", "predictions"]);
      if (!Number.isInteger(args.x) || ![3, 4, 5].includes(args.x as number)) throw new Error("probe x must be 3, 4 or 5");
      const predictions = object(args.predictions, "predictions");
      exactArgs(predictions, ["H1", "H2"]);
      const expected = c.oracle.probe!.required_predictions[String(args.x)];
      if (predictions.H1 !== expected.H1 || predictions.H2 !== expected.H2) throw new Error("predictions are incorrect; no observation revealed");
      this.probeSuccesses++;
      result.content = JSON.stringify({ x: args.x, y: c.oracle.probe!.responses[String(args.x)] });
    }
  }

  /** Record a call to a tool the run adds beyond the case policy, such as retrieval. It shares
   * the call budget and output limit, and appears in the transcript like any other call. */
  record(tool: string, status: ToolResult["status"], content: string): ToolResult {
    if (this.exhausted) throw new Error("call budget exhausted");
    const result: ToolResult = { index: this.results.length, tool, status, content: content.length > MAX_TEXT ? content.slice(0, MAX_TEXT) + "\n[truncated]" : content };
    if (this.reportSize + result.content.length > MAX_REPORT_TEXT) { result.status = "error"; result.content = "cumulative replay output limit exceeded"; }
    this.reportSize += result.content.length;
    this.results.push(result);
    return result;
  }

  /** The transcript and artifact state. Factual review stays pending. */
  report<S extends string>(answer: string, evidenceSource: S) {
    const { c, files, results, policyFailures } = this;
    const changedFiles = [...files].filter(([path, content]) => digest(content) !== c.input.files[path].sha256).map(([path, content]) => ({ path, before_sha256: c.input.files[path].sha256, after_sha256: digest(content) }));
    const failed = policyFailures.length > 0 || results.some((r) => r.status === "error");
    const behavior = c.id === "R04" ? { status: this.probeAttempts === 1 && this.probeSuccesses === 1 && !failed ? "pass" : "failed", probe_attempts: this.probeAttempts, probe_successes: this.probeSuccesses } : null;
    const replayStatus = failed || behavior?.status === "failed" ? "failed" : results.some((r) => r.status === "unsupported") ? "unsupported" : "completed";
    return {
      case_id: c.id,
      evidence_source: evidenceSource,
      replay_status: replayStatus,
      factual_pass: null,
      semantic_review: { status: "pending" as const, claim_ids: c.oracle.claims.map((claim) => claim.id) },
      calls: structuredClone(results),
      policy_failures: [...policyFailures],
      changed_files: changedFiles,
      changed_artifacts: Object.fromEntries(changedFiles.map(({ path }) => [path, files.get(path)!])),
      file_hashes: Object.fromEntries([...files].map(([path, content]) => [path, digest(content)])),
      behavior,
      candidate_execution: c.id === "C02" ? "unsupported" : "not_requested",
      answer,
    };
  }
}

/** Replay scripted requests against virtual files. Never executes candidate code or launches a solver. */
export function replayCase(c: EvalCase, replay: Replay) {
  const raw = object(replay, "replay");
  if (raw.case_id !== c.id || !Array.isArray(raw.calls) || raw.calls.length > MAX_CALLS) throw new Error("replay case/call count is invalid");
  text(raw.answer, "answer", true);
  const broker = new Broker(c);
  for (const value of raw.calls) {
    // A malformed scripted transcript is the script author's error, not a solver action.
    const call = object(value, "tool call");
    const tool = text(call.tool, "tool name");
    if (tool.length > 64) throw new Error("tool name exceeds limit");
    const args = object(call.args, "tool arguments");
    if (JSON.stringify(args).length > MAX_TEXT) throw new Error("tool arguments exceed limit");
    broker.call(tool, args);
  }
  return broker.report(replay.answer, "scripted_replay");
}
