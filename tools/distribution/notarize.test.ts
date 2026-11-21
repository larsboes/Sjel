import { expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const program = join(import.meta.dir, "notarize.ts");
const identity = "Developer ID Application: Fixture (TEAM000000)";

// Fake tools: codesign and stapler append a marker so a stale hash is detectable.
const xcrun = `#!/bin/bash
echo "$*" >> "$FIXTURE/calls"
case "$1 $2" in
  "notarytool submit") echo '{"id":"sub-1","message":"ok"}' ;;
  "notarytool info") printf '{"id":"%s","status":"%s"}\\n' "$3" "$(cat "$FIXTURE/status")" ;;
  "notarytool log") echo "fixture log" ;;
  "stapler staple") echo stapled >> "$3" ;;
  "stapler validate") ;;
  *) exit 64 ;;
esac
`;
const codesign = `#!/bin/bash\necho "codesign $*" >> "$FIXTURE/calls"\necho signed >> "\${@: -1}"\n`;

function fixture() {
  const scratch = mkdtempSync(join(tmpdir(), "sjel-notarize-"));
  const bin = join(scratch, "bin"); mkdirSync(bin);
  writeFileSync(join(bin, "xcrun"), xcrun); chmodSync(join(bin, "xcrun"), 0o755);
  writeFileSync(join(bin, "codesign"), codesign); chmodSync(join(bin, "codesign"), 0o755);
  const out = join(scratch, "out"); mkdirSync(out);
  const name = "Sjel-v0.0.2-aarch64-apple-darwin.dmg";
  writeFileSync(join(out, name), "synthetic image\n");
  const asset = { name, bytes: 16, sha256: createHash("sha256").update("synthetic image\n").digest("hex") };
  writeFileSync(join(out, "release.json"), JSON.stringify({ schema: 1, version: "v0.0.2", signed: true, notarized: false, assets: [asset] }));
  const run = (...args: string[]) => Bun.spawnSync([process.execPath, program, "--dir", out, ...args], {
    env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, FIXTURE: scratch }, stdout: "pipe", stderr: "pipe",
  });
  const status = (value: string) => writeFileSync(join(scratch, "status"), value);
  const catalogue = () => JSON.parse(readFileSync(join(out, "release.json"), "utf8"));
  const digest = () => createHash("sha256").update(readFileSync(join(out, name))).digest("hex");
  return { scratch, out, run, status, catalogue, digest, calls: () => readFileSync(join(scratch, "calls"), "utf8") };
}

test("submission persists, a pending run exits 75, and acceptance staples and rehashes", () => {
  const f = fixture();
  try {
    expect(f.run().stderr.toString()).toContain("first submission requires");
    f.status("In Progress");
    const pending = f.run("--identity", identity, "--notary-profile", "fixture");
    expect(pending.exitCode).toBe(75);
    expect(f.catalogue().notary).toEqual({ profile: "fixture", submission: "sub-1" });
    expect(f.catalogue().assets[0].sha256).toBe(f.digest());
    expect(f.run().exitCode).toBe(75);
    expect(f.calls().match(/notarytool submit/g)).toHaveLength(1);
    f.status("Accepted");
    const accepted = f.run();
    expect(accepted.stderr.toString()).toBe("");
    expect(accepted.exitCode).toBe(0);
    const done = f.catalogue();
    expect(done.notarized).toBe(true);
    expect(done.notary).toBeUndefined();
    expect(done.assets[0].sha256).toBe(f.digest());
    expect(readFileSync(join(f.out, "SHA256SUMS"), "utf8")).toBe(`${f.digest()}  ${done.assets[0].name}\n`);
    expect(f.run().exitCode).toBe(0);
    expect(f.calls().match(/stapler staple/g)).toHaveLength(1);
  } finally { rmSync(f.scratch, { recursive: true, force: true }); }
});

test("a rejected submission fails and keeps the pending record", () => {
  const f = fixture();
  try {
    f.status("Invalid");
    const result = f.run("--identity", identity, "--notary-profile", "fixture");
    expect(result.exitCode).toBe(1);
    expect(result.stderr.toString()).toContain("ended as Invalid");
    expect(f.catalogue().notarized).toBe(false);
    expect(f.catalogue().notary.submission).toBe("sub-1");
  } finally { rmSync(f.scratch, { recursive: true, force: true }); }
});

test("a changed image is refused before any submission", () => {
  const f = fixture();
  try {
    writeFileSync(join(f.out, f.catalogue().assets[0].name), "tampered\n");
    const result = f.run("--identity", identity, "--notary-profile", "fixture");
    expect(result.stderr.toString()).toContain("asset changed");
  } finally { rmSync(f.scratch, { recursive: true, force: true }); }
});
