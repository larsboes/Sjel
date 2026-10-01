// tools/sjel-mcp.ts — Sjel's capabilities as MCP tools for an agent (ISA ISC-40, F10).
//
// One stdio MCP server over every capability whose gate admits the agent. The tools are built
// from each capability's own `GET /routes`, so a new route is a new tool with no list to keep
// in step. What the server offers follows the owner's mode for that capability
// (set on the Systems page; read here from the copy in <overlay>/data/agent-modes.json): `off` offers nothing,
// `read-only` offers the reads, `ask` and `auto` offer the writes too. The gate enforces the
// same mode on every call, so this list is a convenience, not the boundary.
//
// Every call carries the agent token, read from the login Keychain through
// tools/capability-auth and held in this process only. Answers come back pseudonymized by
// the gate (ISA F9). A write that waits for the owner answers 202; this server repeats it
// with X-Sjel-Approval until the owner decides in the menu-bar app or on the Systems page.
//
// MCP's stdio transport is newline-delimited JSON-RPC. The three methods an agent needs
// (initialize, tools/list, tools/call) are implemented here directly, so the server adds no
// dependency to review.
//
// Run: `sjel mcp`, or register it once: `claude mcp add sjel -- sjel mcp`.

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { randomUUID } from "node:crypto";

import { axonRoot, overlayRoot } from "./lib/overlay.ts";

export type Mode = "off" | "read-only" | "ask" | "auto";

export interface Route {
  method: string;
  path: string;
  summary: string;
  request_schema?: unknown;
}

export interface Gate {
  capability: string;
  get_writes: string[];
  confirm: string[];
}

export interface Tool {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
  capability: string;
  method: string;
  path: string;
}

const READS = new Set(["GET", "HEAD"]);
const NOT_TOOLS = ["/health", "/ready", "/routes"];

/** `comms__get_triage_id_status`: MCP names allow [a-zA-Z0-9_-], up to 64 characters. */
export function toolName(capability: string, method: string, path: string): string {
  const slug = path
    .replace(/[{}]/g, "")
    .split("/")
    .filter(Boolean)
    .join("_")
    .replace(/[^a-zA-Z0-9_]/g, "_");
  const name = `${capability}__${method.toLowerCase()}_${slug || "root"}`.replace(/-/g, "_");
  if (name.length <= 64) return name;
  // Long names keep their start and gain a short digest of the whole, so two stay distinct.
  let hash = 0;
  for (const ch of name) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  return `${name.slice(0, 55)}_${hash.toString(36).slice(0, 8)}`;
}

function placeholders(path: string): string[] {
  return [...path.matchAll(/\{([^}]+)\}/g)].map((m) => m[1]);
}

/** The tools one capability offers under `mode`. */
export function toolsFor(capability: string, mode: Mode, routes: Route[], gate: Gate): Tool[] {
  if (mode === "off") return [];
  const getWrites = new Set(gate.get_writes);
  const confirm = new Set(gate.confirm);
  const tools: Tool[] = [];
  for (const route of routes) {
    const method = route.method.toUpperCase();
    if (NOT_TOOLS.includes(route.path) || route.path.startsWith("/__axon/")) continue;
    const key = `${method} ${route.path}`;
    const write = !READS.has(method) || getWrites.has(key);
    if (write && mode === "read-only") continue;
    if (method === "HEAD") continue;
    const params = placeholders(route.path);
    const properties: Record<string, unknown> = {};
    for (const p of params) properties[p] = { type: "string", description: `The {${p}} segment.` };
    properties.query = {
      type: "object",
      description: "Query parameters, as strings.",
      additionalProperties: { type: "string" },
    };
    if (!READS.has(method)) {
      properties.body = route.request_schema ?? { type: "object", description: "The JSON body." };
    }
    const asks = confirm.has(key) || (write && mode === "ask");
    const notes = [
      write ? "Changes state." : "Read only.",
      asks ? "Waits for the owner's Allow before it runs." : "",
      "Names and addresses come back as tokens like <SENDER_ab12cd>; pass a token back unchanged to act on that value in this same capability.",
    ].filter(Boolean);
    tools.push({
      name: toolName(capability, method, route.path),
      description: `${capability}: ${route.summary} (${method} ${route.path}) ${notes.join(" ")}`,
      inputSchema: { type: "object", properties, required: params },
      capability,
      method,
      path: route.path,
    });
  }
  return tools;
}

