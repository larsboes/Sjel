import { randomUUID } from "node:crypto";
import { spawn } from "node:child_process";

import type { EvalCase } from "./agent-eval.ts";

const IMAGE = "python:3.13-slim";
const MAX_SOURCE_BYTES = 128_000;
const MAX_INPUT_BYTES = 512_000;
const MAX_OUTPUT_BYTES = 128_000;
const DEADLINE_MS = 10_000;

const PROBE_CODE = `import json, os, socket
checks = {"nonroot": os.getuid() != 0,
          "host_mount_absent": not os.path.exists("/mnt/mac"),
          "docker_socket_absent": not os.path.exists("/var/run/docker.sock")}
try:
    with open("/sandbox-write-probe", "w") as out:
        out.write("probe")
    checks["root_write_denied"] = False
except OSError:
    checks["root_write_denied"] = True
try:
    pid = os.fork()
    if pid == 0:
        os._exit(0)
    os.waitpid(pid, 0)
    checks["fork_denied"] = False
except OSError:
    checks["fork_denied"] = True
try:
    with socket.create_connection(("1.1.1.1", 443), timeout=1):
        checks["external_connect_denied"] = False
except OSError:
    checks["external_connect_denied"] = True
print(json.dumps(checks))
`;

const PROJECTION_CODE = `import json, sys
payload = json.load(sys.stdin)
encode = json.dumps
namespace = {"__name__": "projection"}
try:
    exec(compile(payload["source"], "projection.py", "exec"), namespace)
    factory = namespace["Projection"]
    snapshots = []
    for changes in payload["sequences"]:
        projection = factory()
        rows = []
        for change in changes:
            projection.apply(change)
            rows.append(dict(projection.rows))
        snapshots.append(rows)
    result = {"snapshots": snapshots}
except BaseException as error:
    result = {"error": type(error).__name__}
print(encode(result))
`;

interface ProcessResult {
  code: number | null;
  stdout: string;
  stderr: string;
  error?: string;
}

export interface SandboxProbe {
  supported: boolean;
  reason?: string;
  image_id?: string;
  checks?: Record<string, boolean>;
}

export interface ProjectionEvaluation {
  status: "passed" | "failed" | "unsupported";
  reason?: string;
  image_id?: string;
  transitions?: number;
  failures?: { sequence: number; step: number }[];
}

function cleanup(name: string): Promise<void> {
  return new Promise((resolve) => {
    const child = spawn("docker", ["--context", "orbstack", "rm", "--force", name], { stdio: "ignore" });
    const timer = setTimeout(() => child.kill("SIGKILL"), 2_000);
    child.once("error", () => { clearTimeout(timer); resolve(); });
    child.once("close", () => { clearTimeout(timer); resolve(); });
  });
}

function docker(args: string[], input = "", containerName?: string): Promise<ProcessResult> {
  return new Promise((resolve) => {
    const child = spawn("docker", ["--context", "orbstack", ...args], { stdio: ["pipe", "pipe", "pipe"] });
    const chunks: { stdout: Buffer[]; stderr: Buffer[] } = { stdout: [], stderr: [] };
    let bytes = 0;
    let failure: string | undefined;
    let settled = false;
    const finish = async (code: number | null, error?: string) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if ((failure || error || code !== 0) && containerName) await cleanup(containerName);
      resolve({ code, stdout: Buffer.concat(chunks.stdout).toString("utf8"), stderr: Buffer.concat(chunks.stderr).toString("utf8"), error: failure ?? error });
    };
    const stop = (reason: string) => {
      if (failure) return;
      failure = reason;
      child.kill("SIGKILL");
    };
    const timer = setTimeout(() => stop("sandbox deadline exceeded"), DEADLINE_MS);
    for (const stream of ["stdout", "stderr"] as const) {
      child[stream].on("data", (chunk: Buffer) => {
        bytes += chunk.length;
        if (bytes > MAX_OUTPUT_BYTES) stop("sandbox output limit exceeded");
        else chunks[stream].push(chunk);
      });
    }
    child.once("error", (error) => { void finish(null, error.message); });
    child.once("close", (code, signal) => { void finish(code, signal ? `docker client stopped by ${signal}` : undefined); });
    child.stdin.on("error", () => { /* A failed guest may close stdin before consuming its input. */ });
    child.stdin.end(input);
  });
}

