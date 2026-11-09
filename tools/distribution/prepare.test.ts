import { describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { chmodSync, copyFileSync, existsSync, linkSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { cliBinaries, detachHardlinks, payloadFiles, selectCliSources, verifyLocalImports } from "./cli-payload.ts";

const root = resolve(import.meta.dir, "../..");
const program = join(import.meta.dir, "prepare.ts");

function cliFixture(scratch: string) {
  const fixture = join(scratch, "repo");
  mkdirSync(fixture);
  const listing = Bun.spawnSync(["git", "ls-files", "-z"], { cwd: root, stdout: "pipe" });
  expect(listing.exitCode).toBe(0);
  for (const path of listing.stdout.toString().split("\0").filter(Boolean)) {
    if (!(path.startsWith("Packs/") || path.startsWith("tools/") || path.startsWith("schemas/") || !path.includes("/"))) continue;
    if (!existsSync(join(root, path))) continue;
    mkdirSync(dirname(join(fixture, path)), { recursive: true });
    copyFileSync(join(root, path), join(fixture, path));
  }
  // The installer belongs to another change; packaging tests never execute these stand-ins.
  for (const name of ["cli-install.sh", "cli-management.sh"]) {
    writeFileSync(join(fixture, "tools/distribution", name), "#!/bin/bash\nexit 99\n");
    chmodSync(join(fixture, "tools/distribution", name), 0o755);
  }
  copyFileSync(join(import.meta.dir, "cli-payload.ts"), join(fixture, "tools/distribution/cli-payload.ts"));
  for (const args of [["init", "-q"], ["add", "."], ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture"]]) {
    const run = Bun.spawnSync(["git", ...args], { cwd: fixture, stdout: "pipe", stderr: "pipe" });
    expect(run.exitCode).toBe(0);
  }
  const target = join(scratch, "target");
  mkdirSync(join(target, "release"), { recursive: true });
  for (const name of [...cliBinaries, "sjel-gpui", "sjel-status", "devices-server"]) {
    writeFileSync(join(target, "release", name), `synthetic ${name}; never executed\n`);
    chmodSync(join(target, "release", name), 0o755);
  }
  return { fixture, target };
}

describe("release preparation (no installation)", () => {
  test("Linux cache hardlinks are materialized before archiving", () => {
    const stage = mkdtempSync(join(tmpdir(), "sjel-hardlinks-"));
    try {
      writeFileSync(join(stage, "one"), "shared bytes\n");
      linkSync(join(stage, "one"), join(stage, "two"));
      expect(lstatSync(join(stage, "one")).nlink).toBe(2);
      detachHardlinks(stage);
      for (const file of ["one", "two"]) {
        expect(lstatSync(join(stage, file)).nlink).toBe(1);
        expect(readFileSync(join(stage, file), "utf8")).toBe("shared bytes\n");
      }
    } finally { rmSync(stage, { recursive: true, force: true }); }
  });
  test("CLI ships only selected coding tooling, prebuilt wrappers and complete payload hashes", () => {
    const scratch = mkdtempSync(join(tmpdir(), "sjel-cli-package-"));
    try {
      const { fixture, target } = cliFixture(scratch);
      const sentinel = join(fixture, "Packs/harness/skills/sjel/private-untracked.txt");
      writeFileSync(sentinel, "must not ship\n");
      const output = join(scratch, "output");
      const run = Bun.spawnSync([process.execPath, join(fixture, "tools/distribution/prepare.ts"), "cli", "--version", "v0.0.2", "--target-dir", target, "--out", output], { cwd: fixture, stdout: "pipe", stderr: "pipe" });
      expect(run.stderr.toString()).toBe("");
      expect(run.exitCode).toBe(0);
      const catalogue = JSON.parse(readFileSync(join(output, "release.json"), "utf8"));
      expect(catalogue.friend_ready).toBe(false);
      expect(catalogue.source.dirty).toBe(true);
      expect(catalogue.source.rebuilt).toBe(false);
      expect(catalogue.capabilities).toEqual([]);
      const archive = catalogue.assets.find((asset: { name: string }) => asset.name.endsWith(".tar.gz"));
      expect(archive.name).toBe(`sjel-cli-v0.0.2-${catalogue.platform}.tar.gz`);
      expect(catalogue.assets.map((asset: { name: string }) => asset.name)).toContain("CLI_VERSION");
      for (const asset of catalogue.assets) {
        const bytes = readFileSync(join(output, asset.name));
        expect(bytes.length).toBe(asset.bytes);
        expect(createHash("sha256").update(bytes).digest("hex")).toBe(asset.sha256);
        expect(readFileSync(join(output, "SHA256SUMS"), "utf8")).toContain(`${asset.sha256}  ${asset.name}\n`);
      }
      const extracted = join(scratch, "extracted");
      mkdirSync(extracted);
      expect(Bun.spawnSync(["tar", "-xzf", join(output, archive.name), "-C", extracted]).exitCode).toBe(0);
      const payload = join(extracted, "sjel");
      const read = (path: string) => readFileSync(join(payload, path), "utf8");
      expect(JSON.parse(read("distribution.json"))).toEqual({ schema: 1, kind: "cli", version: "v0.0.2", platform: catalogue.platform, source_builds: false, core: [] });
      expect(read("CLI_VERSION")).toBe("v0.0.2\n");
      expect(readFileSync(join(output, "CLI_VERSION"), "utf8")).toBe(read("CLI_VERSION"));
      expect(read("CLI_PLATFORM")).toBe(catalogue.platform + "\n");
      expect(payloadFiles(join(payload, "target/release"))).toEqual([...cliBinaries].sort());
      expect(existsSync(join(payload, "apps"))).toBe(false);
      expect(existsSync(join(payload, "capabilities"))).toBe(false);
      const profiles = Bun.TOML.parse(read("profiles.toml"));
      expect(Object.values(profiles.harness_profile)).toEqual(Object.keys(profiles.harness_profile).map(() => "coding-harness"));
      expect(profiles.profile.map((profile: { name: string }) => profile.name)).toEqual(["coding-harness"]);
      expect(profiles.unit_hold.pi).toContain("harness/pi-packages/accordion");
      const selected = selectCliSources(fixture);
      for (const [pack, skills] of selected.packs) {
        expect(existsSync(join(payload, `Packs/${pack}/pack.toml`))).toBe(true);
        expect(Bun.TOML.parse(read(`Packs/${pack}/pack.toml`)).skills).toEqual(skills);
        for (const skill of skills) expect(existsSync(join(payload, `Packs/${pack}/skills/${skill}/SKILL.md`))).toBe(true);
      }
      for (const path of ["Packs/knowledge", "Packs/learning", "Packs/skills", "Packs/anatomy", "Packs/writing/skills/academic-writing", "Packs/writing/agents", "Packs/harness/pi-packages/accordion", "Packs/harness/pi-packages/pi-subagents", "Packs/harness/skills/sjel/private-untracked.txt", "tools/templates/overlay-skeleton"]) expect(existsSync(join(payload, path))).toBe(false);
      expect(JSON.parse(read("CLI_RUNTIME.json")).pi_dependencies).toBe("unprepared");
      for (const [wrapper, binary] of [["claude-code-config", "sjel-claude-config"], ["sjel-mcp", "sjel-mcp"]]) {
        const text = read(`tools/${wrapper}/${wrapper}`);
        expect(text).toContain(`target/release/${binary}`);
        expect(text).not.toContain("cargo");
        expect(text).not.toContain("find ");
      }
      for (const helper of ["harnesses", "packs-pi", "packs-claude", "packs-codex", "packs-opencode", "packs-copilot", "updates", "inference-keys", "local-build", "distribution/cli-install.sh", "distribution/cli-management.sh", "lib/pi-agent-file.ts", "templates/pi/settings.base.json"]) expect(existsSync(join(payload, "tools", helper))).toBe(true);
      expect(Bun.TOML.parse(read("sjel.toml")).overlay).toBe("~/.local/share/sjel/overlay");
      expect(existsSync(join(payload, "tools/update.sh"))).toBe(false);
      const sums = read("PAYLOAD_SHA256SUMS").trimEnd().split("\n");
      expect(sums.length).toBe(payloadFiles(payload).length - 1);
      for (const line of sums) {
        expect(line).toMatch(/^[a-f0-9]{64}  [^\r\n]+$/);
        expect(createHash("sha256").update(readFileSync(join(payload, line.slice(66)))).digest("hex")).toBe(line.slice(0, 64));
      }
      expect(readFileSync(join(output, "READINESS.txt"), "utf8")).toContain("CLI PREPARATION");
    } finally { rmSync(scratch, { recursive: true, force: true }); }
  }, 30_000);

  test("curated Pi package sources retain their transitive local imports without worktree dependencies", () => {
    const scratch = mkdtempSync(join(tmpdir(), "sjel-cli-imports-"));
    try {
      const { fixture } = cliFixture(scratch);
      const stage = join(scratch, "selected");
      for (const path of selectCliSources(fixture).paths) {
        mkdirSync(dirname(join(stage, path)), { recursive: true });
        copyFileSync(join(fixture, path), join(stage, path));
      }
      expect(() => verifyLocalImports(stage)).not.toThrow();
      rmSync(join(stage, "Packs/security/extensions/command-policy/classify.ts"));
      expect(() => verifyLocalImports(stage)).toThrow("missing local payload import");
    } finally { rmSync(scratch, { recursive: true, force: true }); }
  }, 30_000);

  test("CLI selection refuses symlinked parents before copying and invalid profile subsets", () => {
    const scratch = mkdtempSync(join(tmpdir(), "sjel-cli-refusal-"));
    try {
      const { fixture } = cliFixture(scratch);
      rmSync(join(fixture, "tools/templates/pi"), { recursive: true });
      symlinkSync(join(root, "tools/templates/pi"), join(fixture, "tools/templates/pi"));
      expect(() => selectCliSources(fixture)).toThrow("symlink");
      rmSync(join(fixture, "tools/templates/pi"));
      const profiles = readFileSync(join(fixture, "profiles.toml"), "utf8");
      writeFileSync(join(fixture, "profiles.toml"), profiles.replace('skills = { "writing" = ["human-writing", "skill-creator", "unslop"] }', 'skills = { "writing" = ["not-a-skill"] }'));
      expect(() => selectCliSources(fixture)).toThrow("invalid coding-harness skill subset");
    } finally { rmSync(scratch, { recursive: true, force: true }); }
  }, 30_000);

  test("node runtimes retain the managed FDA helper path resolver", () => {
    const scratch = mkdtempSync(join(tmpdir(), "sjel-node-fda-"));
    try {
      const target = join(scratch, "target");
      mkdirSync(join(target, "release"), { recursive: true });
      for (const name of ["sjel-cli", "sjel-status", "devices-server", "entities-server", "sjel-storage", "sjel-capability-auth"]) {
        writeFileSync(join(target, "release", name), "synthetic fixture; never executed\n");
        chmodSync(join(target, "release", name), 0o755);
      }
      const output = join(scratch, "output");
      const run = Bun.spawnSync([process.execPath, program, "node", "--version", "v0.0.2", "--target-dir", target, "--out", output], { cwd: root, stdout: "pipe", stderr: "pipe" });
      expect(run.stderr.toString()).toBe("");
      expect(run.exitCode).toBe(0);
      const catalogue = JSON.parse(readFileSync(join(output, "release.json"), "utf8"));
      const archive = catalogue.assets.find((asset: { name: string }) => asset.name.endsWith(".tar.gz"));
      const extracted = join(scratch, "extracted");
      mkdirSync(extracted);
      expect(Bun.spawnSync(["tar", "-xzf", join(output, archive.name), "-C", extracted]).exitCode).toBe(0);
      const home = join(scratch, "home");
      const path = Bun.spawnSync(["bash", join(extracted, "sjel/tools/fda-launcher/install"), "path"], { env: { ...process.env, HOME: home, SJEL_ROOT: join(extracted, "sjel") }, stdout: "pipe", stderr: "pipe" });
      expect(path.exitCode).toBe(0);
      expect(path.stdout.toString()).toBe(join(home, "Library/Application Support/Sjel/helpers/sjel-fda-launcher") + "\n");
    } finally { rmSync(scratch, { recursive: true, force: true }); }
  }, 30_000);

  test("an optional capability becomes a bounded prebuilt archive with matching hashes", () => {
    const scratch = mkdtempSync(join(tmpdir(), "sjel-package-test-"));
    try {
      const target = join(scratch, "target");
      mkdirSync(join(target, "release"), { recursive: true });
      const body = "synthetic binary fixture; never executed\n";
      writeFileSync(join(target, "release/calendar-server"), body);
      chmodSync(join(target, "release/calendar-server"), 0o755);
      const output = join(scratch, "output");
      const run = Bun.spawnSync([process.execPath, program, "capability", "--version", "v1.2.3", "--target-dir", target, "--out", output], { cwd: root, stdout: "pipe", stderr: "pipe" });
      expect(run.exitCode).toBe(0);
      const catalogue = JSON.parse(readFileSync(join(output, "release.json"), "utf8"));
      expect(catalogue.friend_ready).toBe(false);
      expect(catalogue.signed).toBe(false);
      const asset = catalogue.assets[0];
      const archive = join(output, asset.name);
      expect(createHash("sha256").update(readFileSync(archive)).digest("hex")).toBe(asset.sha256);
      const member = (name: string) => {
        const child = Bun.spawnSync(["tar", "-xOf", archive, "--", name], { stdout: "pipe", stderr: "pipe" });
        expect(child.exitCode).toBe(0);
        return child.stdout;
      };
      const manifest = JSON.parse(member("package.json").toString());
      expect(manifest.name).toBe("calendar");
      expect(manifest.files.map((file: { path: string }) => file.path)).toEqual(["bin/calendar-server", "service.toml"]);
      const service = Bun.TOML.parse(member("payload/service.toml").toString());
      expect(service.build).toBeUndefined();
      expect(service.command).toEqual(["capabilities/calendar/bin/calendar-server"]);
      for (const file of manifest.files) {
        const bytes = member(`payload/${file.path}`);
        expect(bytes.length).toBe(file.bytes);
        expect(createHash("sha256").update(bytes).digest("hex")).toBe(file.sha256);
      }
      expect(readFileSync(join(output, "READINESS.txt"), "utf8")).toContain("NOT YET CLEARED");
      const publication = Bun.spawnSync([process.execPath, join(import.meta.dir, "publish-draft.ts"), "--dir", output, "--confirm-draft"], { stdout: "pipe", stderr: "pipe" });
      expect(publication.exitCode).not.toBe(0);
      expect(publication.stderr.toString()).toContain("rebuilt artifacts from a clean release checkout");
    } finally { rmSync(scratch, { recursive: true, force: true }); }
  });

  test("invalid versions and owner traversal are refused before writing output", () => {
    const scratch = mkdtempSync(join(tmpdir(), "sjel-package-refusal-"));
    try {
      for (const extra of [["--version", "latest"], ["--version", "v1.2.3", "--capability", "../private"]]) {
        const output = join(scratch, "output");
        const child = Bun.spawnSync([process.execPath, program, "capability", ...extra, "--out", output], { stdout: "pipe", stderr: "pipe" });
        expect(child.exitCode).not.toBe(0);
        expect(existsSync(output)).toBe(false);
      }
    } finally { rmSync(scratch, { recursive: true, force: true }); }
  });
});
