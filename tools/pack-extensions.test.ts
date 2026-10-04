// tools/pack-extensions.test.ts — gate the pi extensions the Packs deploy.
//
// WHAT THIS CHECKS, and why it did not exist before: `Packs/*/extensions/*.ts` is loaded
// by pi at startup as agent code with the power to block tool calls, and it was the only
// TypeScript in this repository that nothing checked. `bun test` never imported it, no
// tsconfig reached it, cargo cannot see it. The first time these files were type-checked
// (2026-09-13) three type errors fell out of two extensions, and the secrets guard's
// env-dump pattern turned out to allow every real `env` dump while blocking `rg
// process.env`. Both classes of defect are invisible at runtime and invisible to CI.
//
// It checks four things: every `Packs/*/extensions/*.ts` typechecks against the *installed
// pi's own types* (drift from pi's API is the failure mode, so a local shim would defeat the
// point); the extensions *pi itself has registered* typecheck too; every extension loads
// under a stub ExtensionAPI without throwing; and secrets-guard's bash heuristics block the
// commands that leak secrets and leave the rest of bash alone, case by case.
//
// Registered extensions live outside this repository (`~/.pi/agent/settings.json`) and are
// included because that is where a third extension, and a third type error, was found. The
// consequence for whoever runs this on their own machine: an extension kept there is held to
// the same standard, and a broken one turns this gate red until it is fixed or unregistered.
// That is the intent — such an extension is otherwise invisible until it runs at startup.
//
// WHY IT SKIPS, and this is the honest cost: the extensions import pi's packages
// (@earendil-works/pi-tui, typebox) and pi's types, none of which the bun-tests job can
// install — that job is deliberately dependency-free (Axon#116), and its comment says
// every test there imports bun:test, node builtins and local sources only. pi is also
// where these files can run at all, so the gate opens exactly where the code is live: on a
// machine with pi installed, visible as a skip anywhere else. A green run on a machine
// without pi means "not applicable", not "verified", and bun prints the skip count so that
// reads as a skip.
//
// THE COMPILER IS RESOLVED, NEVER FETCHED. A gate that downloads a compiler goes red on a
// plane, so this uses the dashboard's typescript, then `tsc` on PATH, then a typescript
// already in bun's cache — and fails with instructions when none of the three is there,
// rather than reporting "typecheck passed" after checking nothing.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { harnessById, isInstalled } from "./lib/harness-registry.ts";

const SJEL_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const PI = harnessById("pi");

/* ── where pi lives ──────────────────────────────────────────────────────── */

interface PiInstall {
  /** The installed @earendil-works/pi-coding-agent package directory. */
  packageDir: string;
  /** Where its dependencies (pi-tui, pi-ai, typebox, @types/node) resolve from. */
  modulesDir: string;
}

/**
 * The marker in `harness-registry` decides whether to run (it is what CI lacks), and this
 * finds the package that makes the check possible. Both are needed: the marker without a
 * package is a broken install and should be loud, not a skip.
 */
function findPiInstall(): PiInstall | undefined {
  const candidates: string[] = [];
  const onPath = spawnSync("sh", ["-c", "command -v pi"], { encoding: "utf8" }).stdout?.trim();
  if (onPath) {
    // Realpath first, then climb: `pi` is normally a symlink into the package (Homebrew,
    // ~/.bun/bin), and a lexical climb from the link lands in /opt/homebrew/bin or ~/.bun/bin
    // — directories that hold no package at all.
    try {
      let dir = dirname(realpathSync(onPath));
      for (let i = 0; i < 6 && dir !== dirname(dir); i++) {
        candidates.push(dir);
        dir = dirname(dir);
      }
    } catch {
      // A wrapper that is not a file we can resolve: the known global roots below still apply.
    }
  }
  candidates.push(
    join(homedir(), ".bun", "install", "global", "node_modules", "@earendil-works", "pi-coding-agent"),
    "/opt/homebrew/lib/node_modules/@earendil-works/pi-coding-agent",
    "/usr/local/lib/node_modules/@earendil-works/pi-coding-agent",
    "/usr/lib/node_modules/@earendil-works/pi-coding-agent",
  );

  // pi's MANAGED install (`~/.pi/agent/install`, the layout `pi update` moved to in 1.0.x): the
  // package and its dependencies are siblings under one release's node_modules, not a package
  // with its own node_modules beside it, so the loop below cannot express it.
  const managedRoot = process.env.PI_MANAGED_INSTALL_ROOT ?? join(homedir(), ".pi", "agent", "install");
  const versionFile = join(managedRoot, "current-version");
  if (existsSync(versionFile)) {
    const version = readFileSync(versionFile, "utf8").trim();
    if (version) {
      const modulesDir = join(managedRoot, "releases", version, "node_modules");
      const found = piInstallAt(join(modulesDir, "@earendil-works", "pi-coding-agent"), modulesDir);
      if (found) return found;
    }
  }

  for (const candidate of candidates) {
    const found = piInstallAt(candidate, join(candidate, "node_modules"));
    if (found) return found;
  }
  return undefined;
}

