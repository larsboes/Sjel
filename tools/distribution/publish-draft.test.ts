import { expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

test("draft publication merges identical CLI bootstrap assets across platforms", () => {
  const scratch = mkdtempSync(join(tmpdir(), "sjel-cli-draft-"));
  try {
    const root = join(scratch, "repo");
    mkdirSync(join(root, "tools/distribution"), { recursive: true });
    const program = join(root, "tools/distribution/publish-draft.ts");
    copyFileSync(join(import.meta.dir, "publish-draft.ts"), program);
    const git = (...args: string[]) => {
      const child = Bun.spawnSync(["git", "-c", "core.hooksPath=/dev/null", ...args], { cwd: root, stdout: "pipe", stderr: "pipe" });
      expect(child.exitCode).toBe(0);
      return child.stdout.toString().trim();
    };
    git("init", "-q");
    git("add", ".");
    git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture");
    const commit = git("rev-parse", "HEAD");
    git("tag", "v0.0.2");
    const bin = join(scratch, "bin"); mkdirSync(bin);
    const capture = join(scratch, "capture"); mkdirSync(capture);
    writeFileSync(join(bin, "gh"), '#!/bin/bash\nprintf "%s\\n" "$@" > "$CAPTURE_DIR/args"\nfor arg in "$@"; do case "$arg" in */release.json) cp "$arg" "$CAPTURE_DIR/release.json" ;; esac; done\n');
    chmodSync(join(bin, "gh"), 0o755);
    const directories = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"].map(platform => {
      const dir = join(scratch, platform); mkdirSync(dir);
      const files = new Map([
        ["CLI_VERSION", "v0.0.2\n"], ["cli-install.sh", "synthetic lifecycle installer\n"], ["install.sh", "synthetic POSIX bootstrap\n"],
        [`sjel-cli-v0.0.2-${platform}.tar.gz`, "synthetic archive\n"],
      ]);
      const assets = [...files].map(([name, content]) => {
        writeFileSync(join(dir, name), content);
        return { name, bytes: Buffer.byteLength(content), sha256: createHash("sha256").update(content).digest("hex") };
      });
      writeFileSync(join(dir, "release.json"), JSON.stringify({ schema: 1, version: "v0.0.2", platform, source: { repository: "https://github.com/larsboes/Sjel", commit, dirty: false, rebuilt: true }, assets, capabilities: [] }));
      writeFileSync(join(dir, "READINESS.txt"), "fixture only\n");
      return dir;
    });
    const run = () => Bun.spawnSync([process.execPath, program, "--dir", directories[0], "--include", directories[1], "--confirm-draft"], { env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, CAPTURE_DIR: capture }, stdout: "pipe", stderr: "pipe" });
    const result = run();
    expect(result.stderr.toString()).toBe("");
    expect(result.exitCode).toBe(0);
    const catalogue = JSON.parse(readFileSync(join(capture, "release.json"), "utf8"));
    expect(catalogue.assets).toHaveLength(5);
    expect(catalogue.assets.filter((asset: { name: string }) => asset.name === "install.sh")).toHaveLength(1);
    expect(catalogue.assets.filter((asset: { name: string }) => asset.name === "CLI_VERSION")).toHaveLength(1);
    expect(readFileSync(join(capture, "args"), "utf8")).toContain("--draft\n");
    const metadata = JSON.parse(readFileSync(join(directories[1], "release.json"), "utf8"));
    const content = "different installer\n";
    writeFileSync(join(directories[1], "install.sh"), content);
    Object.assign(metadata.assets.find((asset: { name: string }) => asset.name === "install.sh"), { bytes: Buffer.byteLength(content), sha256: createHash("sha256").update(content).digest("hex") });
    writeFileSync(join(directories[1], "release.json"), JSON.stringify(metadata));
    expect(run().stderr.toString()).toContain("repeated asset name: install.sh");
    // A published release gains only the missing asset and a fresh catalogue.
    const mac = join(scratch, "aarch64-apple-darwin"); mkdirSync(mac);
    const dmg = "Sjel-v0.0.2-aarch64-apple-darwin.dmg";
    writeFileSync(join(mac, dmg), "synthetic image\n");
    writeFileSync(join(mac, "release.json"), JSON.stringify({ schema: 1, version: "v0.0.2", platform: "aarch64-apple-darwin", signed: true, notarized: true,
      source: { repository: "https://github.com/larsboes/Sjel", commit, dirty: false, rebuilt: true },
      assets: [{ name: dmg, bytes: 16, sha256: createHash("sha256").update("synthetic image\n").digest("hex") }], capabilities: [] }));
    writeFileSync(join(directories[1], "install.sh"), "synthetic POSIX bootstrap\n");
    writeFileSync(join(directories[1], "release.json"), JSON.stringify({ ...metadata, assets: JSON.parse(readFileSync(join(capture, "release.json"), "utf8")).assets.filter((asset: { platform: string }) => asset.platform === "aarch64-unknown-linux-gnu" || ["CLI_VERSION", "cli-install.sh", "install.sh"].includes((asset as unknown as { name: string }).name)).map(({ name, bytes, sha256 }: { name: string; bytes: number; sha256: string }) => ({ name, bytes, sha256 })) }));
    writeFileSync(join(bin, "gh"), `#!/bin/bash
if [ "$1 $2" = "release view" ]; then echo '{"assets":[{"name":"install.sh"},{"name":"CLI_VERSION"},{"name":"cli-install.sh"},{"name":"sjel-cli-v0.0.2-x86_64-unknown-linux-gnu.tar.gz"},{"name":"sjel-cli-v0.0.2-aarch64-unknown-linux-gnu.tar.gz"},{"name":"release.json"},{"name":"SHA256SUMS"}]}'; exit 0; fi
printf "%s\\n" "$@" > "$CAPTURE_DIR/args"
for arg in "$@"; do case "$arg" in */release.json) cp "$arg" "$CAPTURE_DIR/release.json" ;; esac; done
`);
    const updated = Bun.spawnSync([process.execPath, program, "--dir", mac, "--include", directories[0], "--include", directories[1], "--update-release"], { env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, CAPTURE_DIR: capture }, stdout: "pipe", stderr: "pipe" });
    expect(updated.stderr.toString()).toBe("");
    expect(updated.exitCode).toBe(0);
    const uploaded = readFileSync(join(capture, "args"), "utf8").split("\n");
    expect(uploaded.slice(0, 4)).toEqual(["release", "upload", "v0.0.2", "--clobber"]);
    expect(uploaded.filter((line) => line.endsWith(".tar.gz") || line.endsWith(".sh"))).toEqual([]);
    expect(uploaded.some((line) => line.endsWith(dmg))).toBe(true);
    expect(JSON.parse(readFileSync(join(capture, "release.json"), "utf8")).assets).toHaveLength(6);
  } finally { rmSync(scratch, { recursive: true, force: true }); }
});
