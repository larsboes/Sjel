#!/usr/bin/env bun
// tools/packs-claude.ts — materialize Axon Packs into Claude Code's skill root.
//
// Replaces the symlink deployment tools/packs.sh used to do (principal,
// 2026-08-09: "we should only deploy from axon overlays never using symlinks").
// ~/.claude is a deployment target now, not a set of pointers into this repo:
// nothing under it can write back into a checkout, and a hand edit there is
// reported as drift instead of silently becoming a commit.
//
// The engine is tools/lib/pack-deploy.ts, shared with codex, opencode and pi.
// What is Claude-specific and lives here: the skill root, the ledger path, and
// the agents/ convention.
//
// agents/ — a Pack MAY carry Packs/<pack>/agents/ of Claude-Code-native subagent
// .md files. Claude Code scans agent directories recursively, so the whole
// directory deploys as ONE owned unit at ~/.claude/agents/<pack>. Deliberately
// not a pack.toml field (CONTRIBUTING.md#harness-neutral-packs): it is a Claude-only
// convention, and other harnesses' deployers simply never look for it.
//
// Two things packs.sh did that are gone on purpose:
//   retired_skills — the ledger already knows what it deployed, so a skill dropped
//     from the manifest is removed by `sync` as stale. A hand-maintained list of
//     old names was only ever needed because a symlink carried no memory.
//   silent skipping — packs.sh skipped anything that was "not our link". Here an
//     unowned destination is a reported collision, and `adopt` is the deliberate
//     way to claim one.
//
//   tools/packs-claude status [<pack>|--all]
//   tools/packs-claude deploy <pack>...
//   tools/packs-claude adopt <pack>...
//   tools/packs-claude sync <pack>|--all
//   tools/packs-claude remove <pack>...
//   tools/packs-claude use [<profile>]

import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import {
  activateProfile,
  adoptPack,
  availablePacks,
  deployPack,
  getStatuses,
  printStatuses,
  readProfiles,
  readState,
  removePack,
  syncPack,
  type DeployConfig,
  type Profile,
} from "./lib/pack-deploy.ts";

const SJEL_ROOT = resolve(import.meta.dir, "..");
const HELP = `tools/packs-claude — materialize Axon Packs into Claude Code.

  tools/packs-claude status [<pack>|--all]  show source/install/drift state
  tools/packs-claude deploy <pack>...       install one or more Packs
  tools/packs-claude adopt <pack>...        take ownership of identical copies already in place
  tools/packs-claude sync <pack>|--all      update already-deployed Packs
  tools/packs-claude remove <pack>...       remove one or more owned Packs
  tools/packs-claude use [<profile>]        activate a profile (or list them interactively)

Environment:
  CLAUDE_SKILLS_DIR       skill destination (default: $HOME/.claude/skills)
  CLAUDE_AGENTS_DIR       agents destination (default: $HOME/.claude/agents)
  SJEL_CLAUDE_STATE_FILE  ownership ledger override (mainly for tests)
`;

function expandHome(path: string): string {
  return path === "~" ? (process.env.HOME ?? "") : path.startsWith("~/") ? join(process.env.HOME ?? "", path.slice(2)) : path;
}

function overlayRoot(): string | null {
  if (process.env.SJEL_OVERLAY_ROOT) return expandHome(process.env.SJEL_OVERLAY_ROOT);
  for (const file of [join(SJEL_ROOT, "axon.local.toml"), join(SJEL_ROOT, "axon.toml")]) {
    if (!existsSync(file)) continue;
    const overlay = (Bun.TOML.parse(readFileSync(file, "utf8")) as Record<string, unknown>).overlay;
    if (typeof overlay === "string" && overlay) return expandHome(overlay);
  }
  return null;
}

export function defaultClaudeDeployConfig(): DeployConfig {
  const home = process.env.HOME ?? "";
  const stateHome = process.env.XDG_STATE_HOME ?? join(home, ".local", "state");
  const roots = [resolve(import.meta.dir, "..", "Packs")];
  const overlay = overlayRoot();
  if (overlay && existsSync(join(overlay, "Packs"))) roots.push(join(overlay, "Packs"));
  return {
    axonRoot: SJEL_ROOT,
    packRoots: roots,
    destination: resolve(process.env.CLAUDE_SKILLS_DIR ?? join(home, ".claude", "skills")),
    stateFile: resolve(
      process.env.SJEL_CLAUDE_STATE_FILE ?? join(stateHome, "axon", "pack-deployments", "claude.json"),
    ),
    adapter: "claude",
    stateEnvVar: "SJEL_CLAUDE_STATE_FILE",
    treeConvention: {
      sourceDir: "agents",
      destinationRoot: resolve(process.env.CLAUDE_AGENTS_DIR ?? join(home, ".claude", "agents")),
    },
  };
}

function main(): void {
  const [command = "status", ...args] = process.argv.slice(2);
  if (command === "-h" || command === "--help" || command === "help") {
    console.log(HELP);
    return;
  }
  const config = defaultClaudeDeployConfig();
  try {
    switch (command) {
      case "status": {
        const target = args[0];
        printStatuses(getStatuses(config, target && target !== "--all" ? target : undefined));
        break;
      }
      case "list":
        for (const pack of availablePacks(config)) console.log(pack);
        break;
      case "deploy":
        if (args.length === 0) throw new Error("usage: tools/packs-claude deploy <pack>...");
        for (const pack of args) {
          console.log(pack);
          for (const line of deployPack(config, pack)) console.log(`  ${line}`);
        }
        break;
      case "adopt":
        if (args.length === 0) throw new Error("usage: tools/packs-claude adopt <pack>...");
        for (const pack of args) {
          console.log(pack);
          for (const line of adoptPack(config, pack)) console.log(`  ${line}`);
        }
        break;
      case "use": {
        const profileName = args[0];
        if (profileName) {
          const profiles = readProfiles(config);
          const profile = profiles.find((p: Profile) => p.name === profileName);
          if (!profile) throw new Error(`no such profile: '${profileName}'`);
          for (const line of activateProfile(config, profile)) console.log(line);
        } else {
          const profiles = readProfiles(config);
          if (!profiles.length) throw new Error("no profiles in profiles.toml");
          console.log("Profiles:");
          for (const p of profiles) console.log(`  ${p.name.padEnd(12)} ${p.description}`);
        }
        break;
      }
      case "sync": {
        const target = args[0];
        if (!target) throw new Error("usage: tools/packs-claude sync <pack>|--all");
        if (target === "--all") {
          const state = readState(config);
          for (const pack of Object.keys(state.packs).sort()) {
            console.log(pack);
            for (const line of syncPack(config, pack)) console.log(`  ${line}`);
          }
        } else {
          for (const line of syncPack(config, target)) console.log(line);
        }
        break;
      }
      case "remove":
        if (args.length === 0) throw new Error("usage: tools/packs-claude remove <pack>...");
        for (const pack of args) {
          console.log(pack);
          for (const line of removePack(config, pack)) console.log(`  ${line}`);
        }
        break;
      default:
        throw new Error(`unknown command '${command}'\n\n${HELP}`);
    }
  } catch (error) {
    console.error(`packs-claude: ${(error as Error).message}`);
    process.exit(1);
  }
}

if (import.meta.main) main();
