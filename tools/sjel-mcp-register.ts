// tools/sjel-mcp-register.ts — register Sjel's MCP server with the agent harnesses on this
// machine, and verify each registration against the server itself (ISA ISC-40).
//
// Why this exists: the server shipped 2026-10-01 and was registered in no harness at all. The
// ISA named `claude mcp add sjel -- sjel mcp`, that command was never run, and nothing in this
// repository could have noticed. Wiring that lives only in a document rots without a claim
// failing. This tool is the claim: it writes the registration, then speaks MCP to the server it
// wrote and reports the tool count it got back.
//
// It drives each harness's own CLI where a CLI can express the registration, the way
// tools/agent-integrations.sh drives upstreams' installers instead of keeping a copy of what
// they emit. pi is the exception, and the measurement is the reason: `pi mcp add` has no option
// for a server's per-request timeout ("Unknown option --timeout", pi 1.0.0), and that timeout is
// load-bearing here — an ask-mode write waits up to 120 s for the owner's Allow while pi's
// default is 60 s. So pi's entry is written directly, and tools/sjel-mcp-register.test.ts holds
// the shape.
//
// Usage:
//   sjel mcp register [<harness>...]     every installed harness that has a path, or the named ones
//   sjel mcp unregister [<harness>...]
//
// Exit 0 = every target registered and verified, 1 = a target failed or was not touched.

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";

import { HARNESSES, harnessById, isInstalled, type Harness } from "./lib/harness-registry.ts";
import { writeFileAtomic } from "./claude-code-config.ts";

export const SERVER_NAME = "sjel";

const HOME = process.env.HOME ?? "";

/** What the server calls itself, in the harness's own words. */
const DESCRIPTION =
  "Sjel's local capabilities on this Mac (comms, calendar, devices). Answers are pseudonymized: " +
  "pass tokens like <SENDER_ab12cd> back unchanged. Writes follow the owner's per-capability mode " +
  "(off, read-only, ask, auto) and an ask-mode write waits for Allow in the menu-bar app.";

/**
 * The command and arguments that start the server. This is also what Claude Code's managed
 * policy allowlists by `serverCommand` — and commands match exactly, every argument in order —
 * so the two must stay one value, not two copies of one string.
 */
export function serverCommand(): string[] {
  return [join(HOME, ".local", "bin", "sjel"), "mcp"];
}

/** pi's entry. `timeout` is seconds and is the reason this is written rather than delegated. */
export function piEntry(): Record<string, unknown> {
  return {
    command: serverCommand()[0],
    args: serverCommand().slice(1),
    exposure: "codemode",
    timeout: 180,
    description: DESCRIPTION,
  };
}

/** pi's config with our entry set, and every other server and key left alone. */
export function piConfigWith(existing: unknown, entry: Record<string, unknown>): Record<string, unknown> {
  const isPlain = (v: unknown) => typeof v === "object" && v !== null && !Array.isArray(v);
  const base = isPlain(existing) ? { ...(existing as Record<string, unknown>) } : {};
  const servers = isPlain(base.mcpServers) ? { ...(base.mcpServers as Record<string, unknown>) } : {};
  servers[SERVER_NAME] = entry;
  base.mcpServers = servers;
  return base;
}

/** pi's config with our entry removed; the file itself is left in place even when it empties. */
export function piConfigWithout(existing: unknown): Record<string, unknown> {
  const isPlain = (v: unknown) => typeof v === "object" && v !== null && !Array.isArray(v);
  const base = isPlain(existing) ? { ...(existing as Record<string, unknown>) } : {};
  if (isPlain(base.mcpServers)) {
    const servers = { ...(base.mcpServers as Record<string, unknown>) };
    delete servers[SERVER_NAME];
    if (Object.keys(servers).length > 0) base.mcpServers = servers;
    else delete base.mcpServers;
  }
  return base;
}

export function claudeAddArgs(): string[] {
  return ["mcp", "add", SERVER_NAME, "--scope", "user", "--", ...serverCommand()];
}

export function claudeRemoveArgs(): string[] {
  return ["mcp", "remove", SERVER_NAME, "--scope", "user"];
}

/**
 * How one harness is registered, or null when this repository has no measured path for it.
 *
 * Codex is null on purpose. It supports MCP, its config format is documented elsewhere, and
 * nothing here has run it: ~/.codex exists on this machine while no `codex` binary does, so a
 * writer for it could not be verified even once. tools/lib/harness-registry.ts says the rule —
 * a row moves only when someone verifies the format, never on the strength of a guess.
 */
function strategyFor(harness: Harness): "config" | "cli" | null {
  if (harness.id === "pi") return "config";
  if (harness.id === "claude") return "cli";
  return null;
}

function piConfigPath(): string {
  return join(HOME, ".pi", "agent", "mcp.json");
}

function readJsonOrNull(path: string): unknown {
  if (!existsSync(path)) return null;
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    throw new Error(`${path} is not valid JSON; fix or move it before registering`);
  }
}