/** The URL for one call, with each `{name}` filled and percent-encoded. */
export function callUrl(base: string, path: string, args: Record<string, unknown>): string {
  const filled = path.replace(/\{([^}]+)\}/g, (_, name: string) => {
    const value = args[name];
    if (typeof value !== "string" || value === "") throw new Error(`missing path parameter: ${name}`);
    return encodeURIComponent(value);
  });
  const query = args.query && typeof args.query === "object" ? (args.query as Record<string, unknown>) : {};
  const search = new URLSearchParams();
  for (const [k, v] of Object.entries(query)) if (v !== undefined && v !== null) search.set(k, String(v));
  const qs = search.toString();
  return `${base}${filled}${qs ? `?${qs}` : ""}`;
}

// --- the running server ------------------------------------------------------------------

interface Registered {
  base: string;
  tools: Tool[];
}

const ROOT = axonRoot();
const SESSION = randomUUID();

function overlay(): string {
  const root = overlayRoot(ROOT);
  if (!root) throw new Error("no overlay is configured");
  return root;
}

function authorization(): string {
  const line = execFileSync(join(ROOT, "tools/capability-auth/capability-auth"), ["--agent"], {
    encoding: "utf8",
  }).trim();
  const value = line.replace(/^Authorization:\s*/i, "");
  if (!value.startsWith("Bearer ")) throw new Error("the agent is not enrolled: run `sjel agent enroll`");
  return value;
}

/** The readable copy of the modes. The real policy is under secrets/, which an agent session
 *  cannot read; this copy only decides which tools are offered, and the gate decides the rest. */
function modes(): Record<string, Mode> {
  const file = join(overlay(), "data/agent-modes.json");
  if (!existsSync(file)) return {};
  return (JSON.parse(readFileSync(file, "utf8")).modes ?? {}) as Record<string, Mode>;
}

function gates(): Gate[] {
  const dir = join(overlay(), "data/agent-gates");
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((f) => f.endsWith(".json"))
    .map((f) => JSON.parse(readFileSync(join(dir, f), "utf8")) as Gate);
}

function bases(): Record<string, string> {
  const out = execFileSync(join(ROOT, "tools/capability.sh"), ["registry"], { encoding: "utf8" });
  const result: Record<string, string> = {};
  for (const entry of JSON.parse(out) as { name: string; port: string; endpoint: string }[]) {
    if (entry.endpoint) result[entry.name] = entry.endpoint;
    else if (entry.port) result[entry.name] = `http://127.0.0.1:${entry.port}`;
  }
  return result;
}

async function capabilityFetch(url: string, init: RequestInit = {}): Promise<Response> {
  const headers = new Headers(init.headers);
  headers.set("Authorization", authorization());
  headers.set("X-Sjel-Agent-Session", SESSION);
  return fetch(url, { ...init, headers, signal: AbortSignal.timeout(60_000) });
}

async function discover(): Promise<Map<string, Registered>> {
  const found = new Map<string, Registered>();
  const policy = modes();
  const where = bases();
  for (const gate of gates()) {
    const base = where[gate.capability];
    if (!base) continue;
    const mode = policy[gate.capability] ?? "auto";
    if (mode === "off") continue;
    try {
      const response = await capabilityFetch(`${base}/routes`);
      if (!response.ok) continue;
      const manifest = (await response.json()) as { routes?: Route[] };
      found.set(gate.capability, { base, tools: toolsFor(gate.capability, mode, manifest.routes ?? [], gate) });
    } catch {
      // A capability that is not running offers no tools now; the next tools/list asks again.
    }
  }
  return found;
}

