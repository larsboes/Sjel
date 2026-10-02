#!/usr/bin/env bun
// tools/packs-codex.ts — materialize Axon Packs into Codex's user skill root.
//
// The deployment engine lives in tools/lib/pack-deploy.ts and is shared with
// every other harness adapter. What stays here is what is genuinely Codex: where
// its skills go, where its ledger lives, and the one extra file it validates.
//
// Optional harness overlay (not part of pack.toml):
//   Packs/<pack>/codex/<skill>/
// Its contents are merged over the shared skill at deploy time. This is where
// Codex-only agents/openai.yaml metadata and any assets it references belong.
// SKILL.md may not be overridden: shared instructions stay canonical.
//
//   tools/packs-codex status [<pack>|--all]
//   tools/packs-codex deploy <pack>...
//   tools/packs-codex sync <pack>|--all
//   tools/packs-codex remove <pack>...
//   tools/packs-codex use [<profile>]

import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline/promises";
import {
  activateProfile,
  availablePacks,
  deployPack,
  getStatuses,
  migrateGeneratedArtifacts,
  printStatuses,
  profileActivePacks,
  readPackSkills,
  readProfiles,
  readState,
  removePack,
  resolveProfilePacks,
  syncPack,
  type DeployConfig,
  type DeploymentState,
  type DesiredFile,
  type Profile,
  type SkillStatus,
  type StatusRow,
} from "./lib/pack-deploy.ts";

// Re-exported because tools/packs-pi.ts, tools/packs-opencode.ts
// and the test suite import them from here. Keeping the surface intact is what
// makes the 2026-08-09 extraction provably behaviour-preserving: the tests did
// not move, so they still test the same entry points.
export {
  activateProfile,
  availablePacks,
  deployPack,
  getStatuses,
  migrateGeneratedArtifacts,
  printStatuses,
  profileActivePacks,
  readPackSkills,
  readProfiles,
  readState,
  removePack,
  resolveProfilePacks,
  syncPack,
  type DeployConfig,
  type DeploymentState,
  type StatusRow,
  type SkillStatus,
};

const SJEL_ROOT = resolve(import.meta.dir, "..");
const HELP = `tools/packs-codex — materialize Axon Packs.

  tools/packs-codex status [<pack>|--all]  show source/install/drift state
  tools/packs-codex deploy <pack>...       install one or more Packs
  tools/packs-codex sync <pack>|--all      update already-deployed Packs
  tools/packs-codex remove <pack>...       remove one or more owned Packs
  tools/packs-codex migrate-generated <pack> --accept-current
                                            adopt generated-artifact exclusions after review
  tools/packs-codex use [<profile>]        activate a profile (or pick interactively)

Environment:
  CODEX_SKILLS_DIR       destination (default: $HOME/.agents/skills)
  SJEL_CODEX_STATE_FILE ownership ledger override (mainly for tests)
`;

/** Codex reads agents/openai.yaml for its own metadata; a malformed one is a deploy-time failure, not a runtime surprise. */
export function validateCodexFiles(files: Map<string, DesiredFile>, label: string): void {
  const openaiYaml = files.get("agents/openai.yaml");
  if (!openaiYaml) return;
  try {
    const parsed = Bun.YAML.parse(readFileSync(openaiYaml.absolutePath, "utf8"));
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      throw new Error("top level must be a mapping");
    }
  } catch (error) {
    throw new Error(`${label}: invalid agents/openai.yaml: ${error}`);
  }
}

export function defaultCodexDeployConfig(): DeployConfig {
  const home = process.env.HOME ?? "";
  const stateHome = process.env.XDG_STATE_HOME ?? join(home, ".local", "state");
  return {
    axonRoot: SJEL_ROOT,
    destination: resolve(process.env.CODEX_SKILLS_DIR ?? join(home, ".agents", "skills")),
    stateFile: resolve(
      process.env.SJEL_CODEX_STATE_FILE ?? join(stateHome, "axon", "pack-deployments", "codex.json"),
    ),
    adapter: "codex",
    stateEnvVar: "SJEL_CODEX_STATE_FILE",
    validateAdapterFiles: validateCodexFiles,
  };
}