/** Fixed container policy; no caller-supplied mounts, networking, environment or entrypoint. */
export function sandboxCommand(imageId: string, name: string, mode: "probe" | "projection"): string[] {
  if (!/^sha256:[a-f0-9]{64}$/.test(imageId)) throw new Error("sandbox requires an immutable local image ID");
  if (!/^sjel-agent-eval-[a-f0-9-]{36}$/.test(name)) throw new Error("invalid evaluator container name");
  if (mode !== "probe" && mode !== "projection") throw new Error("guest entrypoint must be evaluator-owned");
  const code = mode === "probe" ? PROBE_CODE : PROJECTION_CODE;
  return ["run", "--name", name, "--rm", "--pull=never", "--network", "none", "--read-only", "--user", "65534:65534",
    "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--memory", "128m", "--cpus", "1", "--pids-limit", "1",
    "-i", "--entrypoint", "python", imageId, "-I", "-S", "-B", "-c", code];
}

async function guest(imageId: string, mode: "probe" | "projection", input = ""): Promise<ProcessResult> {
  const name = `sjel-agent-eval-${randomUUID()}`;
  return docker(sandboxCommand(imageId, name, mode), input, name);
}

/** Probe the existing OrbStack engine and local Python image without pulling or using host mounts. */
export async function sandboxProbe(): Promise<SandboxProbe> {
  const inspected = await docker(["image", "inspect", "--format", "{{.Id}}", IMAGE]);
  const imageId = inspected.stdout.trim();
  if (inspected.error || inspected.code !== 0 || !/^sha256:[a-f0-9]{64}$/.test(imageId)) {
    return { supported: false, reason: inspected.error ?? "OrbStack engine or local python:3.13-slim image unavailable" };
  }
  const result = await guest(imageId, "probe");
  if (result.error || result.code !== 0) return { supported: false, image_id: imageId, reason: result.error ?? "sandbox probe failed" };
  try {
    const checks = JSON.parse(result.stdout) as Record<string, boolean>;
    const names = ["nonroot", "host_mount_absent", "docker_socket_absent", "root_write_denied", "fork_denied", "external_connect_denied"];
    if (!checks || Array.isArray(checks) || names.some((name) => checks[name] !== true) || Object.keys(checks).length !== names.length) {
      return { supported: false, image_id: imageId, reason: "sandbox probe did not establish every required boundary" };
    }
    return { supported: true, image_id: imageId, checks };
  } catch {
    return { supported: false, image_id: imageId, reason: "sandbox probe returned invalid evidence" };
  }
}

interface ProjectionSequence {
  steps: { change: Record<string, unknown>; expected_rows: Record<string, unknown> }[];
}

function sequences(c: EvalCase): ProjectionSequence[] {
  const value = c.oracle.event_sequences;
  if (!Array.isArray(value) || value.length === 0 || value.length > 64) throw new Error("C02 requires bounded evaluator sequences");
  let transitions = 0;
  for (const sequence of value) {
    if (!sequence || !Array.isArray(sequence.steps) || sequence.steps.length === 0) throw new Error("invalid evaluator sequence");
    for (const step of sequence.steps) {
      transitions++;
      if (!step || !step.change || typeof step.change !== "object" || Array.isArray(step.change)
        || !step.expected_rows || typeof step.expected_rows !== "object" || Array.isArray(step.expected_rows)) {
        throw new Error("invalid evaluator transition");
      }
      const change = step.change;
      const allowed = change.op === "upsert" ? ["id", "revision", "op", "value"] : ["id", "revision", "op"];
      if (typeof change.id !== "string" || !change.id.length || change.id.length > 256
        || !Number.isSafeInteger(change.revision) || change.revision < 1
        || !["upsert", "delete"].includes(change.op)
        || Object.keys(change).some((key) => !allowed.includes(key))
        || (change.op === "upsert" && typeof change.value !== "string")
        || Object.values(step.expected_rows).some((value) => typeof value !== "string")) {
        throw new Error("unsupported evaluator change shape");
      }
    }
  }
  if (transitions > 256) throw new Error("too many evaluator transitions");
  return value as ProjectionSequence[];
}