/** Both must be there: the manifest proves it is pi, the dependency proves the extensions resolve. */
function piInstallAt(packageDir: string, modulesDir: string): PiInstall | undefined {
  if (!existsSync(join(packageDir, "package.json")) || !existsSync(join(modulesDir, "typebox"))) return undefined;
  try {
    const manifest = JSON.parse(readFileSync(join(packageDir, "package.json"), "utf8")) as { name?: string };
    if (manifest.name !== "@earendil-works/pi-coding-agent") return undefined;
  } catch {
    return undefined;
  }
  return { packageDir, modulesDir };
}

/**
 * pi is installed when its agent config exists. That is also the CI condition: the runner
 * has neither, so the gate skips there instead of failing on packages it cannot install.
 */
const piConfigured = isInstalled(PI);
const piInstall = piConfigured ? findPiInstall() : undefined;

/* ── the extensions under test ───────────────────────────────────────────── */

interface ExtensionFile {
  pack: string;
  /** Absolute path in the checkout. */
  path: string;
  /** Path relative to the repository root, which is also its path in the fixture. */
  rel: string;
}

/**
 * Discovered, never listed: a new `Packs/<pack>/extensions/*.ts`, or a new directory
 * extension `<pack>/extensions/<name>/index.ts`, is gated by existing. Directory
 * extensions are included because that is where an extension with sidecars lives
 * (`inference-keys` keeps its vault helper and shell tools beside its entry), and a
 * gate that only walked the top level would let the one file pi actually loads go
 * unchecked. The sidecars themselves are not extensions and are not gated here.
 */
function packExtensions(): ExtensionFile[] {
  const found: ExtensionFile[] = [];
  for (const pack of readdirSync(join(SJEL_ROOT, "Packs")).sort()) {
    const dir = join(SJEL_ROOT, "Packs", pack, "extensions");
    if (!existsSync(dir)) continue;
    for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      if (entry.isDirectory()) {
        const entryPoint = join(dir, entry.name, "index.ts");
        if (!existsSync(entryPoint)) continue;
        found.push({ pack, path: entryPoint, rel: join("Packs", pack, "extensions", entry.name, "index.ts") });
        continue;
      }
      const name = entry.name;
      if (!name.endsWith(".ts") || name.endsWith(".test.ts")) continue;
      found.push({ pack, path: join(dir, name), rel: join("Packs", pack, "extensions", name) });
    }
  }
  return found;
}

/**
 * The extensions pi actually has registered, read from its settings. They live outside the
 * repository on purpose (a machine's own harness config), and they are included because one
 * of them is where a type error was found: the gate is about what loads at startup, not
 * about what happens to be committed. Ones that are pack files are left to the pack test,
 * so a broken file is reported once rather than twice.
 */