/** Speak MCP to the server this tool just registered, and report what it answers. */
function handshake(): { ok: boolean; tools: number | null; error?: string } {
  const command = serverCommand();
  if (!existsSync(command[0])) return { ok: false, tools: null, error: `not found: ${command[0]}` };
  const lines = [
    JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: { protocolVersion: "2025-06-18", clientInfo: { name: "sjel-mcp-register", version: "0" }, capabilities: {} },
    }),
    JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/list" }),
  ].join("\n");
  const run = spawnSync(command[0], command.slice(1), { input: `${lines}\n`, encoding: "utf8", timeout: 120_000 });
  if (run.error) return { ok: false, tools: null, error: run.error.message };
  const replies = (run.stdout ?? "")
    .split("\n")
    .filter(Boolean)
    .map((line) => {
      try {
        return JSON.parse(line) as { id?: number; result?: { tools?: unknown[] }; error?: { message?: string } };
      } catch {
        return null;
      }
    })
    .filter(Boolean) as { id?: number; result?: { tools?: unknown[] }; error?: { message?: string } }[];
  const initialized = replies.find((r) => r.id === 1);
  const listed = replies.find((r) => r.id === 2);
  if (!initialized || initialized.error) {
    return { ok: false, tools: null, error: initialized?.error?.message ?? "no initialize reply" };
  }
  if (!listed || listed.error) return { ok: false, tools: null, error: listed?.error?.message ?? "no tools/list reply" };
  return { ok: true, tools: (listed.result?.tools ?? []).length };
}

function registerConfig(harness: Harness): string[] {
  const path = piConfigPath();
  const before = readJsonOrNull(path);
  const after = piConfigWith(before, piEntry());
  mkdirSync(dirname(path), { recursive: true });
  writeFileAtomic(path, `${JSON.stringify(after, null, 2)}\n`, 0o644);
  return [`wrote ${path}`];
}

function unregisterConfig(): string[] {
  const path = piConfigPath();
  if (!existsSync(path)) return [`nothing at ${path}`];
  writeFileAtomic(path, `${JSON.stringify(piConfigWithout(readJsonOrNull(path)), null, 2)}\n`, 0o644);
  return [`removed ${SERVER_NAME} from ${path}`];
}

function registerCli(harness: Harness, removing: boolean): { ok: boolean; notes: string[] } {
  const args = removing ? claudeRemoveArgs() : claudeAddArgs();
  const run = spawnSync("claude", args, { encoding: "utf8", timeout: 120_000 });
  if (run.error) {
    return { ok: false, notes: [`could not run claude: ${run.error.message}`] };
  }
  const output = `${run.stdout ?? ""}${run.stderr ?? ""}`.trim();
  if (run.status !== 0) {
    const notes = [`claude ${args.slice(0, 3).join(" ")} failed: ${output}`];
    if (/enterprise policy/i.test(output)) {
      // The measured reason, not a mystery: the managed allowlist has to name this command
      // before a user-scope registration is permitted.
      notes.push(
        "the deployed managed policy forbids it; deploy the current policy first: tools/claude-code-config --managed",
      );
    }
    return { ok: false, notes };
  }
  return { ok: true, notes: [output || `claude ${args.slice(0, 3).join(" ")} ok`] };
}

type Result = { harness: string; ok: boolean; touched: boolean; notes: string[] };

function act(harness: Harness, removing: boolean): Result {
  const strategy = strategyFor(harness);
  if (strategy === null) {
    const where = isInstalled(harness) ? "installed" : "not installed";
    return {
      harness: harness.id,
      ok: false,
      touched: false,
      notes: [`${where}; no MCP registration path is measured for it in this repository — not touched`],
    };
  }
  if (!isInstalled(harness)) {
    return { harness: harness.id, ok: false, touched: false, notes: [`not installed (no ${harness.marker})`] };
  }
  if (strategy === "config") {
    const notes = removing ? unregisterConfig() : registerConfig(harness);
    return { harness: harness.id, ok: true, touched: !removing, notes };
  }
  const { ok, notes } = registerCli(harness, removing);
  return { harness: harness.id, ok, touched: ok && !removing, notes };
}

function main(): never {
  const [verb, ...names] = process.argv.slice(2);
  const removing = verb === "unregister";
  if (verb !== "register" && verb !== "unregister") {
    console.error("usage: sjel mcp register [<harness>...] | sjel mcp unregister [<harness>...]");
    process.exit(1);
  }

  let targets: Harness[];
  if (names.length > 0) {
    try {
      targets = names.map(harnessById);
    } catch (error) {
      console.error(`sjel-mcp-register: ${error instanceof Error ? error.message : String(error)}`);
      process.exit(1);
    }
  } else {
    targets = HARNESSES;
  }

  let failed = false;
  const results: Result[] = targets.map((harness) => act(harness, removing));

  // One handshake for the whole run: the server is one server, whatever registered it.
  const check = removing ? null : handshake();

  for (const result of results) {
    console.log(`${result.ok ? "✓" : "✗"} ${result.harness}`);
    for (const note of result.notes) console.log(`    ${note}`);
    if (!result.ok) failed = true;
  }

  if (removing) {
    // Silence is ambiguous here: a harness with no measured path was never registered to begin
    // with, so say what this run actually did rather than let "✗ codex" read as a failed removal.
    console.log("\nNothing to verify after a removal.");
  } else if (check === null) {
    failed = true;
  } else if (check.ok) {
    console.log(`\n✓ server verified: ${check.tools} tools over stdio`);
    console.log("  Restart a harness session, or /reload it, to pick the server up.");
  } else {
    console.log(`\n✗ server did not answer MCP: ${check.error}`);
    failed = true;
  }

  process.exit(failed ? 1 : 0);
}

if (import.meta.main) main();