/** Build guest input by whitelisting source and changes; answer keys stay in the parent. */
export function projectionInput(c: EvalCase, source: string): string {
  if (c.id !== "C02") throw new Error("projection execution is available for C02 only");
  if (typeof source !== "string" || Buffer.byteLength(source) > MAX_SOURCE_BYTES) throw new Error("projection source exceeds limit");
  const input = JSON.stringify({ source, sequences: sequences(c).map((sequence) => sequence.steps.map(({ change }) => ({
    id: change.id, revision: change.revision, op: change.op, ...(change.op === "upsert" ? { value: change.value } : {}),
  }))) });
  if (Buffer.byteLength(input) > MAX_INPUT_BYTES) throw new Error("projection guest input exceeds limit");
  return input;
}

/** Compare untrusted guest snapshots in the parent without interpreting guest code or claims. */
export function compareProjection(c: EvalCase, output: string): ProjectionEvaluation {
  if (c.id !== "C02") throw new Error("projection comparison is available for C02 only");
  if (Buffer.byteLength(output) > MAX_OUTPUT_BYTES) return { status: "failed", reason: "guest output exceeds limit" };
  const expected = sequences(c);
  let body: unknown;
  try { body = JSON.parse(output); } catch { return { status: "failed", reason: "guest returned invalid JSON" }; }
  if (!body || typeof body !== "object" || Array.isArray(body)) return { status: "failed", reason: "guest returned invalid result" };
  if (Object.keys(body).length !== 1 || !Object.hasOwn(body, "snapshots")) return { status: "failed", reason: "guest returned invalid result fields" };
  const snapshots = (body as Record<string, unknown>).snapshots;
  if (!Array.isArray(snapshots) || snapshots.length !== expected.length) return { status: "failed", reason: "guest did not return one sequence result per input" };
  const failures: { sequence: number; step: number }[] = [];
  let transitions = 0;
  for (let i = 0; i < expected.length; i++) {
    const actual = snapshots[i];
    if (!Array.isArray(actual) || actual.length !== expected[i]!.steps.length) return { status: "failed", reason: "guest did not return one snapshot per event" };
    for (let j = 0; j < actual.length; j++) {
      transitions++;
      const rows = actual[j];
      const wanted = expected[i]!.steps[j]!.expected_rows;
      if (!rows || typeof rows !== "object" || Array.isArray(rows)
        || Object.keys(rows).length !== Object.keys(wanted).length
        || Object.keys(wanted).some((key) => !Object.hasOwn(rows, key) || JSON.stringify(rows[key]) !== JSON.stringify(wanted[key]))) {
        failures.push({ sequence: i, step: j });
      }
    }
  }
  return { status: failures.length ? "failed" : "passed", transitions, failures };
}

/** Execute only in a freshly probed confined container; no host-Python fallback exists. */
export async function evaluateProjection(c: EvalCase, source: string): Promise<ProjectionEvaluation> {
  const input = projectionInput(c, source);
  const probe = await sandboxProbe();
  if (!probe.supported || !probe.image_id) return { status: "unsupported", reason: probe.reason };
  const result = await guest(probe.image_id, "projection", input);
  if (result.error || result.code !== 0) return { status: "failed", image_id: probe.image_id, reason: result.error ?? "guest process failed" };
  return { ...compareProjection(c, result.stdout), image_id: probe.image_id };
}