async function main(): Promise<void> {
  const home = process.env.HOME ?? "";
  const [command = "status", ...args] = process.argv.slice(2);
  if (command === "-h" || command === "--help" || command === "help") {
    console.log(HELP);
    return;
  }
  const config = defaultCodexDeployConfig();
  try {
    switch (command) {
      case "status": {
        const target = args[0];
        printStatuses(getStatuses(config, target && target !== "--all" ? target : undefined));
        break;
      }
      case "deploy":
        if (args.length === 0) throw new Error("usage: tools/packs-codex deploy <pack>...");
        for (const pack of args) {
          for (const line of deployPack(config, pack)) console.log(line);
        }
        break;
      case "sync": {
        const target = args[0];
        if (!target) throw new Error("usage: tools/packs-codex sync <pack>|--all");
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
        if (args.length === 0) throw new Error("usage: tools/packs-codex remove <pack>...");
        for (const pack of args) {
          for (const line of removePack(config, pack)) console.log(line);
        }
        // ~/.agents/skills is pi's global discovery root (pi docs: skills.md), so on a
        // machine with pi installed the destination has TWO readers and this removal
        // costs pi the skills it was loading from here. Name that cost instead of
        // letting the removal look like a pure cleanup.
        if (config.destination === join(home, ".agents", "skills") && existsSync(join(home, ".pi", "agent", "settings.json"))) {
          console.log(
            `note: pi is installed and discovers ${config.destination} by default, so the removed unit(s) were loaded by pi until now. ` +
              `If pi should keep them, deploy them there before removing next time: tools/packs-pi deploy ${args.join(" ")}`,
          );
        }
        break;
      case "migrate-generated": {
        const pack = args.find((arg) => !arg.startsWith("--"));
        if (!pack || args.some((arg) => arg !== pack && arg !== "--accept-current")) {
          throw new Error(
            "usage: tools/packs-codex migrate-generated <pack> --accept-current",
          );
        }
        for (const line of migrateGeneratedArtifacts(config, pack, args.includes("--accept-current"))) {
          console.log(line);
        }
        break;
      }
      case "profile":
      case "use": {
        const profileName = args[0];
        if (profileName) {
          const profiles = readProfiles(config);
          const profile = profiles.find((p: Profile) => p.name === profileName);
          if (!profile) throw new Error(`no such profile: '${profileName}'`);
          for (const line of activateProfile(config, profile)) console.log(line);
        } else {
          const profiles = readProfiles(config);
          if (profiles.length === 0) { console.log("No profiles defined. Add them to profiles.toml."); break; }
          console.log();
          for (let i = 0; i < profiles.length; i++) {
            const active = profileActivePacks(config, profiles[i]).length > 0 ? " [active]" : "";
            console.log(`  ${(i + 1).toString().padEnd(3)} ${profiles[i].name.padEnd(20)} ${profiles[i].description}${active}`);
          }
          const rl = createInterface({ input: process.stdin, output: process.stdout });
          const answer = await rl.question("\nSelect profile (number or name): ");
          rl.close();
          const num = parseInt(answer, 10);
          let profile: Profile | undefined;
          if (!isNaN(num) && num >= 1 && num <= profiles.length) {
            profile = profiles[num - 1];
          } else {
            profile = profiles.find((p: Profile) => p.name === answer);
          }
          if (!profile) { console.log(`No profile matches '${answer}'`); break; }
          for (const line of activateProfile(config, profile)) console.log(line);
        }
        break;
      }
      default:
        throw new Error(`unknown command '${command}'\n\n${HELP}`);
    }
  } catch (error) {
    console.error(`packs-codex: ${(error as Error).message}`);
    process.exit(1);
  }
}

if (import.meta.main) await main();