const APPROVAL_WAIT_MS = 120_000;

async function call(tool: Tool, base: string, args: Record<string, unknown>): Promise<string> {
  const url = callUrl(base, tool.path, args);
  const init: RequestInit = { method: tool.method };
  if (!READS.has(tool.method)) {
    init.body = JSON.stringify(args.body ?? {});
    init.headers = { "Content-Type": "application/json" };
  }
  let response = await capabilityFetch(url, init);
  const started = Date.now();
  while (response.status === 202) {
    const ask = (await response.json()) as { approval?: string; state?: string };
    if (!ask.approval) break;
    if (Date.now() - started > APPROVAL_WAIT_MS) {
      return `Waiting for the owner to allow this write (approval ${ask.approval}). Ask them to decide in the Sjel menu-bar app, then call this tool again.`;
    }
    await new Promise((r) => setTimeout(r, 3000));
    response = await capabilityFetch(url, {
      ...init,
      headers: { ...(init.headers as Record<string, string>), "X-Sjel-Approval": ask.approval },
    });
  }
  const text = await response.text();
  return response.ok ? text : `HTTP ${response.status}: ${text}`;
}

type Rpc = { jsonrpc: "2.0"; id?: number | string; method: string; params?: Record<string, unknown> };

function send(message: unknown): void {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

async function main(): Promise<void> {
  let registered = new Map<string, Registered>();
  let buffer = "";
  const handle = async (rpc: Rpc) => {
    const reply = (result: unknown) => rpc.id !== undefined && send({ jsonrpc: "2.0", id: rpc.id, result });
    const fail = (code: number, message: string) =>
      rpc.id !== undefined && send({ jsonrpc: "2.0", id: rpc.id, error: { code, message } });
    try {
      switch (rpc.method) {
        case "initialize":
          return reply({
            protocolVersion: (rpc.params?.protocolVersion as string) ?? "2025-06-18",
            capabilities: { tools: { listChanged: false } },
            serverInfo: { name: "sjel", version: "0.1.0" },
            instructions:
              "Sjel's capabilities on this machine. Answers are pseudonymized: pass tokens back unchanged. Writes follow the owner's per-capability mode and may wait for their Allow.",
          });
        case "tools/list":
          registered = await discover();
          return reply({
            tools: [...registered.values()].flatMap((r) =>
              r.tools.map(({ name, description, inputSchema }) => ({ name, description, inputSchema })),
            ),
          });
        case "tools/call": {
          const name = rpc.params?.name as string;
          if (registered.size === 0) registered = await discover();
          for (const r of registered.values()) {
            const tool = r.tools.find((t) => t.name === name);
            if (tool) {
              const text = await call(tool, r.base, (rpc.params?.arguments as Record<string, unknown>) ?? {});
              return reply({ content: [{ type: "text", text }] });
            }
          }
          return reply({ content: [{ type: "text", text: `No tool named ${name}.` }], isError: true });
        }
        case "ping":
          return reply({});
        default:
          if (rpc.id !== undefined) return fail(-32601, `method not found: ${rpc.method}`);
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (rpc.method === "tools/call") return reply({ content: [{ type: "text", text: message }], isError: true });
      return fail(-32603, message);
    }
  };
  process.stdin.setEncoding("utf8");
  process.stdin.on("data", (chunk: string) => {
    buffer += chunk;
    let newline: number;
    while ((newline = buffer.indexOf("\n")) >= 0) {
      const line = buffer.slice(0, newline).trim();
      buffer = buffer.slice(newline + 1);
      if (!line) continue;
      try {
        void handle(JSON.parse(line) as Rpc);
      } catch {
        send({ jsonrpc: "2.0", id: null, error: { code: -32700, message: "parse error" } });
      }
    }
  });
}

if (import.meta.main) await main();
