import { afterEach, describe, expect, setDefaultTimeout, test } from "bun:test";
import { mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, symlinkSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fmtReceiptKey, runWithReceipt } from "./ci-receipts.ts";

setDefaultTimeout(30_000);

const scratch: string[] = [];
afterEach(() => { for (const dir of scratch.splice(0)) rmSync(dir, { recursive: true, force: true }); });
const fmt = ["cargo", "fmt", "--all", "--check"];

function fixture() {
  const dir = mkdtempSync(join(tmpdir(), "ci-receipts-"));
  scratch.push(dir);
  const root = join(dir, "repo"), bin = join(dir, "bin"), home = join(dir, "home");
  for (const path of [root, bin, home]) mkdirSync(path);
  const env = { PATH: `${bin}:${process.env.PATH}`, HOME: home, CARGO_HOME: join(home, ".cargo"), RUSTUP_HOME: join(home, ".rustup") };
  const version = (tool: string, value: string) => writeFileSync(join(bin, tool), `#!/bin/sh\nprintf '%s\\n' '${value}'\n`, { mode: 0o755 });
  for (const tool of ["cargo", "rustfmt", "bun"]) version(tool, `${tool} fixture 1`);
  const git = (...args: string[]) => {
    const p = Bun.spawnSync(["git", ...args], { cwd: root, env });
    if (p.exitCode !== 0) throw new Error(p.stderr.toString());
    return p.stdout.toString().trim();
  };
  writeFileSync(join(root, ".gitignore"), "target/\n");
  writeFileSync(join(root, "main.rs"), "fn main() {}\n");
  writeFileSync(join(root, "fixture.json"), "{}\n");
  git("init", "-q"); git("add", ".");
  git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false", "commit", "-qm", "fixture");
  return { root, env, version, git, key: (cmd = fmt, e = env) => fmtReceiptKey(root, cmd, e) };
}

