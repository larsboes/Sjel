#!/usr/bin/env bun
// tools/generate-marketplace.ts — regenerate the Claude Code plugin marketplace
// (.claude-plugin/marketplace.json, plus one Packs/<pack>/.claude-plugin/plugin.json
// per pack) from pack.toml.
//
// Generated, like ARCHITECTURE.md (CONTRIBUTING.md, "Generated architecture"): never
// hand-edit the JSON this writes, re-run this script instead.
// Verify without writing back: tools/check-marketplace-fresh.sh
//
// This is a second, native Claude Code install path alongside tools/packs-claude
// (which copies a Pack into ~/.claude/skills). A pack installed both ways can
// double-load its skills — pick one path per pack.
//
// Reads only this repo's Packs/, never an overlay's: marketplace.json is a
// committed public artifact, and an overlay Pack is private
// (CONTRIBUTING.md#harness-neutral-packs). A pack whose manifest names a `deployer` is
// owned by that tool alone and is skipped here too, matching every other generic
// adapter (tools/lib/pack-deploy.ts).
//
// Usage: tools/generate-marketplace.ts
// Env: MARKETPLACE_OUT_ROOT — write under this root instead of the repo (the
//   freshness check uses it to generate into a scratch dir and diff).

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { availablePacks, type DeployConfig } from "./lib/pack-deploy.ts";

const SJEL_ROOT = resolve(import.meta.dir, "..");
const OUT_ROOT = resolve(process.env.MARKETPLACE_OUT_ROOT ?? SJEL_ROOT);

// destination/stateFile are unused: this script only reads manifests, it never
// deploys, so it needs no install target or ownership ledger.
const config: DeployConfig = {
  axonRoot: SJEL_ROOT,
  packRoots: [join(SJEL_ROOT, "Packs")],
  destination: "",
  stateFile: "",
  adapter: "marketplace",
};

// LICENSE's copyright line is the one tracked owner fact; every plugin.json's
// author and the marketplace's owner point at it rather than inventing a
// per-pack value pack.toml does not carry.
const MARKETPLACE_OWNER = { name: "Lars Boes", url: "https://github.com/larsboes/Sjel" };

type PackManifest = { name: string; description: string; license?: string };

function readManifest(pack: string): PackManifest {
  const path = join(SJEL_ROOT, "Packs", pack, "pack.toml");
  const parsed = Bun.TOML.parse(readFileSync(path, "utf8")) as Record<string, unknown>;
  if (typeof parsed.description !== "string" || !parsed.description) {
    throw new Error(`${path}: description must be a non-empty string`);
  }
  return {
    name: pack,
    description: parsed.description,
    license: typeof parsed.license === "string" ? parsed.license : undefined,
  };
}

function writeJson(path: string, value: unknown): void {
  mkdirSync(join(path, ".."), { recursive: true });
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

function main(): void {
  const packs = availablePacks(config).sort();
  if (packs.length === 0) throw new Error("no Packs found under Packs/ — nothing to generate");

  for (const pack of packs) {
    const manifest = readManifest(pack);
    writeJson(join(OUT_ROOT, "Packs", pack, ".claude-plugin", "plugin.json"), {
      name: manifest.name,
      description: manifest.description,
      author: MARKETPLACE_OWNER,
      ...(manifest.license ? { license: manifest.license } : {}),
    });
  }

  writeJson(join(OUT_ROOT, ".claude-plugin", "marketplace.json"), {
    name: "axon-packs",
    description: "Togglable agent-skill bundles mirrored from Axon's harness-neutral Packs (Packs/<name>/pack.toml).",
    owner: MARKETPLACE_OWNER,
    metadata: { pluginRoot: "./Packs" },
    plugins: packs.map((name) => ({ name, source: name })),
  });

  console.log(`generated marketplace.json + ${packs.length} plugin.json (${packs.join(", ")})`);
}

if (import.meta.main) {
  try {
    main();
  } catch (error) {
    console.error(`generate-marketplace: ${(error as Error).message}`);
    process.exit(1);
  }
}
