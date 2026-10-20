// tools/lib/pi-agent-file.ts — translate a Claude-Code-native agent file into the
// shape pi-subagents can load.
//
// Sjel keeps ONE canonical agent file per agent, written for Claude Code, under
// Packs/<pack>/agents/. pi does not read those, for two reasons that are both
// verified against @tintinweb/pi-subagents 0.19.0 and pi's own tool registry:
//
// 1. `tools:` names differ in CASE, and `Glob` does not exist in pi at all.
//    Claude Code's are TitleCase (`Read`, `Grep`, `Glob`); pi's builtins are
//    lowercase and search with `find`. This is not a cosmetic difference: an
//    unrecognised name is reported by pi-subagents as a `tools-error` and dropped
//    from the agent's allowlist (upstream issue #75, "previously this produced a
//    silently broken agent"). A member copied across unchanged would therefore
//    come up with NO read tool, every claim it made would be `[unverified]`, and
//    the council clerk would have nothing to resolve — the check the Pack README
//    calls "the point" would be structurally dead on pi. So an unmapped name is a
//    hard error here, never a passthrough: the failure it prevents is silent.
//
// 2. `model:` is dropped. The Pack pins `sonnet` for members and `opus` for the
//    sceptic and the attack lenses, which is a Claude Code idiom. On pi the
//    recorded decision (2026-09-16) is to inherit the session model, so the
//    tiering described in Packs/deliberation/README.md is Claude-Code-only.
//
// Deliberately NOT a generator for the Claude Code copy: that file is the source
// and stays hand-written. This runs one way only, at deploy time, in packs-pi.

/** Claude Code tool name → pi builtin. Covers the full pi builtin set (7 tools). */
export const PI_TOOL_NAME: Record<string, string> = {
  Read: "read",
  Write: "write",
  Edit: "edit",
  Bash: "bash",
  Grep: "grep",
  Glob: "find",
  LS: "ls",
};

/**
 * The builtin names pi actually registers: `createCodingTools` → read/bash/edit/write,
 * `createReadOnlyTools` → grep/find/ls. Checked against the output of this map so a
 * future mapping typo is caught here rather than by a member with no tools.
 */
export const PI_BUILTIN_TOOL_NAMES: string[] = ["read", "bash", "edit", "write", "grep", "find", "ls"];

/** Values that are selectors rather than tool names, and pass through untouched. */
const SELECTORS = new Set(["*", "all", "none"]);

function translateToolList(value: string, label: string): string {
  const names = value
    .split(",")
    .map((name) => name.trim())
    .filter((name) => name.length > 0);
  if (names.length === 0) throw new Error(`${label}: 'tools:' is present but empty`);
  const translated = names.map((name) => {
    if (SELECTORS.has(name.toLowerCase())) return name.toLowerCase();
    if (name.startsWith("ext:")) return name;
    const mapped = PI_TOOL_NAME[name];
    if (!mapped) {
      throw new Error(
        `${label}: tool '${name}' has no pi equivalent. Claude Code names are TitleCase and pi's are `
          + `lowercase; if this is a new tool, add it to PI_TOOL_NAME in tools/lib/pi-agent-file.ts. `
          + `Known: ${Object.keys(PI_TOOL_NAME).join(", ")}`,
      );
    }
    if (!PI_BUILTIN_TOOL_NAMES.includes(mapped)) {
      throw new Error(`${label}: '${name}' maps to '${mapped}', which is not a pi builtin`);
    }
    return mapped;
  });
  return translated.join(", ");
}

/**
 * Rewrite one Claude-Code-native agent file for pi.
 *
 * The body is returned byte-for-byte. Only the frontmatter is touched: `tools:` is
 * remapped, `model:` is removed, and a provenance comment is inserted so a reader
 * who finds the deployed copy at ~/.pi/agent/agents/ can see where it came from and
 * why the model pin is gone. A generated file is never the place to make an edit.
 *
 * Throws rather than producing a degraded agent:
 *   - no frontmatter block, or an unterminated one
 *   - no `tools:` line, because pi-subagents defaults to ALL SEVEN builtins when
 *     `tools:` is absent, which would hand a read-only contract agent `write` and
 *     `edit`
 *   - a tool name with no pi equivalent
 */
export function translateAgentForPi(content: string, label: string): string {
  const text = content.startsWith("\uFEFF") ? content.slice(1) : content;
  if (!text.startsWith("---")) throw new Error(`${label}: no frontmatter block, refusing to translate`);
  const end = text.indexOf("\n---", 3);
  if (end === -1) throw new Error(`${label}: unterminated frontmatter block`);

  const head = text.slice(4, end);
  const tail = text.slice(end);

  let sawTools = false;
  let droppedModel = false;
  const rewritten: string[] = [];
  for (const line of head.split("\n")) {
    const tools = /^tools:[ \t]*(.*)$/.exec(line);
    if (tools) {
      sawTools = true;
      rewritten.push(`tools: ${translateToolList(tools[1], label)}`);
      continue;
    }
    if (/^model:[ \t]*/.test(line)) {
      droppedModel = true;
      continue;
    }
    rewritten.push(line);
  }
  if (!sawTools) {
    throw new Error(
      `${label}: no 'tools:' line. pi-subagents defaults to all seven builtins, which would give this `
        + `agent write and edit access. Declare the read-only set explicitly.`,
    );
  }

  const provenance = [
    `# Generated by Sjel tools/packs-pi from ${label} — do not edit this copy;`,
    `# edit the Pack and re-run. Tool names are remapped for pi (Glob -> find).`,
    droppedModel
      ? "# The Claude Code 'model:' pin was removed: on pi this agent inherits the session model."
      : "# This agent carried no 'model:' pin.",
  ].join("\n");

  return `---\n${provenance}\n${rewritten.join("\n")}${tail}`;
}
