import { describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { chmodSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, readlinkSync, realpathSync, rmSync, symlinkSync, writeFileSync, copyFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { standaloneInstaller } from "./cli-payload.ts";

const installer = join(import.meta.dir, "cli-install.sh");
const manager = join(import.meta.dir, "cli-management.sh");
const bootstrap = standaloneInstaller("v0.0.3", readFileSync(installer, "utf8"));
const platform = `${process.arch === "arm64" ? "aarch64" : "x86_64"}-${process.platform === "darwin" ? "apple-darwin" : "unknown-linux-gnu"}`;
const hash = (path: string) => createHash("sha256").update(readFileSync(path)).digest("hex");

function fixture() {
  const scratch = realpathSync(mkdtempSync(join(tmpdir(), "sjel-cli-install-")));
  const prefix = join(scratch, "data with spaces", "sjel");
  const bin = join(scratch, "bin with spaces");
  const home = join(scratch, "home");
  mkdirSync(home);
  const env = { ...process.env, HOME: home, SJEL_OVERLAY_ROOT: "", SJEL_ROOT: "", SJEL_CLI_BIN: "", XDG_STATE_HOME: join(home, ".local/state") };
  const run = (...args: string[]) => Bun.spawnSync(["bash", installer, ...args, "--prefix", prefix, "--bin-dir", bin], { env, stdout: "pipe", stderr: "pipe" });
  const cli = (...args: string[]) => Bun.spawnSync(["bash", join(bin, "sjel"), ...args], { env, stdout: "pipe", stderr: "pipe" });
  const archive = (version: string, options: { badBinary?: boolean; platform?: string; link?: boolean; extra?: boolean } = {}) => {
    const stage = join(scratch, `stage-${version}`);
    const payload = join(stage, "sjel");
    mkdirSync(join(payload, "target/release"), { recursive: true });
    mkdirSync(join(payload, "tools/distribution"), { recursive: true });
    writeFileSync(join(payload, "CLI_VERSION"), `${version}\n`);
    writeFileSync(join(payload, "CLI_PLATFORM"), `${options.platform ?? platform}\n`);
    writeFileSync(join(payload, "distribution.json"), JSON.stringify({ kind: "cli", version }));
    writeFileSync(join(payload, "target/release/sjel-cli"), options.badBinary ? "#!/bin/bash\nexit 70\n" : '#!/bin/bash\nprintf "fixture:%s:%s\\n" "$SJEL_ROOT" "$*"\n');
    chmodSync(join(payload, "target/release/sjel-cli"), 0o755);
    copyFileSync(installer, join(payload, "tools/distribution/cli-install.sh"));
    copyFileSync(manager, join(payload, "tools/distribution/cli-management.sh"));
    const files = (path: string, prefix = ""): string[] => readdirSync(path, { withFileTypes: true }).flatMap((entry) => entry.isDirectory() ? files(join(path, entry.name), `${prefix}${entry.name}/`) : [`${prefix}${entry.name}`]);
    const names = files(payload).sort();
    writeFileSync(join(payload, "PAYLOAD_SHA256SUMS"), names.map(name => `${hash(join(payload, name))}  ${name}\n`).join(""));
    if (options.link) symlinkSync("/etc/passwd", join(payload, "escape"));
    if (options.extra) writeFileSync(join(payload, "unexpected"), "not recorded");
    const path = join(scratch, `${version}.tar.gz`);
    expect(Bun.spawnSync(["tar", "-czf", path, "sjel"], { cwd: stage }).exitCode).toBe(0);
    return ["--archive", path, "--sha256", hash(path)];
  };
  const standalone = (args: string[] = [], extraEnv: Record<string, string | undefined> = {}, source = bootstrap) => Bun.spawnSync(["/bin/sh", "-s", "--", ...args, "--prefix", prefix, "--bin-dir", bin], {
    env: { ...env, ...extraEnv }, stdin: Buffer.from(source), stdout: "pipe", stderr: "pipe",
  });
  return { scratch, prefix, bin, home, env, run, cli, archive, standalone, close: () => rmSync(scratch, { recursive: true, force: true }) };
}

describe("direct CLI installation", () => {
  test("POSIX stdin bootstrap supports help, local archives and custom paths", () => {
    const f = fixture();
    try {
      expect(f.standalone(["--help"]).exitCode).toBe(0);
      expect(existsSync(f.prefix)).toBe(false);
      const installed = f.standalone(f.archive("v0.0.3"));
      expect(installed.stderr.toString()).toBe("");
      expect(installed.exitCode).toBe(0);
      expect(installed.stdout.toString()).toContain(`export PATH=${f.bin.replaceAll(" ", "\\ ")}:$PATH`);
      expect(f.cli("--version").stdout.toString()).toBe("v0.0.3\n");
    } finally { f.close(); }
  });

  test("incomplete piped scripts and a missing Bash never start installation", () => {
    const f = fixture();
    try {
      const truncated = bootstrap.slice(0, bootstrap.lastIndexOf("\nmain \"$@\""));
      expect(f.standalone([], {}, truncated).exitCode).toBe(0);
      expect(existsSync(f.prefix)).toBe(false);
      const incompleteBody = bootstrap.slice(0, bootstrap.indexOf("\nSJEL_EMBEDDED_BASH_INSTALLER\n"));
      expect(f.standalone([], {}, incompleteBody).exitCode).not.toBe(0);
      expect(existsSync(f.prefix)).toBe(false);
      const missing = f.standalone([], { PATH: f.scratch });
      expect(missing.exitCode).not.toBe(0);
      expect(missing.stderr.toString()).toContain("Bash is required");
      expect(existsSync(f.prefix)).toBe(false);
      const commands = join(f.scratch, "minimal"); mkdirSync(commands);
      for (const name of ["bash", "uname"]) symlinkSync(Bun.which(name)!, join(commands, name));
      const prerequisite = f.standalone([], { PATH: commands });
      expect(prerequisite.exitCode).not.toBe(0);
      expect(prerequisite.stderr.toString()).toContain("required command is missing");
      expect(existsSync(f.prefix)).toBe(false);
      expect(() => standaloneInstaller("v1.2.3;exit 0", "")).toThrow();
      expect(() => standaloneInstaller("v1.2.3", "SJEL_EMBEDDED_BASH_INSTALLER\n")).toThrow();
    } finally { f.close(); }
  });

  test("help is non-mutating, including nested lifecycle help", () => {
    const f = fixture();
    try {
      expect(f.run("--help").exitCode).toBe(0);
      expect(f.run("update", "--help").exitCode).toBe(0);
      expect(existsSync(f.prefix)).toBe(false);
    } finally { f.close(); }
  });

  test("install, explicit update and rollback retain stable paths without touching harness settings", () => {
    const f = fixture();
    try {
      mkdirSync(join(f.home, ".pi/agent"), { recursive: true });
      const settings = join(f.home, ".pi/agent/settings.json");
      writeFileSync(settings, '{"unowned":true}\n');
      const first = f.run(...f.archive("v0.0.2"));
      expect(first.stderr.toString()).toBe("");
      expect(first.exitCode).toBe(0);
      expect(f.cli("--version").stdout.toString()).toBe("v0.0.2\n");
      expect(f.cli("harnesses", "list").stdout.toString()).toContain(`${f.prefix}/current:harnesses list`);
      expect(f.cli("self", "status").stdout.toString()).toContain("installed: v0.0.2");
      const update = f.cli("self", "update", ...f.archive("v0.0.3"));
      expect(update.stderr.toString()).toBe("");
      expect(update.exitCode).toBe(0);
      expect(f.cli("--version").stdout.toString()).toBe("v0.0.3\n");
      expect(readlinkSync(join(f.prefix, "previous"))).toBe("versions/v0.0.2");
      expect(f.cli("self", "rollback").exitCode).toBe(0);
      expect(f.cli("--version").stdout.toString()).toBe("v0.0.2\n");
      expect(readFileSync(settings, "utf8")).toBe('{"unowned":true}\n');
      expect(lstatSync(join(f.bin, "sjel")).isSymbolicLink()).toBe(true);
      expect(existsSync(join(f.prefix, "overlay"))).toBe(false);
    } finally { f.close(); }
  });

  test("corruption, wrong platform, missing files, links and failed smoke checks preserve the current release", () => {
    const f = fixture();
    try {
      expect(f.run(...f.archive("v0.0.2")).exitCode).toBe(0);
      const badDigest = f.archive("v0.0.3"); badDigest[3] = "0".repeat(64);
      const candidates = [badDigest, f.archive("v0.0.4", { platform: "wrong-platform" }), f.archive("v0.0.5", { badBinary: true }), f.archive("v0.0.6", { link: true }), f.archive("v0.0.7", { extra: true })];
      for (const args of candidates) {
        expect(f.run("update", ...args).exitCode).not.toBe(0);
        expect(readlinkSync(join(f.prefix, "current"))).toBe("versions/v0.0.2");
        expect(existsSync(join(f.prefix, ".install-lock"))).toBe(false);
      }
    } finally { f.close(); }
  });

  test("concurrent installs, edited payloads and unrelated command owners are refused", () => {
    const f = fixture();
    try {
      expect(f.run(...f.archive("v0.0.2")).exitCode).toBe(0);
      mkdirSync(join(f.prefix, ".install-lock"));
      const next = f.archive("v0.0.3");
      expect(f.run("update", ...next).stderr.toString()).toContain("another installation holds");
      rmSync(join(f.prefix, ".install-lock"), { recursive: true });
      writeFileSync(join(f.prefix, "current/CLI_PLATFORM"), "edited\n");
      expect(f.run("update", ...next).stderr.toString()).toContain("payload changed");
      expect(readlinkSync(join(f.prefix, "current"))).toBe("versions/v0.0.2");
    } finally { f.close(); }
    const other = fixture();
    try {
      mkdirSync(other.bin, { recursive: true });
      writeFileSync(join(other.bin, "sjel"), "another package manager owns this\n");
      expect(other.run(...other.archive("v0.0.2")).stderr.toString()).toContain("owned by another installation");
      expect(readFileSync(join(other.bin, "sjel"), "utf8")).toBe("another package manager owns this\n");
      expect(existsSync(join(other.prefix, "current"))).toBe(false);
    } finally { other.close(); }
  });

  test("official-channel checks are read-only and explicit downloads update through trusted checksums", () => {
    const f = fixture();
    try {
      expect(f.run(...f.archive("v0.0.2")).exitCode).toBe(0);
      const next = f.archive("v0.0.3");
      const mockBin = join(f.scratch, "mock-bin"); mkdirSync(mockBin);
      const curl = `#!/bin/bash
set -eu
url=""; out=""
while [ "$#" -gt 0 ]; do
  case "$1" in https://*) url="$1" ;; -o) shift; out="$1" ;; esac
  shift
done
printf '%s\\n' "$url" >> "$MOCK_REQUESTS"
[ "\${MOCK_FAIL:-}" != yes ] || exit 22
case "$url" in
  https://github.com/larsboes/Sjel/releases/latest/download/CLI_VERSION) printf '%s\\n' "\${MOCK_LATEST:-v0.0.3}" > "$out" ;;
  https://github.com/larsboes/Sjel/releases/download/v0.0.3/SHA256SUMS) printf '%s  sjel-cli-v0.0.3-%s.tar.gz\\n' "$MOCK_DIGEST" "$MOCK_PLATFORM" > "$out" ;;
  https://github.com/larsboes/Sjel/releases/download/v0.0.3/sjel-cli-*.tar.gz) cp "$MOCK_ARCHIVE" "$out" ;;
  *) exit 99 ;;
esac
`;
      writeFileSync(join(mockBin, "curl"), curl); chmodSync(join(mockBin, "curl"), 0o755);
      const env = { ...process.env, PATH: `${mockBin}:${process.env.PATH}`, MOCK_ARCHIVE: next[1], MOCK_DIGEST: next[3], MOCK_PLATFORM: platform, MOCK_REQUESTS: join(f.scratch, "requests") };
      const run = (...args: string[]) => Bun.spawnSync(["bash", join(f.bin, "sjel"), "self", ...args], { env, stdout: "pipe", stderr: "pipe" });
      const check = run("check");
      expect(check.exitCode).toBe(0);
      expect(check.stdout.toString()).toContain("available: v0.0.3");
      expect(readlinkSync(join(f.prefix, "current"))).toBe("versions/v0.0.2");
      const failed = Bun.spawnSync(["bash", join(f.bin, "sjel"), "self", "update"], { env: { ...env, MOCK_FAIL: "yes" }, stdout: "pipe", stderr: "pipe" });
      expect(failed.exitCode).not.toBe(0);
      expect(readlinkSync(join(f.prefix, "current"))).toBe("versions/v0.0.2");
      writeFileSync(env.MOCK_REQUESTS, "");
      const piped = f.standalone([], env);
      expect(piped.stderr.toString()).toBe("");
      expect(piped.exitCode).toBe(0);
      expect(readFileSync(env.MOCK_REQUESTS, "utf8")).not.toContain("/latest/");
      expect(readFileSync(env.MOCK_REQUESTS, "utf8")).toContain("/download/v0.0.3/SHA256SUMS");
      expect(run("update").exitCode).toBe(0);
      expect(f.cli("--version").stdout.toString()).toBe("v0.0.3\n");
      const later = Bun.spawnSync(["bash", join(f.bin, "sjel"), "self", "check"], { env: { ...env, MOCK_LATEST: "v0.0.9" }, stdout: "pipe", stderr: "pipe" });
      expect(later.exitCode).toBe(0);
      expect(later.stdout.toString()).toContain("available: v0.0.9");
    } finally { f.close(); }
  });

  for (const mode of ["ok", "http", "badsha", "404", "oversize", "tls", "missing"]) {
    test(`wget-only bootstrap handles ${mode} without curl`, () => {
      const f = fixture();
      try {
        const archive = f.archive("v0.0.3");
        const commands = join(f.scratch, "commands"); mkdirSync(commands);
        for (const name of ["bash", "mkdir", "mktemp", "uname", "rm", "rmdir", "readlink", "ls", "awk", "sort", "uniq", "wc", "tr", "find", "grep", "head", "gzip", "tar", "mv", "ln", "cp", "chmod", "cat", "dirname", "sha256sum", "shasum"]) {
          const path = Bun.which(name);
          if (path) symlinkSync(path, join(commands, name));
        }
        writeFileSync(join(commands, "wget"), `#!/bin/bash
set -eu
url=""
case " $* " in *" --max-redirect=0 "*) ;; *) exit 99 ;; esac
for arg in "$@"; do case "$arg" in https://*|http://*) url="$arg" ;; esac; done
printf '%s\\n' "$url" >> "$MOCK_REQUESTS"
[ "$MOCK_MODE" != tls ] || exit 5
if [ "$MOCK_MODE" = 404 ]; then printf '  HTTP/1.1 404 Not Found\\n' >&2; exit 8; fi
case "$url" in
  https://github.com/larsboes/Sjel/releases/download/v0.0.3/SHA256SUMS)
    scheme=https; [ "$MOCK_MODE" != http ] || scheme=http
    printf '  HTTP/1.1 302 Found\\n  Location: %s://downloads.example.invalid/checksums\\n' "$scheme" >&2
    exit 8 ;;
  https://downloads.example.invalid/checksums)
    printf '%s  sjel-cli-v0.0.3-${platform}.tar.gz\\n' "$MOCK_DIGEST" ;;
  https://github.com/larsboes/Sjel/releases/download/v0.0.3/sjel-cli-*.tar.gz) cat "$MOCK_ARCHIVE" ;;
  https://github.com/larsboes/Sjel/releases/latest/download/CLI_VERSION)
    if [ "$MOCK_MODE" = oversize ]; then printf '%0128d\\n' 0; else printf 'v0.0.3\\n'; fi ;;
  *) exit 99 ;;
esac
`);
        chmodSync(join(commands, "wget"), 0o755);
        if (mode === "missing") rmSync(join(commands, "wget"));
        const env = { ...f.env, PATH: commands, MOCK_MODE: mode, MOCK_ARCHIVE: archive[1], MOCK_DIGEST: mode === "badsha" ? "0".repeat(64) : archive[3], MOCK_REQUESTS: join(f.scratch, "requests") };
        const result = f.standalone([], env);
        if (mode === "ok" || mode === "oversize") {
          expect(result.stderr.toString()).toBe("");
          expect(result.exitCode).toBe(0);
          expect(f.cli("--version").stdout.toString()).toBe("v0.0.3\n");
          const check = Bun.spawnSync(["/bin/bash", join(f.bin, "sjel"), "self", "check"], { env, stdout: "pipe", stderr: "pipe" });
          expect(check.exitCode === 0).toBe(mode === "ok");
          expect(readlinkSync(join(f.prefix, "current"))).toBe("versions/v0.0.3");
        } else {
          expect(result.exitCode).not.toBe(0);
          expect(existsSync(join(f.prefix, "current"))).toBe(false);
          expect(existsSync(join(f.prefix, ".install-lock"))).toBe(false);
        }
        if (mode === "missing") {
          expect(result.stderr.toString()).toContain("curl or wget is required");
          expect(existsSync(f.prefix)).toBe(false);
        }
        const requests = existsSync(env.MOCK_REQUESTS) ? readFileSync(env.MOCK_REQUESTS, "utf8") : "";
        expect(requests).not.toContain("http://downloads.example.invalid");
        expect(existsSync(join(f.home, ".pi"))).toBe(false);
      } finally { f.close(); }
    });
  }

  test("rollback recovers a damaged active binary without deleting it", () => {
    const f = fixture();
    try {
      expect(f.run(...f.archive("v0.0.2")).exitCode).toBe(0);
      expect(f.run("update", ...f.archive("v0.0.3")).exitCode).toBe(0);
      rmSync(join(f.prefix, "current/target/release/sjel-cli"));
      expect(f.cli("self", "rollback").exitCode).toBe(0);
      expect(f.cli("--version").stdout.toString()).toBe("v0.0.2\n");
      expect(existsSync(join(f.prefix, "versions/v0.0.3"))).toBe(true);
    } finally { f.close(); }
  });

  test("a symlinked versions directory cannot redirect an update", () => {
    const f = fixture();
    try {
      expect(f.run(...f.archive("v0.0.2")).exitCode).toBe(0);
      const moved = join(f.scratch, "outside-versions");
      expect(Bun.spawnSync(["mv", join(f.prefix, "versions"), moved]).exitCode).toBe(0);
      symlinkSync(moved, join(f.prefix, "versions"));
      const update = f.run("update", ...f.archive("v0.0.3"));
      expect(update.stderr.toString()).toContain("versions directory must not be a symlink");
      expect(existsSync(join(moved, "v0.0.3"))).toBe(false);
    } finally { f.close(); }
  });

  test("a failed command publication does not activate a new release", () => {
    const f = fixture();
    try {
      expect(f.run(...f.archive("v0.0.2")).exitCode).toBe(0);
      const args = f.archive("v0.0.3");
      rmSync(join(f.bin, "sjel"));
      const script = 'touch "$BLOCK_DIR/.sjel-link-$$"; exec bash "$@"';
      const update = Bun.spawnSync(["bash", "-c", script, "fixture", installer, "update", ...args, "--prefix", f.prefix, "--bin-dir", f.bin], {
        env: { ...process.env, BLOCK_DIR: f.bin }, stdout: "pipe", stderr: "pipe",
      });
      expect(update.exitCode).not.toBe(0);
      expect(update.stderr.toString()).toContain("temporary command link already exists");
      expect(readlinkSync(join(f.prefix, "current"))).toBe("versions/v0.0.2");
      expect(existsSync(join(f.prefix, ".install-lock"))).toBe(false);
    } finally { f.close(); }
  });

  test("invalid arguments do not create installation state", () => {
    const f = fixture();
    try {
      for (const args of [["--version", "../bad"], ["--archive", "/tmp/missing"], ["--unknown"], ["--version", "v1.2.3-alpha.1"]]) {
        expect(f.run(...args).exitCode).not.toBe(0);
        expect(existsSync(f.prefix)).toBe(false);
      }
    } finally { f.close(); }
  });
});