describe("format receipts", () => {
  test("reuses success in the git common directory, without changing checkout content", async () => {
    const f = fixture();
    let calls = 0;
    const execute = async () => { calls++; return 0; };
    const before = f.key();
    expect(before).not.toBeNull();
    expect(await runWithReceipt(f.root, fmt, execute, { env: f.env })).toEqual({ code: 0, reused: false, recorded: true });
    expect(await runWithReceipt(f.root, fmt, execute, { env: f.env })).toEqual({ code: 0, reused: true, recorded: false });
    expect(calls).toBe(1);
    expect(f.key()).toBe(before);
    const files = readdirSync(join(f.root, ".git/sjel-ci-receipts"));
    expect(files).toEqual([`${before}.json`]);
    expect(JSON.parse(readFileSync(join(f.root, ".git/sjel-ci-receipts", files[0]), "utf8"))).toEqual({ key: before });
    expect(f.git("status", "--porcelain")).toBe("");
  });

  test("failed checks are never recorded or reused", async () => {
    const f = fixture();
    let calls = 0;
    const execute = async () => { calls++; return 1; };
    for (let i = 0; i < 2; i++) expect(await runWithReceipt(f.root, fmt, execute, { env: f.env })).toEqual({ code: 1, reused: false, recorded: false });
    expect(calls).toBe(2);
  });

  test("full mode bypasses successful receipts and writes none", async () => {
    const f = fixture();
    const execute = async () => 0;
    expect((await runWithReceipt(f.root, fmt, execute, { env: f.env, full: true })).recorded).toBe(false);
    await runWithReceipt(f.root, fmt, execute, { env: f.env });
    expect(await runWithReceipt(f.root, fmt, async () => 1, { env: f.env, full: true })).toEqual({ code: 1, reused: false, recorded: false });
  });

  test("mutation during the check cannot earn a receipt", async () => {
    const f = fixture();
    const result = await runWithReceipt(f.root, fmt, async () => {
      writeFileSync(join(f.root, "fixture.json"), '{"changed":true}');
      return 0;
    }, { env: f.env });
    expect(result).toEqual({ code: 0, reused: false, recorded: false });
    let ran = false;
    await runWithReceipt(f.root, fmt, async () => { ran = true; return 0; }, { env: f.env });
    expect(ran).toBe(true);
  });

  test("tracked fixture changes and deletions invalidate success", async () => {
    const f = fixture();
    await runWithReceipt(f.root, fmt, async () => 0, { env: f.env });
    const before = f.key();
    writeFileSync(join(f.root, "fixture.json"), "changed");
    expect(f.key()).not.toBe(before);
    expect((await runWithReceipt(f.root, fmt, async () => 1, { env: f.env })).code).toBe(1);
    unlinkSync(join(f.root, "fixture.json"));
    expect(f.key()).not.toBe(before);
    expect((await runWithReceipt(f.root, fmt, async () => 1, { env: f.env })).reused).toBe(false);
  });

  test("untracked files, format config and toolchain changes invalidate", () => {
    const f = fixture();
    for (const path of ["new-fixture.txt", "rustfmt.toml", "rust-toolchain.toml", ".cargo/config.toml"]) {
      const before = f.key();
      mkdirSync(join(f.root, path, ".."), { recursive: true });
      writeFileSync(join(f.root, path), "fixture");
      expect(f.key()).not.toBe(before);
      unlinkSync(join(f.root, path));
    }
  });

  test("ignored build artifacts do not invalidate", () => {
    const f = fixture(), before = f.key();
    mkdirSync(join(f.root, "target"));
    writeFileSync(join(f.root, "target/output"), "build");
    expect(f.key()).toBe(before);
  });

  test("versions, formatting environment and exact command participate", () => {
    const f = fixture();
    for (const tool of ["bun", "cargo", "rustfmt"]) {
      const before = f.key();
      f.version(tool, `${tool} fixture 2`);
      expect(f.key()).not.toBe(before);
    }
    const before = f.key();
    expect(fmtReceiptKey(f.root, fmt, { ...f.env, RUSTUP_TOOLCHAIN: "different" })).not.toBe(before);
    expect(fmtReceiptKey(f.root, fmt, { ...f.env, LC_ALL: "C" })).not.toBe(before);
    const gpui = ["cargo", "fmt", "--manifest-path", "apps/gpui/Cargo.toml", "--all", "--check"];
    expect(f.key(gpui)).not.toBe(before);
    expect(f.key(["cargo", "fmt", "--check", "--all"])).toBeNull();
  });

  test("unknown tool overrides and external configuration refuse reuse", () => {
    const f = fixture();
    for (const key of ["RUSTFMT", "RUSTC_WRAPPER", "CARGO_ENCODED_RUSTFLAGS", "BUN_OPTIONS", "DYLD_INSERT_LIBRARIES", "GIT_DIR"]) {
      expect(fmtReceiptKey(f.root, fmt, { ...f.env, [key]: "override" })).toBeNull();
    }
    mkdirSync(f.env.CARGO_HOME, { recursive: true });
    writeFileSync(join(f.env.CARGO_HOME, "config.toml"), "external");
    expect(f.key()).toBeNull();
  });

  test("secret-shaped paths and symlinks are not read for reuse", () => {
    const f = fixture();
    writeFileSync(join(f.root, ".env"), "fixture");
    expect(f.key()).toBeNull();
    unlinkSync(join(f.root, ".env"));
    symlinkSync(join(f.root, "main.rs"), join(f.root, "link.rs"));
    expect(f.key()).toBeNull();
  });

  test("publication, tests and clippy always execute", async () => {
    const f = fixture();
    for (const cmd of [["cargo", "test"], ["cargo", "clippy"], ["tools/check-publication-hygiene.sh"], ["bun", "tools/ci-run.ts", "gates"]]) {
      let calls = 0;
      for (let i = 0; i < 2; i++) expect((await runWithReceipt(f.root, cmd, async () => { calls++; return 0; }, { env: f.env })).reused).toBe(false);
      expect(calls).toBe(2);
    }
  });
});
