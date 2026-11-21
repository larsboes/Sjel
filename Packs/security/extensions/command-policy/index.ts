/** Project-scoped autonomy and approval gates for consequential effects. */
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { appendFileSync, mkdirSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { classify, classifyWrite, resolvePath, type Scope, type Verdict } from "./classify.ts";
import { loadProtectedRoots } from "./config.ts";
import { checkoutRoot, checkpointWarning, createScratch, dirtyFiles, FileObservations, ownedProcessIds, releaseScratch } from "./runtime.ts";
import { powerDeferral } from "./power.ts";

function logDecision(entry: Record<string, unknown>): void {
  const overlay = process.env.SJEL_OVERLAY_ROOT ?? process.env.SJEL_PERSONAL_ROOT;
  if (!overlay) return;
  try {
    const dir = join(overlay, "logs");
    mkdirSync(dir, { recursive: true });
    appendFileSync(join(dir, "command-policy.jsonl"), `${JSON.stringify({ ts: new Date().toISOString(), ...entry })}\n`);
  } catch {
    // A log that cannot be written never changes a decision.
  }
}

function scopeFor(cwd: string): Scope {
  return { cwd, home: homedir(), writable: [] };
}

function judge(toolName: string, input: Record<string, unknown>, scope: Scope): Verdict | null {
  if (toolName === "bash" && typeof input.command === "string") return classify(input.command, scope);
  if (toolName === "vault_exec" && typeof input.cmd === "string") return classify(input.cmd, { ...scope, externalEnvironment: true });
  if ((toolName === "write" || toolName === "edit") && typeof input.path === "string") {
    return classifyWrite(input.path, scope, toolName);
  }
  return null;
}

function preview(toolName: string, input: Record<string, unknown>): string {
  const text = toolName === "bash" ? String(input.command) : toolName === "vault_exec" ? String(input.cmd) : String(input.path);
  return text.length > 600 ? `${text.slice(0, 600)}…` : text;
}

export default function commandPolicy(pi: ExtensionAPI) {
  const granted = new Set<string>();
  let project: string | undefined;
  let scratch: string | undefined;
  let protectedRoots: string[] = [];
  let observations = new FileObservations();
  let checkpoint: ReturnType<typeof dirtyFiles>;
  let paused = true;
  let generation = 0;
  const activeScope = (cwd: string): Scope => ({
    ...scopeFor(cwd),
    autonomy: {
      writeRoots: paused ? [] : [project, scratch].filter((path): path is string => path !== undefined),
      disposableRoots: !paused && scratch ? [scratch] : [],
      protectedRoots,
    },
  });
  const update = (ctx: ExtensionContext) => ctx.ui.setStatus?.("permissions", paused ? "permissions: paused" : undefined);

  pi.on("session_start", (_event, ctx) => {
    generation++;
    granted.clear();
    paused = true;
    project = undefined;
    protectedRoots = [];
    observations = new FileObservations();
    if (scratch) releaseScratch(scratch);
    scratch = undefined;
    try {
      const scope = scopeFor(ctx.cwd);
      protectedRoots = loadProtectedRoots(scope);
      project = checkoutRoot(scope);
      scratch = resolvePath(createScratch(), scope);
      paused = false;
    } catch {
      const warning = "Command-policy setup failed; automatic project and scratch grants are paused. Check the overlay restrictions and temporary-directory access.";
      if (ctx.hasUI) ctx.ui.notify(warning, "warning");
      else console.error(warning);
    }
    update(ctx);
  });

  pi.registerCommand("permissions", {
    description: "Show the active checkout and session scratch. Off pauses scoped grants until reload or session change.",
    handler: async (args, ctx) => {
      if (!ctx.hasUI || ctx.signal?.aborted) return;
      if (args.trim() === "off") {
        generation++;
        paused = true;
        granted.clear();
        update(ctx);
      }
      const status = paused ? "Scoped grants paused until reload or session change."
        : `Project: ${project ?? "none"}. Session scratch: ${scratch ?? "none"}.`;
      ctx.ui.notify(`${status} Use /permissions or /permissions off.`, "info");
    },
  });

  pi.on("session_shutdown", () => {
    if (scratch) releaseScratch(scratch);
  });

  pi.on("before_agent_start", (_event, ctx) => {
    checkpoint = dirtyFiles(activeScope(ctx.cwd));
  });

  pi.on("agent_settled", (_event, ctx) => {
    const warning = checkpointWarning(checkpoint, activeScope(ctx.cwd));
    if (warning) pi.sendMessage({ customType: "command-policy-checkpoint", content: warning, display: true }, { triggerTurn: false });
  });

  pi.on("tool_result", (event, ctx) => {
    if (event.toolName === "read") observations.finishRead(event.toolCallId, !!event.isError);
    if (!event.isError && (event.toolName === "write" || event.toolName === "edit") && typeof event.input.path === "string") {
      observations.finishWrite(resolvePath(event.input.path, activeScope(ctx.cwd)));
    }
  });

  pi.on("tool_call", async (event, ctx) => {
    const epoch = generation;
    const input = (event.input ?? {}) as Record<string, unknown>;
    const scope = activeScope(ctx.cwd ?? process.cwd());
    if (event.toolName === "read" && typeof input.path === "string") {
      observations.read(event.toolCallId, resolvePath(input.path, scope));
    }
    if (!paused && (event.toolName === "bash" || event.toolName === "vault_exec") && /\bkill\b/.test(String(input.command ?? input.cmd))) {
      scope.ownedPids = ownedProcessIds();
    }
    const v = judge(event.toolName, input, scope);
    if (!v) return undefined;
    const base = { tool: event.toolName, class: v.class, rule: v.rule, programs: v.programs };
    const cwd = resolvePath(scope.cwd, scope);
    const key = `${cwd}\u0000${event.toolName}\u0000${JSON.stringify(input)}`;

    if (v.class === "allow") {
      if ((event.toolName === "write" || event.toolName === "edit") && typeof input.path === "string") {
        const reason = observations.beforeWrite(resolvePath(input.path, scope));
        if (reason) {
          logDecision({ ...base, rule: "C1", decision: "stale-or-unread" });
          return { block: true, reason };
        }
      }
      if (event.toolName === "bash" || event.toolName === "vault_exec") {
        const reason = powerDeferral(String(input.command ?? input.cmd ?? ""), ctx.cwd ?? process.cwd());
        if (reason) {
          logDecision({ ...base, rule: "P1", decision: "battery-deferred" });
          return { block: true, reason };
        }
      }
      logDecision({ ...base, decision: "allow" });
      return undefined;
    }
    if (v.class === "ask" && granted.has(key)) {
      logDecision({ ...base, decision: "exact-grant" });
      return undefined;
    }
    if (!ctx.hasUI) {
      logDecision({ ...base, decision: "refused-no-ui" });
      return { block: true, reason: `Command policy ${v.rule}: ${v.why}. No user is present to approve it. Continue independent work only; do not bypass this gate.` };
    }

    const shown = preview(event.toolName, input);
    const choice = await ctx.ui.select(
      v.class === "ask" ? `Approve ${event.toolName}? (${v.rule}: ${v.why})\n\n  ${shown}`
        : `BLOCKED by rule ${v.rule}: ${v.why}\n\n  ${shown}\n\nThis rule never grants for a session.`,
      v.class === "ask" ? ["Allow once", "Allow this exact call for this session", "Deny"] : ["Deny", "Allow once"],
      { signal: ctx.signal },
    );
    if (epoch !== generation || ctx.signal?.aborted) {
      logDecision({ ...base, decision: "aborted" });
      return { block: true, reason: "Permission request aborted or session changed. Nothing executed and no approval was granted." };
    }
    if (choice === "Allow this exact call for this session" && v.class === "ask") granted.add(key);
    const allowed = choice === "Allow once" || v.class === "ask" && choice === "Allow this exact call for this session";
    if (allowed) {
      logDecision({ ...base, decision: choice === "Allow once" ? "allow-once" : "session-grant-added" });
      return undefined;
    }
    if (choice === undefined) {
      logDecision({ ...base, decision: "cancelled" });
      return { block: true, reason: `Command policy ${v.rule}: approval cancelled. Nothing executed and no approval was granted. No pending action is stored. Continue only independent approved work; do not retry this action without user authorization.` };
    }
    logDecision({ ...base, decision: "denied" });
    return { block: true, reason: `Command policy ${v.rule}: ${v.why}. The user denied it. Do not retry it in another form; ask the user instead.` };
  });
}
