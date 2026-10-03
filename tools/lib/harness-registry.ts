// tools/lib/harness-registry.ts — the one place that knows which agent harnesses
// exist, how to tell whether one is installed on THIS machine, and which
// DeployConfig drives it.
//
// It exists because nothing asked. Every packs-* adapter writes to a hardcoded
// default destination and the engine creates it, so on 2026-09-07 this machine
// carried three Packs materialized into ~/.agents/skills for a Codex that is not
// installed (no ~/.codex, no codex on PATH), while pi — installed, and one of the
// three harnesses actually in use — had one Pack of fourteen and no row in
// tools/doctor at all. Deployment tracked ADAPTERS THAT EXIST rather than
// HARNESSES THAT ARE INSTALLED, and the two had drifted in both directions.
//
// Each adapter keeps its own CLI and its own ledger. This registry only makes the
// set enumerable, so one tool can ask every harness the same question.

import { existsSync } from "node:fs";
import { join } from "node:path";
import type { DeployConfig } from "./pack-deploy.ts";
import { defaultClaudeDeployConfig } from "../packs-claude.ts";
import { defaultCodexDeployConfig } from "../packs-codex.ts";
import { defaultOpencodeDeployConfig } from "../packs-opencode.ts";
import { defaultPiDeployConfig } from "../packs-pi.ts";

const home = process.env.HOME ?? "";

/**
 * How a harness receives a Pack.
 *
 * This describes the SKILL channel, which is what the status reader dispatches on and
 * what the `destination` and drift reporting below describe. A harness is not
 * required to use one model for every artifact it needs, and one does not:
 *
 * `materialized` — the adapter COPIES the skill to a destination it owns, so a
 * destination edit is drift and the ledger's digest can prove it.
 *
 * `registry` — the harness reads the Pack source in place through a path list in
 * its own settings file. There is no copy, so there is no drift by construction;
 * the failure mode is a registered path that no longer exists.
 *
 * `registry` is NOT a claim that nothing is ever copied. pi registers its skills
 * this way and ALSO materializes agent files, because the pi-subagents extension
 * reads those off disk and ignores settings.json entirely (packs-pi.ts says why).
 * That second channel keeps its own ledger through the shared engine, and
 * `tools/harnesses status` shows it alongside the registered skill rows. Reading
 * this field as "pi copies nothing" is the mistake the sentence above prevents.
 */
export type DeliveryModel = "materialized" | "registry";

export type Harness = {
  id: string;
  label: string;
  /** The path whose presence means this harness is installed. Reported either way. */
  marker: string;
  model: DeliveryModel;
  /** The CLI that owns deployment for this harness. Named in every hint. */
  cli: string;
  config: () => DeployConfig;
};

export const HARNESSES: Harness[] = [
  {
    id: "claude",
    label: "Claude Code",
    marker: join(home, ".claude"),
    model: "materialized",
    cli: "tools/packs-claude",
    config: defaultClaudeDeployConfig,
  },
  {
    id: "codex",
    label: "Codex",
    // The skills land in ~/.agents/skills, which is the shared agent-skill
    // convention rather than anything Codex-specific — so the presence marker is
    // Codex's own config directory. Reading the destination instead is what let
    // three Packs sit there with no Codex installed.
    marker: join(home, ".codex"),
    model: "materialized",
    cli: "tools/packs-codex",
    config: defaultCodexDeployConfig,
  },
  {
    id: "opencode",
    label: "opencode",
    marker: join(home, ".config", "opencode"),
    model: "materialized",
    cli: "tools/packs-opencode",
    config: defaultOpencodeDeployConfig,
  },
  {
    id: "pi",
    label: "pi",
    marker: join(home, ".pi", "agent", "settings.json"),
    // Registry for skills, materialized for agent files — see DeliveryModel. The
    // field stays `registry` because the status reader uses it to pick the
    // settings-based reader for the skill rows, and pi's agent rows come from a second
    // config (defaultPiAgentsDeployConfig) rather than from a different value here.
    model: "registry",
    cli: "tools/packs-pi",
    config: defaultPiDeployConfig,
  },
];

export function harnessById(id: string): Harness {
  const found = HARNESSES.find((h) => h.id === id);
  if (!found) {
    throw new Error(`unknown harness '${id}'; known: ${HARNESSES.map((h) => h.id).join(", ")}`);
  }
  return found;
}

export function isInstalled(harness: Harness): boolean {
  return existsSync(harness.marker);
}

/**
 * Harnesses with no adapter, listed so a report can say "not supported" instead
 * of silently omitting a harness the operator uses every day. A row moves out of
 * here when someone verifies the harness's skill format and writes the adapter —
 * never on the strength of a guess about what it reads.
 */
export const UNSUPPORTED: { id: string; label: string; why: string }[] = [
  {
    id: "antigravity",
    label: "Antigravity",
    why: "no adapter, and no verified skill/extension format for it in this repository. Nothing is installed on this machine to measure against (2026-09-07).",
  },
];
