/**
 * Battery admission for the commands an agent starts: a local build or test suite is deferred
 * while the runtime profile is On the go, unless the command itself carries the one-command
 * exception. The decision is crates/runtime's; this file only recognises the command and asks.
 *
 * Wrappers that already ask (tools/cargo-hermetic, tools/ci-local, sjel) are deliberately absent
 * so a refusal is printed once, by the tool that was about to compile.
 */
import { spawnSync } from "node:child_process";
import { join } from "node:path";

const HEAVY: Record<string, RegExp> = {
  cargo: /^(build|check|clippy|test|bench|run|nextest|install|rustc|doc)$/,
  bun: /^(test|run|build|install|x|create)$/,
  npm: /^(test|run|install|ci|exec)$/,
  pnpm: /^(test|run|install|exec)$/,
  yarn: /^(test|run|install|build)$/,
  uv: /^(run|sync|pip|build)$/,
  swift: /^(build|test|run|package)$/,
  xcodebuild: /^/,
  make: /^/,
  cmake: /^/,
  tsc: /^/,
  "svelte-check": /^/,
  "nextest": /^/,
};

const OVERRIDE = /\bSJEL_ALLOW_LOCAL_BUILD=1\b/;

/** The heavy program a shell command starts, or null when nothing in it compiles. */
export function heavyCommand(command: string): string | null {
  for (const raw of command.split(/\n|&&|\|\||[;|&]/)) {
    const words = raw.trim().split(/\s+/).filter(Boolean);
    while (words.length && /^[A-Za-z_][A-Za-z0-9_]*=/.test(words[0])) words.shift();
    const program = words[0]?.replace(/^["']|["']$/g, "").split("/").pop() ?? "";
    const subcommand = words[1] ?? "";
    const heavy = HEAVY[program];
    if (heavy && heavy.test(subcommand)) {
      // A program with no subcommand split (xcodebuild, make) is named on its own.
      return heavy.test("") ? program : `${program} ${subcommand}`;
    }
  }
  return null;
}

/**
 * null when admitted, otherwise the sentence to refuse with.
 *
 * `root` is the checkout whose `tools/local-build` answers; nothing is built to decide.
 */
export function powerDeferral(command: string, root: string, env: NodeJS.ProcessEnv = process.env): string | null {
  const heavy = heavyCommand(command);
  if (!heavy || OVERRIDE.test(command)) return null;
  const check = spawnSync("bash", [join(root, "tools/local-build")], { cwd: root, env, timeout: 10_000, encoding: "utf8" });
  if (check.status !== 3) return null;
  const reason = (check.stderr ?? "").trim();
  return `${heavy} is deferred while this Mac is On the go.${reason ? ` ${reason}` : ""} Plug in, or prefix the command with SJEL_ALLOW_LOCAL_BUILD=1 for this one run.`;
}
