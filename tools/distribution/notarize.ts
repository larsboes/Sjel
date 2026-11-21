#!/usr/bin/env bun
// Resumable notarization of a prepared Mac output. Submits once, records the submission in
// release.json, and staples on a later run, so a slow Apple queue holds no terminal.
// Workflow: https://developer.apple.com/documentation/security/customizing-the-notarization-workflow
import { createHash } from "node:crypto";
import { readFileSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

export const PENDING_EXIT = 75;

interface Asset { name: string; bytes: number; sha256: string }
interface Catalogue {
  assets: Asset[];
  signed?: boolean;
  notarized?: boolean;
  notary?: { profile: string; submission: string };
  [key: string]: unknown;
}

function capture(argv: string[]) {
  const result = Bun.spawnSync(argv, { stdout: "pipe", stderr: "inherit" });
  if (result.exitCode !== 0) throw new Error(`${argv.slice(0, 3).join(" ")} failed (${result.exitCode})`);
  return result.stdout.toString();
}

function run(argv: string[]) {
  const result = Bun.spawnSync(argv, { stdout: "inherit", stderr: "inherit" });
  if (result.exitCode !== 0) throw new Error(`${argv.slice(0, 3).join(" ")} failed (${result.exitCode})`);
}

function write(dir: string, catalogue: Catalogue) {
  for (const asset of catalogue.assets) {
    const path = join(dir, asset.name);
    asset.bytes = statSync(path).size;
    asset.sha256 = createHash("sha256").update(readFileSync(path)).digest("hex");
  }
  writeFileSync(join(dir, "release.json"), JSON.stringify(catalogue, null, 2) + "\n");
  writeFileSync(join(dir, "SHA256SUMS"), catalogue.assets.map((asset) => `${asset.sha256}  ${asset.name}\n`).join(""));
}

/** Returns "accepted" when the DMG is stapled, "pending" while Apple still processes it. */
export function notarize(dir: string, identity = "", profile = ""): "accepted" | "pending" {
  const catalogue = JSON.parse(readFileSync(join(dir, "release.json"), "utf8")) as Catalogue;
  if (catalogue.notarized === true && !catalogue.notary) return "accepted";
  const images = catalogue.assets.filter((asset) => asset.name.endsWith(".dmg"));
  if (images.length !== 1) throw new Error("notarization needs exactly one .dmg asset in release.json");
  const dmg = join(dir, images[0].name);
  const digest = createHash("sha256").update(readFileSync(dmg)).digest("hex");
  if (digest !== images[0].sha256) throw new Error(`asset changed: ${images[0].name}`);

  if (!catalogue.notary) {
    if (!identity.startsWith("Developer ID Application:") || !profile) {
      throw new Error("first submission requires --identity 'Developer ID Application: …' and --notary-profile NAME");
    }
    if (catalogue.signed !== true) throw new Error("the app in this output is not signed; rebuild with --identity");
    run(["codesign", "--sign", identity, "--timestamp", dmg]);
    const submitted = JSON.parse(capture(["xcrun", "notarytool", "submit", dmg, "--keychain-profile", profile, "--output-format", "json"]));
    if (typeof submitted.id !== "string" || !submitted.id) throw new Error("notarytool returned no submission id");
    catalogue.notarized = false;
    catalogue.notary = { profile, submission: submitted.id };
    write(dir, catalogue);
    console.log(`Submitted ${images[0].name} as ${submitted.id}.`);
  }

  const { profile: saved, submission } = catalogue.notary!;
  const info = JSON.parse(capture(["xcrun", "notarytool", "info", submission, "--keychain-profile", saved, "--output-format", "json"]));
  if (info.status === "In Progress") {
    console.log(`Submission ${submission} is in progress. Run again later: bun tools/distribution/notarize.ts --dir ${dir}`);
    return "pending";
  }
  if (info.status !== "Accepted") {
    Bun.spawnSync(["xcrun", "notarytool", "log", submission, "--keychain-profile", saved], { stdout: "inherit", stderr: "inherit" });
    throw new Error(`submission ${submission} ended as ${String(info.status)}`);
  }
  run(["xcrun", "stapler", "staple", dmg]);
  run(["xcrun", "stapler", "validate", dmg]);
  catalogue.notarized = true;
  delete catalogue.notary;
  write(dir, catalogue);
  console.log(`Stapled ${images[0].name}; release.json and SHA256SUMS now record the stapled image.`);
  return "accepted";
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  if (args.includes("--help")) {
    console.log("bun tools/distribution/notarize.ts --dir OUTPUT [--identity 'Developer ID Application: …' --notary-profile NAME]");
    console.log(`Exit ${PENDING_EXIT}: submission still in progress; run the same command again later.`);
    process.exit(0);
  }
  try {
    const values = new Map<string, string>();
    while (args.length) {
      const flag = args.shift()!;
      if (!["--dir", "--identity", "--notary-profile"].includes(flag) || !args.length) throw new Error(`unknown or incomplete option: ${flag}`);
      if (values.has(flag)) throw new Error(`repeated option: ${flag}`);
      values.set(flag, args.shift()!);
    }
    if (!values.get("--dir")) throw new Error("--dir OUTPUT is required");
    const state = notarize(resolve(values.get("--dir")!), values.get("--identity"), values.get("--notary-profile"));
    if (state === "pending") process.exitCode = PENDING_EXIT;
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
