#!/usr/bin/env bun
// tools/pack-drift-hook — a Claude Code hook that makes Pack drift visible at the
// moment it matters, instead of the next time somebody happens to run a status.
//
// Two events, one script:
//
//   SessionStart (matcher "startup") — if any deployed skill differs from its
//     Pack source, say so once, at the top of the session. Silent when clean.
//
//   FileChanged (matcher "SKILL.md") — when a SKILL.md under a harness skill root
//     is written, say that the file is a DEPLOYED COPY and name both moves:
//     `accept` keeps the edit, `sync` discards it. This is the moment the
//     information is worth having; a week later the edit is either lost or
//     mysterious.
//
// Contract: this hook never blocks and never fails loudly. It exits 0 in every
// path, and prints nothing at all when there is nothing to report — a hook that
// speaks every session gets muted, and a muted hook reports nothing forever.
//
// Wire it in ~/.claude/settings.json (see tools/harnesses and the harness Pack's
// README for the exact block).

import { existsSync } from "node:fs";
import { relative, resolve } from "node:path";
import { HARNESSES, isInstalled } from "./lib/harness-registry.ts";
import { getStatuses, packUnits, readState } from "./lib/pack-deploy.ts";

const SJEL_ROOT = resolve(import.meta.dir, "..");

type HookInput = {
  hook_event_name?: string;
  source?: string;
  file_paths?: string[];
};

function materializedHarnesses() {
  return HARNESSES.filter((h) => h.model === "materialized" && isInstalled(h));
}

function sessionStart(): string[] {
  const lines: string[] = [];
  for (const harness of materializedHarnesses()) {
    const config = harness.config();
    for (const row of getStatuses(config)) {
      if (row.status !== "drifted") continue;
      const unit = packUnits(config, row.pack).find((u) => u.key === row.skill);
      const source = unit ? relative(SJEL_ROOT, unit.sourceRoot) : `${row.pack}/${row.skill}`;
      lines.push(`  ${harness.id}: ${row.pack}/${row.skill} — the installed copy differs from ${source}`);
    }
  }
  if (!lines.length) return [];
  return [
    "Axon Pack drift, deployed copies that no longer match their source:",
    ...lines,
    "  Keep the edit: tools/harnesses accept <pack> <skill> --from <harness>",
    "  Discard it:    tools/harnesses sync <pack>",
    "  Detail:        tools/harnesses drift --diff",
  ];
}

function fileChanged(paths: string[]): string[] {
  const lines: string[] = [];
  for (const path of paths) {
    const full = resolve(path);
    for (const harness of materializedHarnesses()) {
      const config = harness.config();
      if (!full.startsWith(`${config.destination}/`)) continue;
      const skill = relative(config.destination, full).split("/")[0];
      const owner = Object.entries(readState(config).packs).find(([, record]) => record.skills[skill]);
      if (!owner) continue;
      const unit = packUnits(config, owner[0]).find((u) => u.key === skill);
      if (!unit || !existsSync(unit.sourceRoot)) continue;
      lines.push(
        `${path} is a DEPLOYED COPY, not the source.`,
        `  Source:        ${relative(SJEL_ROOT, unit.sourceRoot)} (Pack '${owner[0]}', harness ${harness.id})`,
        `  Keep this edit: tools/harnesses accept ${owner[0]} ${skill} --from ${harness.id}`,
        `  Next sync of Pack '${owner[0]}' refuses to run until one of those happens.`,
      );
    }
  }
  return lines;
}

async function main(): Promise<void> {
  let input: HookInput = {};
  try {
    const raw = await Bun.stdin.text();
    if (raw.trim()) input = JSON.parse(raw) as HookInput;
  } catch {
    return; // Malformed hook input is the harness's problem, not a reason to shout.
  }
  const event = input.hook_event_name ?? "";
  const lines =
    event === "SessionStart"
      ? sessionStart()
      : event === "FileChanged"
        ? fileChanged(input.file_paths ?? [])
        : [];
  if (lines.length) console.log(lines.join("\n"));
}

try {
  await main();
} catch {
  // Never fail a session over a status check.
}
