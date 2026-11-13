/** Command approval cards, session file scopes, and deferred permission decisions. */
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { appendFileSync, mkdirSync, statSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { classify, classifyWrite, resolvePath, type Scope, type Verdict } from "./classify.ts";
import { powerDeferral } from "./power.ts";
import { scopedCommand, scopedWrite } from "./scoped.ts";

export const APPROVAL_TIMEOUT_MS = 60_000;

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
  return { cwd, home: homedir(), writable: [...new Set([tmpdir(), "/tmp"])] };
}

function judge(toolName: string, input: Record<string, unknown>, scope: Scope): Verdict | null {
  if (toolName === "bash" && typeof input.command === "string") return classify(input.command, scope);
  if (toolName === "vault_exec" && typeof input.cmd === "string") return classify(input.cmd, scope);
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
  const once = new Set<string>();
  const roots = new Set<string>();
  let generation = 0;
  const pending = new Map<string, { tool: string; cwd: string; input: Record<string, unknown>; verdict: Verdict }>();
  const update = (ctx: ExtensionContext) => ctx.ui.setStatus?.("permissions", roots.size || pending.size
    ? `permissions: ${roots.size} file scopes · ${pending.size} pending (/permissions pending)` : undefined);

  pi.on("session_start", (_event, ctx) => {
    generation++;
    granted.clear();
    once.clear();
    roots.clear();
    pending.clear();
    update(ctx);
  });

  pi.registerCommand("permissions", {
    description: "Session file scopes: auto <directory>, off, status, pending. Reload clears scopes and pending cards.",
    handler: async (args, ctx) => {
      const epoch = generation;
      if (!ctx.hasUI || ctx.signal?.aborted) return;
      const text = args.trim();
      if (text.startsWith("auto ")) {
        const scope = scopeFor(ctx.cwd);
        const path = resolvePath(text.slice(5).trim(), scope);
        if (path === "/" || path === resolvePath(scope.home, scope)) {
          ctx.ui.notify("Choose a task directory, not the whole account or filesystem.", "warning");
          return;
        }
        try {
          if (!statSync(path).isDirectory()) throw new Error("not a directory");
        } catch {
          ctx.ui.notify("The file scope must name an existing directory.", "error");
          return;
        }
        roots.add(path);
        update(ctx);
        ctx.ui.notify(`File scope enabled: ${path}. Routine write/edit, cp -n, mkdir and touch only. Sensitive/config/agent-policy targets still ask. Cleared on reload or session change.`, "info");
      } else if (text === "off") {
        generation++;
        roots.clear();
        granted.clear();
        once.clear();
        update(ctx);
        ctx.ui.notify("All session permission grants cleared; pending items remain unresolved.", "info");
      } else if (text === "pending") {
        const entries = [...pending.entries()];
        if (!entries.length) { ctx.ui.notify("No deferred permission decisions.", "info"); return; }
        const labels = entries.map(([, p], i) => `${i + 1}. ${p.verdict.rule}: ${preview(p.tool, p.input)}`);
        const selected = await ctx.ui.select("Deferred permissions — nothing has executed", labels, { timeout: APPROVAL_TIMEOUT_MS, signal: ctx.signal });
        if (epoch !== generation || ctx.signal?.aborted) return;
        const index = labels.indexOf(selected ?? "");
        if (index < 0) return;
        const [key, p] = entries[index];
        const decision = await ctx.ui.select(`${p.verdict.rule}: ${p.verdict.why}\nWorking directory: ${p.cwd}\n${preview(p.tool, p.input)}`, ["Keep deferred", "Allow next exact call once", "Dismiss"], { timeout: APPROVAL_TIMEOUT_MS, signal: ctx.signal });
        if (epoch !== generation || ctx.signal?.aborted) return;
        if (decision === "Allow next exact call once") {
          once.add(key);
          pending.delete(key);
          pi.sendMessage({ customType: "permission-resolved", content: `User approved one retry of the deferred ${p.tool} call in working directory ${p.cwd}: ${preview(p.tool, p.input)}. Use the original exact arguments from the transcript, not this possibly truncated preview. Recheck that the action is still needed; nothing has executed.`, display: true }, { triggerTurn: true });
        } else if (decision === "Dismiss") pending.delete(key);
        update(ctx);
      } else {
        ctx.ui.notify(`File scopes: ${[...roots].join(", ") || "none"}. ${pending.size} deferred decisions. Use /permissions auto <directory>, off, or pending.`, "info");
      }
    },
  });

  pi.on("tool_call", async (event, ctx) => {
    const epoch = generation;
    const input = (event.input ?? {}) as Record<string, unknown>;
    const scope = scopeFor(ctx.cwd ?? process.cwd());
    const v = judge(event.toolName, input, scope);
    if (!v) return undefined;
    const base = { tool: event.toolName, class: v.class, rule: v.rule, programs: v.programs };
    const cwd = resolvePath(scope.cwd, scope);
    const key = `${cwd}\u0000${event.toolName}\u0000${JSON.stringify(input)}`;
    const deferred = () => ({ block: true, reason: `Command policy ${v.rule}: deferred, NOT denied or approved. Nothing executed. Await the user via /permissions pending. Continue only independent work; do not retry this action through another command or tool. If all remaining work depends on it, stop and report the dependency.` });

    if (v.class === "allow") {
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
    if (once.delete(key) || v.class === "ask" && granted.has(key)) {
      logDecision({ ...base, decision: "exact-grant" });
      return undefined;
    }
    if (pending.has(key)) {
      logDecision({ ...base, decision: "already-deferred" });
      return deferred();
    }
    if (v.class === "ask" && v.rule === "A1" && roots.size) {
      const paths = [...roots];
      const auto = (event.toolName === "write" || event.toolName === "edit")
        ? scopedWrite(String(input.path), scope, paths)
        : event.toolName === "bash" && scopedCommand(String(input.command), scope, paths);
      if (auto) {
        logDecision({ ...base, decision: "task-file-scope" });
        return undefined;
      }
    }
    if (!ctx.hasUI) {
      logDecision({ ...base, decision: "refused-no-ui" });
      return { block: true, reason: `Command policy ${v.rule}: ${v.why}. No user is present to approve it. Continue independent work only; do not bypass this gate.` };
    }

    const shown = preview(event.toolName, input);
    const choice = await ctx.ui.select(
      v.class === "ask" ? `Approve ${event.toolName}? (${v.rule}: ${v.why})\n\n  ${shown}`
        : `BLOCKED by rule ${v.rule}: ${v.why}\n\n  ${shown}\n\nThis rule never grants for a session.`,
      v.class === "ask" ? ["Allow once", "Allow this exact call for this session", "Defer", "Deny"] : ["Deny", "Allow once", "Defer"],
      { timeout: APPROVAL_TIMEOUT_MS, signal: ctx.signal },
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
    if (choice === undefined || choice === "Defer") {
      pending.set(key, { tool: event.toolName, cwd, input: structuredClone(input), verdict: v });
      update(ctx);
      logDecision({ ...base, decision: "deferred" });
      return deferred();
    }
    logDecision({ ...base, decision: "denied" });
    return { block: true, reason: `Command policy ${v.rule}: ${v.why}. The user denied it. Do not retry it in another form; ask the user instead.` };
  });
}