function registeredExtensions(): ExtensionFile[] {
  const settingsPath = join(homedir(), ".pi", "agent", "settings.json");
  if (!existsSync(settingsPath)) return [];
  const inPacks = new Set(packExtensions().map((file) => resolve(file.path)));
  try {
    const settings = JSON.parse(readFileSync(settingsPath, "utf8")) as { extensions?: unknown };
    if (!Array.isArray(settings.extensions)) return [];
    return settings.extensions
      .filter((entry): entry is string => typeof entry === "string" && entry.endsWith(".ts"))
      .filter((entry) => existsSync(entry) && !inPacks.has(resolve(entry)))
      .map((entry) => ({ pack: "pi settings", path: entry, rel: join("registered", entry.replace(/^\//, "")) }));
  } catch {
    return [];
  }
}

/* ── fixture: a directory where the extensions can resolve pi ────────────── */

let fixtureDir: string | undefined;

/**
 * Extension imports resolve from the importing file's real path, so a copy has to sit
 * somewhere with a `node_modules` that reaches pi — the checkout has none, and the gate
 * must not write into it. Copies keep the repository's own layout so a diagnostic prints a
 * path that maps back to the file it came from.
 */
function fixture(): string {
  if (fixtureDir) return fixtureDir;
  const pi = piInstall;
  if (!pi) throw new Error("pi is not installed");

  // Realpath, because macOS resolves /var to /private/var when a process is spawned:
  // the compiler then prints diagnostics relative to a cwd that does not share a prefix
  // with the path in the tsconfig, which turns every line into ../../../../../../ noise.
  const dir = realpathSync(mkdtempSync(join(tmpdir(), "axon-pack-extensions-")));
  const modules = join(dir, "node_modules");
  mkdirSync(join(modules, "@earendil-works"), { recursive: true });
  mkdirSync(join(modules, "@types"), { recursive: true });
  symlinkSync(pi.packageDir, join(modules, "@earendil-works", "pi-coding-agent"));
  for (const name of ["pi-tui", "pi-ai"]) {
    symlinkSync(join(pi.modulesDir, "@earendil-works", name), join(modules, "@earendil-works", name));
  }
  symlinkSync(join(pi.modulesDir, "typebox"), join(modules, "typebox"));
  symlinkSync(join(pi.modulesDir, "@types", "node"), join(modules, "@types", "node"));

  for (const file of [...packs, ...registered]) {
    const destination = join(dir, file.rel);
    mkdirSync(dirname(destination), { recursive: true });
    cpSync(file.path, destination);
  }

  fixtureDir = dir;
  return dir;
}

afterAll(() => {
  if (fixtureDir) rmSync(fixtureDir, { recursive: true, force: true });
});

/* ── the compiler ───────────────────────────────────────────────────────── */

interface Compiler {
  cmd: string;
  via: string;
}

/**
 * Resolved in preference order, never fetched: a cached typescript for bunx counts as
 * available, an empty cache does not (that would be a download).
 */
function findCompiler(): Compiler | undefined {
  const dashboard = join(SJEL_ROOT, "dashboard", "node_modules", ".bin", "tsc");
  if (existsSync(dashboard)) return { cmd: dashboard, via: "the dashboard's typescript" };

  const onPath = spawnSync("sh", ["-c", "command -v tsc"], { encoding: "utf8" }).stdout?.trim();
  if (onPath) return { cmd: onPath, via: "tsc on PATH" };

  const cache = join(homedir(), ".bun", "install", "cache");
  if (existsSync(cache) && readdirSync(cache).some((entry) => entry.startsWith("typescript"))) {
    return { cmd: "bunx", via: "a typescript already in bun's cache" };
  }
  return undefined;
}

const compiler = piConfigured ? findCompiler() : undefined;

/** The pi version the extensions were checked against, named in a failure. */
function piVersion(): string {
  try {
    const manifest = JSON.parse(readFileSync(join(piInstall!.packageDir, "package.json"), "utf8")) as { version?: string };
    return manifest.version ?? "(version unknown)";
  } catch {
    return "(version unknown)";
  }
}

const NO_COMPILER =
  "no TypeScript compiler found. Looked for dashboard/node_modules/.bin/tsc, tsc on PATH " +
  "and a typescript in bun's install cache. Install one (`cd dashboard && bun install`, or " +
  "`bun add -g typescript`) and re-run: this gate resolves a compiler rather than fetching " +
  "one, so it never reports a typecheck it did not run.";

/** Typechecks one extension in isolation, the way pi loads it. */
function typecheckWe(file: ExtensionFile, dir: string): { ok: boolean; output: string } {  const tsconfig = join(dir, `tsconfig.${file.pack}-${file.rel.split("/").pop()?.replace(/\.ts$/, "")}.json`);
  writeFileSync(
    tsconfig,
    `${JSON.stringify(
      {
        compilerOptions: {
          target: "es2023",
          module: "esnext",
          moduleResolution: "bundler",
          strict: true,
          noEmit: true,
          skipLibCheck: true,
          allowImportingTsExtensions: true,
          types: ["node"],
        },
        files: [file.rel],
      },
      null,
      2,
    )}\n`,
  );
  const args = compiler?.cmd === "bunx" ? ["tsc", "-p", tsconfig] : ["-p", tsconfig];
  const run = spawnSync(compiler!.cmd, args, { cwd: dir, encoding: "utf8" });
  // Diagnostics arrive with the throwaway fixture's path on the front of every line. The
  // copy keeps the repository layout, so stripping the prefix leaves the path a reader can
  // open. A failure that names a temp directory is a failure nobody can act on.
  const output = `${run.stdout ?? ""}${run.stderr ?? ""}`
    .trim()
    .split("\n")
    .map((line) => line.replaceAll(`${dir}/`, ""))
    .join("\n");
  return { ok: run.status === 0, output };
}

/* ── loading an extension under a stub ExtensionAPI ──────────────────────── */

type Handler = (event: unknown, ctx: unknown) => Promise<unknown>;

interface Loaded {
  factory: unknown;
  handlers: Map<string, Handler[]>;
  tools: string[];
  /**
   * The full tool definitions, not just their names.
   *
   * Names are enough to prove an extension registered something; they are not enough to drive it.
   * The questions widgets are only testable by calling `execute` with a stub context and feeding
   * keystrokes into the widget the extension builds, so the definition object has to survive the
   * load. Kept alongside `tools` rather than replacing it, so every existing assertion still reads
   * the shape it was written against.
   */
  toolDefs: Map<string, ToolDef>;
  commands: string[];
  providers: string[];
}

/** Only the parts the tests reach; the rendering callbacks stay untyped on purpose. */
type ToolDef = {
  name: string;
  execute: (...args: unknown[]) => Promise<unknown>;
};

async function load(file: ExtensionFile, dir: string): Promise<Loaded> {
  const loaded: Loaded = { factory: undefined, handlers: new Map(), tools: [], toolDefs: new Map(), commands: [], providers: [] };
  const api = {
    on: (event: string, handler: Handler) => {
      loaded.handlers.set(event, [...(loaded.handlers.get(event) ?? []), handler]);
    },
    registerTool: (definition: { name: string }) => {
      loaded.tools.push(definition.name);
      loaded.toolDefs.set(definition.name, definition as ToolDef);
    },
    registerCommand: (name: string) => loaded.commands.push(name),
    registerProvider: (id: string) => loaded.providers.push(id),
    sendMessage: () => {},
  };
  const module = (await import(join(dir, file.rel))) as { default?: (api: unknown) => void };
  loaded.factory = module.default;
  if (typeof module.default === "function") module.default(api);
  return loaded;
}

/* ── the gate ────────────────────────────────────────────────────────────── */

const packs = packExtensions();
const registered = registeredExtensions();

/**
 * The skip has to be legible: `bun test` prints a skip count but not what was skipped or
 * why, and "not applicable" reading as "verified" is the failure this file exists to
 * prevent. A skipped test's body never runs, so the reason is stated at load time.
 */
if (!piConfigured) {
  console.log(
    `pack-extensions: not checked — ${PI.marker} does not exist, so pi's packages and types ` +
      `are unavailable. This gate opens where pi is installed, which is the only place these ` +
      `extensions can run. No Packs/*/extensions/*.ts was type-checked or loaded in this run.`,
  );
}

describe.skipIf(!piConfigured)("pack extensions", () => {
  beforeAll(() => {
    if (piConfigured) fixture();
  });

  test("the installed pi was found, or the reason it was not is stated", () => {
    if (!piInstall) {
      // The marker exists, so pi is configured on this machine: a package search that
      // comes up empty is a broken install and must not pass as "nothing to check".
      throw new Error(
        `pi is configured (${PI.marker}) but its package was not found. Set it up so the ` +
          `extensions' imports resolve, or remove the config — this gate will not report a ` +
          `typecheck it could not run.`,
      );
    }
    expect(existsSync(join(piInstall.packageDir, "package.json"))).toBe(true);
    expect(existsSync(join(piInstall.modulesDir, "typebox"))).toBe(true);
  });

  test("every Packs/*/extensions/*.ts typechecks against pi's own types", () => {
    expect(packs.length).toBeGreaterThan(0);
    if (!compiler) throw new Error(NO_COMPILER);
    const dir = fixture();
    const failures: string[] = [];
    for (const file of packs) {
      const { ok, output } = typecheckWe(file, dir);
      if (!ok) failures.push(output);
    }
    // Thrown rather than asserted so the diagnostics print as written: a compiler's
    // file/line/column message is the whole value of this test, and a diff around it is noise.
    if (failures.length > 0) {
      throw new Error(
        `these extensions do not typecheck against pi ${piVersion()}, using ${compiler?.via}:\n\n${failures.join("\n\n")}`,
      );
    }
  });

  test("the extensions pi has registered typecheck too (they live outside this repository)", () => {
    if (registered.length === 0) return;
    if (!compiler) throw new Error(NO_COMPILER);
    const dir = fixture();
    const failures: string[] = [];
    for (const file of registered) {
      const { ok, output } = typecheckWe(file, dir);
      if (!ok) failures.push(output);
    }
    if (failures.length > 0) {
      throw new Error(`these pi-registered extensions do not typecheck:\n\n${failures.join("\n\n")}`);
    }
  });

  test("every extension loads and registers under a stub api", async () => {
    const dir = fixture();
    for (const file of packs) {
      const loaded = await load(file, dir);
      expect(`${file.rel}: ${typeof loaded.factory}`).toBe(`${file.rel}: function`);
      // An extension that loads but registers nothing is dead code, or a registration
      // guarded by something that is not there — either way, not what it looks like.
      const registeredSomething = loaded.tools.length + loaded.commands.length + loaded.handlers.size;
      expect(`${file.rel}: ${registeredSomething > 0}`).toBe(`${file.rel}: true`);
    }
  });

  test("secrets-guard blocks what leaks secrets and leaves the rest of bash alone", async () => {
    const guard = packs.find((file) => file.path.endsWith("secrets-guard.ts"));
    expect(guard).toBeDefined();
    const dir = fixture();
    const loaded = await load(guard!, dir);
    const toolCall = loaded.handlers.get("tool_call")?.[0];
    expect(typeof toolCall).toBe("function");
    const ctx = { ui: { notify: () => {} } };
    const mismatches: string[] = [];

    const blocked = async (toolName: string, input: Record<string, string>) =>
      (await toolCall!({ toolName, input }, ctx) as { block?: boolean } | undefined)?.block === true;

    // Each case is a command plus what must happen to it. The two failures this table was
    // written for are marked: the old pattern required a trailing newline, so a bare `env`
    // passed while a multi-line command merely containing "env" was refused.
    const cases: [what: string, toolName: string, input: Record<string, string>, want: "block" | "allow"][] = [
      ["bare env", "bash", { command: "env" }, "block"],
      ["bare env with a trailing newline", "bash", { command: "env\n" }, "block"],
      ["env piped to grep", "bash", { command: "env | grep -i key" }, "block"],
      ["env piped to sort", "bash", { command: "env | sort" }, "block"],
      ["env redirected to a file", "bash", { command: "env > /tmp/env.txt" }, "block"],
      ["env after a separator", "bash", { command: "echo hi; env" }, "block"],
      ["env on a later line", "bash", { command: "cd /tmp && ls\nenv" }, "block"],
      ["env under sudo", "bash", { command: "sudo env" }, "block"],
      ["env -0", "bash", { command: "env -0" }, "block"],
      ["env in a subshell", "bash", { command: "echo $(env)" }, "block"],
      ["printenv", "bash", { command: "printenv" }, "block"],
      ["printenv of one variable", "bash", { command: "printenv HOME" }, "block"],
      ["echo $API_KEY", "bash", { command: "echo $API_KEY" }, "block"],
      ["cat .env", "bash", { command: "cat .env" }, "block"],
      ["grep a token out of .env", "bash", { command: "grep -n TOKEN .env" }, "block"],
      ["cat a named env file", "bash", { command: "cat myapp.env" }, "block"],
      ["cat .env.local", "bash", { command: "cat .env.local" }, "block"],
      ["bw get", "bash", { command: "bw get item infra" }, "block"],
      ["read of a .env path", "read", { path: "/tmp/x/.env" }, "block"],
      ["edit of a .env path", "edit", { path: "/tmp/x/.env", oldText: "a", newText: "b" }, "block"],
      // Code that mentions env, not code that dumps it.
      ["grep for process.env", "bash", { command: 'grep -n "process.env" index.ts' }, "allow"],
      ["rg for process.env", "bash", { command: "rg process.env src/" }, "allow"],
      ["grep of a code namespace plus the real .env", "bash", { command: "grep -n process.env .env" }, "block"],
      ["rg for import.meta.env", "bash", { command: "rg 'import.meta.env' src/" }, "allow"],
      ["rg for os.environ", "bash", { command: "rg 'os.environ' src/" }, "allow"],
      ["rg for the word env", "bash", { command: "rg 'env' --type ts" }, "allow"],
      ["env as a command prefix", "bash", { command: 'env -i bash -c "echo hi"' }, "allow"],
      ["env with an assignment prefix", "bash", { command: "env FOO=1 make test" }, "allow"],
      ["env var in code, mid-script", "bash", { command: 'ls\nFOO="bar"\nprocess.env.MISSING = 1\necho done' }, "allow"],
      [
        "heredoc writing code-namespace assignments",
        "bash",
        { command: 'cat > t.ts <<\'EOF\'\nprocess.env.PI_BW_BIN = "/tmp/w.sh";\nconst x = process.env.FOO ?? "";\nEOF' },
        "allow",
      ],
      ["a yaml env_file key", "bash", { command: "printf 'env_file: .env\\n' > compose.yml" }, "allow"],
      ["a yaml environment key", "bash", { command: "rg 'environment:' docker-compose.yml" }, "allow"],
      ["the word environment in prose", "bash", { command: 'echo "check the environment first"' }, "allow"],
      ["env in a filename", "bash", { command: "tail -f logs/environment.log" }, "allow"],
      ["source .env, the allowed form", "bash", { command: "source .env && ls" }, "allow"],
      ["read of an ordinary config file", "read", { path: "/tmp/x/config.yaml" }, "allow"],
    ];

    for (const [what, toolName, input, want] of cases) {
      const got = (await blocked(toolName, input)) ? "block" : "allow";
      // Collected rather than asserted per case so one run reports every mismatch: a gate
      // that shows the first of six problems is a gate someone re-runs six times.
      if (got !== want) mismatches.push(`${want.padEnd(5)} ${what} — got ${got}`);
    }
    expect(mismatches).toEqual([]);
  });
});

/* ── the questions widgets ────────────────────────────────────────────────
 *
 * `ask`, `quiz` and `narrow` are TUI widgets, so the only way to test one is to call the registered
 * tool's `execute` with a stub context whose `ui.custom` hands back the widget, feed keystrokes into
 * it, and assert on what the tool returns. That is what this block does.
 *
 * It is not theoretical. The harness caught a real bug in the ask free-text phase the first time it
 * ran: the `free` phase was added but the editor-routing guard still read `phase === "note"`, so a
 * typed answer was parsed as option keys and `o` then Enter recorded nothing. Every other check this
 * repository runs — typecheck, load-without-throwing, the handler tests above — is blind to it,
 * because nothing throws and nothing fails to register.
 *
 * Each widget renders at the end of every drive, so a render-path crash fails the test too.
 */
describe.skipIf(!piConfigured)("pack extensions > questions widgets", () => {
  const theme = new Proxy({}, { get: () => (s: unknown) => s });
  const tui = { requestRender() {}, terminal: { rows: 24, cols: 80 } };

  /** A stub ctx that builds the widget and then feeds it `keys`, resolving with the tool result. */
  function drive(keys: string[]): unknown {
    return {
      mode: "tui",
      ui: {
        custom: (factory: (t: unknown, th: unknown, kb: unknown, done: (r: unknown) => void) => unknown) =>
          new Promise((resolve) => {
            const widget = factory(tui, theme, {}, resolve) as {
              handleInput: (d: string) => void;
              render: (w: number) => string[];
            };
            for (const k of keys) widget.handleInput(k);
            widget.render(80);
          }),
      },
    };
  }

  /** The questions extension's tools, loaded once against the fixture. */
  let toolDefs: Map<string, ToolDef> | undefined;
  async function questionsTools(): Promise<Map<string, ToolDef>> {
    if (toolDefs) return toolDefs;
    const file = packs.find((f) => f.path.endsWith("questions.ts"));
    expect(file).toBeDefined();
    const loaded = await load(file!, fixture());
    expect([...loaded.toolDefs.keys()].sort()).toEqual(["ask", "narrow", "quiz"]);
    toolDefs = loaded.toolDefs;
    return toolDefs;
  }
  const run = async (tool: string, params: unknown, keys: string[], ctx?: unknown) => {
    const defs = await questionsTools();
    const d = defs.get(tool);
    expect(d).toBeDefined();
    return (await d!.execute("id", params, undefined, undefined, ctx ?? drive(keys))) as {
      content: { type: string; text?: string }[];
      details: any;
    };
  };
  const textOf = (r: { content: { text?: string }[] }) => r.content.map((c) => c.text ?? "").join("\n");

  describe("ask free text", () => {
    const round = {
      topic: "t",
      questions: [
        { id: "q1", title: "Storage", prompt: "Where?", options: [{ label: "vault" }, { label: "db" }], recommendedIndex: 0 },
        { id: "q2", title: "Scope", prompt: "How much?", options: [{ label: "all" }, { label: "some" }], recommendedIndex: 1 },
      ],
    };

    test("an answer in the user's own words is not a skip", async () => {
      const r = await run("ask", round, ["o", ..."neither, use the filesystem", "\r", "\r", ..."went with some", "\r"]);
      const a1 = r.details.answers[0];
      expect(a1.freeText).toBe("neither, use the filesystem");
      expect(a1.skipped).toBe(false);
      expect(a1.selectedIndex).toBeNull();
      expect(a1.label).toBeNull();
      expect(textOf(r)).toMatch(/NONE OF THE OPTIONS/);
      expect(textOf(r)).toMatch(/ask differently/);
      // The second question still behaves normally: a choice plus a note.
      expect(r.details.answers[1].label).toBe("some");
      expect(r.details.answers[1].note).toBe("went with some");
      expect(r.details.answers[1].freeText).toBeNull();
    });

    test("skip stays distinct from free text", async () => {
      const r = await run("ask", round, ["s", "s"]);
      expect(r.details.answers.every((a: any) => a.skipped && a.freeText === null)).toBe(true);
      expect(textOf(r)).toMatch(/still open/);
      expect(textOf(r)).not.toMatch(/NONE OF THE OPTIONS/);
    });

    test("Esc leaves the free-text editor and the options still answer", async () => {
      const r = await run("ask", round, ["o", "x", "\x1b", "\r", "\r", "s"]);
      expect(r.details.answers[0].label).toBe("vault");
      expect(r.details.answers[0].freeText).toBeNull();
    });
  });

  describe("quiz contest and review", () => {
    const Q = {
      topic: "t",
      questions: [{ id: "q1", prompt: "Which?", options: ["A", "B"], correctIndex: 0, explanation: "A is right" }],
    };

    test("a contested answer is marked and carries its reason", async () => {
      const r = await run("quiz", Q, ["2", "c", "n", ..."because B looked right", "\r", "\r"]);
      const a = r.details.answers[0];
      expect(a.contested).toBe(true);
      expect(a.note).toBe("because B looked right");
      // The grading itself is untouched: a contest is a claim, not a correction.
      expect(a.correct).toBe(false);
      expect(textOf(r)).toMatch(/CONTESTED/);
      expect(textOf(r)).toMatch(/mode: "review"/);
    });

    test("a correct answer carries no contest and no note", async () => {
      const r = await run("quiz", Q, ["1", "\r"]);
      expect(r.details.answers[0].correct).toBe(true);
      expect(r.details.answers[0].contested).toBe(false);
      expect(r.details.answers[0].note).toBeNull();
      expect(textOf(r)).not.toMatch(/CONTESTED/);
    });

    test("review mode renders the revised verdict and takes an acceptance", async () => {
      const r = await run("quiz",
        { topic: "t", mode: "review", reviews: [{ id: "q1", prompt: "Which?", chosen: "B", correct: "A", accepted: true, reason: "you were right" }] },
        ["\r"]);
      expect(r.details.outcomes[0].accepted).toBe(true);
      expect(textOf(r)).toMatch(/all 1 accepted/);
    });

    test("pushing back carries the reply and forbids moving on", async () => {
      const r = await run("quiz",
        { topic: "t", mode: "review", reviews: [{ id: "q1", prompt: "Which?", chosen: "B", correct: "A", accepted: false, reason: "B is right because A" }] },
        ["p", ..."no, because C", "\r"]);
      expect(r.details.outcomes[0].accepted).toBe(false);
      expect(r.details.outcomes[0].note).toBe("no, because C");
      expect(textOf(r)).toMatch(/PUSHED BACK/);
      expect(textOf(r)).toMatch(/Do not move on while a contest is open/);
    });

    test("review mode refuses an empty review list", async () => {
      expect(textOf(await run("quiz", { topic: "t", mode: "review", reviews: [] }, []))).toMatch(/needs at least one entry/);
    });
  });

  describe("narrow winnowing", () => {
    const round = {
      topic: "t",
      candidates: [
        { id: "c1", oneLine: "shard the store", generator: "analogy" },
        { id: "c2", oneLine: "buy a bigger box", generator: "extreme-scale" },
        { id: "c3", oneLine: "cache at the edge", generator: "inversion" },
      ],
    };

    test("space keeps and advances, so a pile is winnowed in one pass", async () => {
      // The 2026-09-23 regression, as a test: space used to toggle in place, so keeping
      // three candidates cost six keystrokes and a second press on one retracted it. A
      // session over eleven candidates ended `0 kept, 0 rejected, 11 undecided`.
      const r = await run("narrow", round, [" ", " ", " ", "d"]);
      const kept = r.details.verdicts.filter((v: any) => v.kept).map((v: any) => v.id);
      expect(kept).toEqual(["c1", "c2", "c3"]);
      expect(textOf(r)).toMatch(/3 kept, 0 rejected, 0 undecided/);
    });

    test("enter keeps like space, so the ask reflex cannot end a pass", async () => {
      // The second 2026-09-23 regression, and the one a real session hit. `ask` accepts
      // its focused option on Enter, so the reflex a user arrives with is to press Enter
      // to keep a candidate. While Enter finished the pass instead, that reflex ended it:
      // a round over eleven candidates came back `0 kept, 0 rejected, 11 undecided` and
      // the user could only ever make one choice. Finishing is `d` now, so no accept
      // gesture can end a pass.
      const r = await run("narrow", round, ["\r", "\r", "\r", "d"]);
      const kept = r.details.verdicts.filter((v: any) => v.kept).map((v: any) => v.id);
      expect(kept).toEqual(["c1", "c2", "c3"]);
      expect(textOf(r)).toMatch(/3 kept, 0 rejected, 0 undecided/);
    });

    test("finishing is its own key, so it is never an accident", async () => {
      const r = await run("narrow", round, [" ", "d"]);
      expect(r.details.cancelled).toBe(false);
      expect(textOf(r)).toMatch(/1 kept, 0 rejected, 2 undecided/);
    });

    test("keep, drop-with-reason and undecided are three distinct outcomes", async () => {
      // space keeps c1 and moves to c2; `s` skips c2 and moves to c3; `x` drops c3 with a
      // reason. One keystroke per candidate, no arrow keys.
      const r = await run("narrow", round, [" ", "s", "x", ..."too costly", "\r", "d"]);
      const kept = r.details.verdicts.filter((v: any) => v.kept).map((v: any) => v.id);
      const dropped = r.details.verdicts.filter((v: any) => !v.kept);
      expect(kept).toEqual(["c1"]);
      expect(dropped).toHaveLength(1);
      expect(dropped[0].id).toBe("c3");
      expect(dropped[0].why).toBe("too costly");
      // A candidate that was skipped must NOT be recorded as rejected.
      expect(r.details.verdicts.some((v: any) => v.id === "c2")).toBe(false);
      expect(textOf(r)).toMatch(/1 kept, 1 rejected, 1 undecided/);
      expect(textOf(r)).toMatch(/Do not re-propose a rejected candidate/);
    });

    test("a scattered pick is reachable, which is why space advancing needs a skip", async () => {
      // Without `s`, a space that advances could only ever keep a prefix: keeping c1 and c3
      // would mean pressing space twice and keeping c2 as well.
      const r = await run("narrow", round, [" ", "s", " ", "d"]);
      const kept = r.details.verdicts.filter((v: any) => v.kept).map((v: any) => v.id);
      expect(kept).toEqual(["c1", "c3"]);
      expect(textOf(r)).toMatch(/2 kept, 0 rejected, 1 undecided/);
    });

    test("u retracts a verdict rather than space undoing itself", async () => {
      // Space advances, so retracting what it just kept takes a move back: keep c1, `k` to
      // c1, then `u`. This is the key that used to be a second press of space.
      const r = await run("narrow", round, [" ", "k", "u", "d"]);
      expect(r.details.verdicts).toHaveLength(0);
      expect(textOf(r)).toMatch(/0 kept, 0 rejected, 3 undecided/);
    });

    test("a drop with no reason records null, not an empty string", async () => {
      const r = await run("narrow", round, ["x", "\r", "d"]);
      const c1 = r.details.verdicts.find((v: any) => v.id === "c1");
      expect(c1.kept).toBe(false);
      expect(c1.why).toBeNull();
    });

    test("Esc cancels without inventing verdicts", async () => {
      const r = await run("narrow", round, [" ", "\x1b"]);
      expect(r.details.cancelled).toBe(true);
      expect(textOf(r)).toMatch(/stopped early/);
    });

    test("an empty pile is refused rather than opening an empty widget", async () => {
      expect(textOf(await run("narrow", { topic: "t", candidates: [] }, []))).toMatch(/no candidates/);
    });
  });

  describe("every questions tool refuses to hang a non-interactive run", () => {
    test("all three return the graceful error instead of calling ui.custom", async () => {
      const nonTui = { mode: "json", ui: { custom() { throw new Error("ui.custom must not be called"); } } };
      const cases: [string, unknown][] = [
        ["ask", { topic: "t", questions: [{ id: "q", prompt: "p", options: [{ label: "a" }, { label: "b" }] }] }],
        ["quiz", { topic: "t", questions: [{ id: "q", prompt: "p", options: ["a", "b"], correctIndex: 0 }] }],
        ["narrow", { topic: "t", candidates: [{ id: "c", oneLine: "x" }] }],
      ];
      for (const [tool, params] of cases) {
        expect(textOf(await run(tool, params, [], nonTui))).toMatch(/non-interactive/);
      }
    });
  });
});

/**
 * A skipped test's body never runs, so this exists only to put a skip in the count that
 * the message above explains. The discovery assertion next to it is the one check that
 * runs on every machine, pi or not.
 */
describe.skipIf(piConfigured)("pack extensions (not checked in this run)", () => {
  test("needs a machine with pi installed", () => {});
});

test("this gate found the extensions it means to check", () => {
  expect(packs.map((file) => relative(SJEL_ROOT, file.path)).sort()).toContain(
    relative(SJEL_ROOT, join(SJEL_ROOT, "Packs", "security", "extensions", "secrets-guard.ts")),
  );
});
