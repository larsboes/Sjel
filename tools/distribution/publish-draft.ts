#!/usr/bin/env bun
// Explicit maintainer action. Never turns a preparation build into a public release.
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const args = process.argv.slice(2);
if (args.includes("--help")) {
  console.log("bun tools/distribution/publish-draft.ts --dir OUTPUT [--include OUTPUT] --confirm-draft");
  process.exit(0);
}
try {
  if (args.length < 3 || args[0] !== "--dir" || args.at(-1) !== "--confirm-draft") {
    throw new Error("usage: --dir OUTPUT --confirm-draft (creates a remote draft release)");
  }
  const dir = resolve(args[1]);
  const directories = [dir];
  for (let index = 2; index < args.length - 1; index += 2) {
    if (args[index] !== "--include" || !args[index + 1] || args[index + 1].startsWith("--")) throw new Error("expected --include OUTPUT");
    directories.push(resolve(args[index + 1]));
  }
  const root = resolve(import.meta.dir, "../..");
  const catalogues = directories.map((path) => JSON.parse(readFileSync(join(path, "release.json"), "utf8")));
  for (const catalogue of catalogues) {
    if (catalogue.schema !== 1 || !/^v\d+\.\d+\.\d+$/.test(catalogue.version) || !Array.isArray(catalogue.assets) || !catalogue.assets.length) {
      throw new Error("invalid release catalogue");
    }
    if (!catalogue.source || catalogue.source.repository !== "https://github.com/larsboes/Sjel"
      || catalogue.source.dirty !== false || catalogue.source.rebuilt !== true) {
      throw new Error("publication requires rebuilt artifacts from a clean release checkout");
    }
    if (catalogue.version !== catalogues[0].version || catalogue.source.commit !== catalogues[0].source.commit) throw new Error("all artifacts must describe the same release commit");
    const tag = Bun.spawnSync(["git", "rev-list", "-n", "1", catalogue.version], { cwd: root, stdout: "pipe", stderr: "pipe" });
    if (tag.exitCode !== 0 || tag.stdout.toString().trim() !== catalogue.source.commit) throw new Error("artifact source does not match the release tag");
  }
  const paths: string[] = [];
  const assets: Array<{ name: string; bytes: number; sha256: string; platform: string; signed: boolean; notarized: boolean }> = [];
  const capabilities: Array<{ name: string; version: string; platform: string; requires: string[]; feature: Record<string, unknown>; asset: string; bytes: number; sha256: string }> = [];
  const names = new Set<string>();
  for (let index = 0; index < catalogues.length; index++) {
    const catalogue = catalogues[index];
    for (const asset of catalogue.assets) {
      if (!/^[a-zA-Z0-9][a-zA-Z0-9._-]+\.(dmg|tar\.gz)$/.test(asset.name)) throw new Error("invalid asset filename");
      if (names.has(asset.name)) throw new Error(`repeated asset name: ${asset.name}`);
      names.add(asset.name);
      const path = join(directories[index], asset.name);
      const digest = createHash("sha256").update(readFileSync(path)).digest("hex");
      if (digest !== asset.sha256 || statSync(path).size !== asset.bytes) throw new Error(`asset changed: ${asset.name}`);
      paths.push(path);
      assets.push({ ...asset, platform: catalogue.platform, signed: catalogue.signed === true, notarized: catalogue.notarized === true });
    }
    if (catalogue.capabilities !== undefined && !Array.isArray(catalogue.capabilities)) {
      throw new Error("invalid capability package catalogue");
    }
    for (const capability of catalogue.capabilities ?? []) {
      const featureKeys = new Set(["schema", "title", "summary", "category", "recommend_topics", "data_access"]);
      if (capability.version !== catalogue.version || capability.platform !== catalogue.platform
        || !/^[a-z0-9][a-z0-9-]{0,79}$/.test(capability.name)
        || !Array.isArray(capability.requires)
        || capability.requires.some((name: unknown) => typeof name !== "string" || !/^[a-z0-9][a-z0-9-]{0,79}$/.test(name))
        || capability.requires.includes(capability.name)
        || !capability.feature || capability.feature.schema !== 1
        || Object.keys(capability.feature).some((key) => !featureKeys.has(key))
        || typeof capability.feature.title !== "string" || !capability.feature.title.trim() || capability.feature.title.length > 80
        || typeof capability.feature.summary !== "string" || !capability.feature.summary.trim() || capability.feature.summary.length > 240
        || typeof capability.feature.category !== "string" || !capability.feature.category.trim() || capability.feature.category.length > 40
        || !Array.isArray(capability.feature.recommend_topics) || !capability.feature.recommend_topics.length || capability.feature.recommend_topics.length > 24
        || capability.feature.recommend_topics.some((topic: unknown) => typeof topic !== "string" || !/^[a-z0-9][a-z0-9-]{0,47}$/.test(topic))
        || new Set(capability.feature.recommend_topics).size !== capability.feature.recommend_topics.length
        || !Array.isArray(capability.feature.data_access) || capability.feature.data_access.length > 32
        || capability.feature.data_access.some((item: unknown) => typeof item !== "string" || !item.trim() || item.length > 120)
        || new Set(capability.feature.data_access).size !== capability.feature.data_access.length) {
        throw new Error("invalid capability package entry");
      }
      const asset = catalogue.assets.find((candidate: { name: string }) => candidate.name === capability.asset);
      if (!asset || asset.sha256 !== capability.sha256 || asset.bytes !== capability.bytes
        || capability.asset !== `sjel-${capability.name}-${capability.version}-${capability.platform}.tar.gz`
        || !/^[0-9a-f]{64}$/.test(capability.sha256)
        || !Number.isSafeInteger(capability.bytes) || capability.bytes < 0
        || new Set(capability.requires).size !== capability.requires.length
        || capability.requires.includes(capability.name)) {
        throw new Error(`capability package does not match its release asset: ${capability.name}`);
      }
      if (capabilities.some((known) => known.name === capability.name && known.platform === capability.platform)) {
        throw new Error(`repeated capability package: ${capability.name} (${capability.platform})`);
      }
      capabilities.push({ ...capability });
    }
  }
  const stage = mkdtempSync(join(tmpdir(), "sjel-draft-catalogue-"));
  try {
    const version = catalogues[0].version;
    writeFileSync(join(stage, "release.json"), JSON.stringify({ schema: 1, version, source: catalogues[0].source, friend_ready: false, assets, capabilities }, null, 2) + "\n");
    writeFileSync(join(stage, "SHA256SUMS"), assets.map((asset) => `${asset.sha256}  ${asset.name}\n`).join(""));
    const result = Bun.spawnSync(["gh", "release", "create", version, "--draft", "--verify-tag",
      "--title", `Sjel ${version} — preparation`, "--notes-file", join(dir, "READINESS.txt"),
      ...paths, join(stage, "release.json"), join(stage, "SHA256SUMS")], { cwd: root, stdout: "inherit", stderr: "inherit" });
    process.exitCode = result.exitCode;
  } finally { rmSync(stage, { recursive: true, force: true }); }
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
}
