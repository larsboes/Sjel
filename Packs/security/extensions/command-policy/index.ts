/**
 * Command policy — pi extension. Classifies every `bash` call, every `vault_exec` command and
 * every `write`/`edit` by the rules in docs/plans/agent.md §3 (see ./classify.ts), then:
 *
 *   allow  runs, no card
 *   ask    card: allow once · allow this exact call for the rest of this pi process · deny
 *   block  card naming the rule: deny (the default) · allow once. Never a standing grant.
 *
 * Without a UI (print mode, a non-interactive subagent) ask and block both refuse: a card nobody
 * can answer must not turn into a yes.
 *
 * Secrets stay with secrets-guard (rule B6's main half); this extension does not duplicate it.
 *
 * Every decision is appended to <overlay>/logs/command-policy.jsonl with the class, the rule
 * and the program names. Arguments are never logged: a command line can carry a token.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { appendFileSync, mkdirSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { classify, classifyWrite, type Scope, type Verdict } from "./classify.ts";

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
  // Session grants, keyed by tool and exact arguments (spec §3, class Ask).
  const granted = new Set<string>();

  pi.on("tool_call", async (event, ctx) => {
    const input = (event.input ?? {}) as Record<string, unknown>;
    const v = judge(event.toolName, input, scopeFor(ctx.cwd ?? process.cwd()));
    if (!v) return undefined;
    const base = { tool: event.toolName, class: v.class, rule: v.rule, programs: v.programs };

    if (v.class === "allow") {
      logDecision({ ...base, decision: "allow" });
      return undefined;
    }
    const key = `${event.toolName}\u0000${JSON.stringify(input)}`;
    if (v.class === "ask" && granted.has(key)) {
      logDecision({ ...base, decision: "session-grant" });
      return undefined;
    }
    if (!ctx.hasUI) {
      logDecision({ ...base, decision: "refused-no-ui" });
      return { block: true, reason: `Command policy ${v.rule}: ${v.why}. No user is present to approve it.` };
    }

    const shown = preview(event.toolName, input);
    let choice: string | undefined;
    if (v.class === "ask") {
      choice = await ctx.ui.select(
        `Approve ${event.toolName}? (${v.rule}: ${v.why})\n\n  ${shown}`,
        ["Allow once", "Allow this exact call for this session", "Deny"],
      );
      if (choice === "Allow this exact call for this session") granted.add(key);
    } else {
      choice = await ctx.ui.select(
        `BLOCKED by rule ${v.rule}: ${v.why}\n\n  ${shown}\n\nThis rule never grants for a session.`,
        ["Deny", "Allow once"],
      );
    }
    const allowed = choice === "Allow once" || choice === "Allow this exact call for this session";
    logDecision({ ...base, decision: allowed ? (choice === "Allow once" ? "allow-once" : "session-grant-added") : "denied" });
    if (allowed) return undefined;
    return {
      block: true,
      reason: `Command policy ${v.rule}: ${v.why}. The user denied it. Do not retry it in another form; ask the user instead.`,
    };
  });
}
