import { describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dir, "../..");
const program = join(import.meta.dir, "prepare.ts");

describe("release preparation (no installation)", () => {
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
