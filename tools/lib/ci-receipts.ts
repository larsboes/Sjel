import { createHash } from "node:crypto";
import { existsSync, lstatSync, mkdirSync, readFileSync, realpathSync, renameSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { homedir } from "node:os";

/** Explicit environment and full-run policy for format receipt reuse. */
export interface ReceiptOptions {
  full?: boolean;
  env?: NodeJS.ProcessEnv;
}

const modeledEnv = new Set(["PATH", "HOME", "CARGO_HOME", "RUSTUP_HOME", "RUSTUP_TOOLCHAIN", "XDG_CONFIG_HOME", "LANG", "LANGUAGE", "TZ"]);
const sensitivePath = /(^|\/)(?:\.env(?:\..*)?|\.envrc|\.(?:netrc|npmrc|pypirc|ssh|gnupg)|\.?secrets?(?:\..*)?|credentials(?:\..*)?|id_(?:rsa|ed25519|ecdsa)|[^/]+\.(?:pem|key|p12|pfx))(?=\/|$)|(?:^|\/)\.docker\/config\.json$/i;

function fmtCommand(cmd: string[]): boolean {
  return JSON.stringify(cmd) === JSON.stringify(["cargo", "fmt", "--all", "--check"]) ||
    JSON.stringify(cmd) === JSON.stringify(["cargo", "fmt", "--manifest-path", "apps/gpui/Cargo.toml", "--all", "--check"]);
}

function output(root: string, cmd: string[], env: NodeJS.ProcessEnv): string {
  const proc = Bun.spawnSync({ cmd, cwd: root, env, stdout: "pipe", stderr: "pipe" });
  if (proc.exitCode !== 0) throw new Error("receipt input unavailable");
  return proc.stdout.toString();
}

function externalConfig(root: string, env: NodeJS.ProcessEnv): boolean {
  const home = env.HOME ?? homedir();
  const cargoHome = env.CARGO_HOME ?? join(home, ".cargo");
  if (["config", "config.toml"].some((p) => existsSync(join(cargoHome, p)))) return true;
  if (["rustfmt.toml", ".rustfmt.toml"].some((p) => existsSync(join(env.XDG_CONFIG_HOME ?? join(home, ".config"), "rustfmt", p)))) return true;
  for (let dir = dirname(root); ; dir = dirname(dir)) {
    if ([".cargo/config", ".cargo/config.toml", "rustfmt.toml", ".rustfmt.toml", "rust-toolchain", "rust-toolchain.toml"]
      .some((p) => existsSync(join(dir, p)))) return true;
    if (dirname(dir) === dir) break;
  }
  return false;
}

/** Digest all nonignored checkout inputs for exact format checks; return null when reuse is unsafe. */
export function fmtReceiptKey(root: string, cmd: string[], env: NodeJS.ProcessEnv = process.env): string | null {
  if (!fmtCommand(cmd)) return null;
  if (["HOME", "CARGO_HOME", "RUSTUP_HOME", "XDG_CONFIG_HOME"].some((key) => env[key] !== undefined && !isAbsolute(env[key]!))) return null;
  if (Object.keys(env).some((key) => /^(?:CARGO|RUST|BUN|GIT_|LD_|DYLD_)/.test(key) && !modeledEnv.has(key))) return null;
  try {
    root = realpathSync(root);
    if (externalConfig(root, env)) return null;
    const hash = createHash("sha256");
    const add = (value: string | Buffer) => { hash.update(String(Buffer.byteLength(value))); hash.update(":"); hash.update(value); };
    add(JSON.stringify({ schema: 1, root, cmd, os: process.platform, arch: process.arch, runtime: Bun.version }));
    for (const key of Object.keys(env).filter((key) => modeledEnv.has(key) || key.startsWith("LC_")).sort()) {
      add(key); add(env[key] ?? "");
    }
    for (const tool of ["bun", "cargo", "rustfmt"]) {
      const executable = Bun.which(tool, { PATH: env.PATH, cwd: root });
      if (!executable) return null;
      add(realpathSync(executable));
      add(output(root, [tool, "--version"], env));
    }
    add(output(root, ["cargo", "fmt", "--version"], env));
    const paths = [...new Set(output(root, ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], env).split("\0").filter(Boolean))].sort();
    if (paths.some((path) => !path.endsWith("/.env.example") && path !== ".env.example" && sensitivePath.test(path))) return null;
    for (const path of paths) {
      add(path);
      const absolute = resolve(root, path);
      if (!absolute.startsWith(root + "/")) return null;
      // Never follow links outside the content set or read special files.
      if (realpathSync(dirname(absolute)) !== dirname(absolute)) return null;
      let stat;
      try { stat = lstatSync(absolute); } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
        add("deleted"); continue;
      }
      if (!stat.isFile()) return null;
      add(String(stat.mode));
      add(readFileSync(absolute));
    }
    return hash.digest("hex");
  } catch {
    // An unreadable or racing input disables reuse, never the check itself.
    return null;
  }
}

/** Reuse only successful, stable format checks. Full runs always execute and do not write receipts.
 * @throws If the check executor rejects.
 */
export async function runWithReceipt(
  root: string, cmd: string[], execute: () => Promise<number>, options: ReceiptOptions = {},
): Promise<{ code: number; reused: boolean; recorded: boolean }> {
  const env = options.env ?? process.env;
  const before = options.full ? null : fmtReceiptKey(root, cmd, env);
  let receipt: string | null = null;
  if (before) {
    try {
      const common = output(root, ["git", "rev-parse", "--git-common-dir"], env).trim();
      receipt = join(resolve(root, common), "sjel-ci-receipts", `${before}.json`);
      if (existsSync(receipt) && JSON.parse(readFileSync(receipt, "utf8")).key === before && fmtReceiptKey(root, cmd, env) === before) {
        return { code: 0, reused: true, recorded: false };
      }
    } catch {
      // Receipt corruption or unavailable storage requires a fresh check.
      receipt = null;
    }
  }
  const code = await execute();
  if (code !== 0 || !before || !receipt || fmtReceiptKey(root, cmd, env) !== before) {
    return { code, reused: false, recorded: false };
  }
  const temp = `${receipt}.${process.pid}.${crypto.randomUUID()}.tmp`;
  try {
    mkdirSync(dirname(receipt), { recursive: true });
    writeFileSync(temp, JSON.stringify({ key: before }), { mode: 0o600 });
    renameSync(temp, receipt);
    return { code, reused: false, recorded: true };
  } catch {
    // Storage failure cannot turn a successful executed check into a failed check.
    return { code, reused: false, recorded: false };
  }
}
