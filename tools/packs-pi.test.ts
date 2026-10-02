// tools/packs-pi.test.ts — the pi adapter's two delivery channels.
//
// pi is a MIXED adapter: skills and extensions are registered as paths in
// settings.json, while agent files must be MATERIALIZED into one flat directory that
// every pack shares (pi-subagents reads them off disk and does not recurse). This
// file tests the second channel end to end, through the CLI, because the first
// version of it was wrong in a way only a full run exposes:
//
//   packs-pi.ts defines its own local readState() for the settings ledger. Importing
//   readState from pack-deploy and calling it with a DeployConfig silently called the
//   LOCAL one instead — same arity at the call site, different ledger — so the agent
//   channel read the settings ledger, concluded it owned no packs, and skipped
//   removal entirely while reporting success. A direct call to removePack() worked,
//   which is why this is a spawn-the-CLI test and not a unit test.
//
// Every run is pointed at temp paths by env. Nothing here touches ~/.pi.
//
// Run: bun test tools/packs-pi.test.ts

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";

const SJEL_ROOT = resolve(import.meta.dir, "..");
const CLI = join(SJEL_ROOT, "tools", "packs-pi.ts");

let root: string;
let env: Record<string, string>;

function paths() {
  return {
    settings: join(root, "settings.json"),
    state: join(root, "pi.json"),
    agentsState: join(root, "pi-agents.json"),
    agents: join(root, "agents"),
  };
}

/** Run the CLI exactly as a user would, against the temp paths. */
function pi(...args: string[]): { out: string; code: number } {
  const proc = Bun.spawnSync(["bun", "run", CLI, ...args], {
    env: { ...process.env, ...env },
    stdout: "pipe",
    stderr: "pipe",
  });
  return { out: `${proc.stdout.toString()}${proc.stderr.toString()}`, code: proc.exitCode ?? -1 };
}

/**
 * How many agent files the deliberation Pack carries. Derived rather than hardcoded: the count
 * changed when `council-advocate` was added, and a test that has to be edited every time an agent
 * is added is testing the number, not the deployment.
 */
function deliberationAgentCount(): number {
  return readdirSync(join(SJEL_ROOT, "Packs", "deliberation", "agents")).filter((f) => f.endsWith(".md")).length;
}

/**
 * How many skills the deliberation Pack declares. Derived from the manifest rather than hardcoded:
 * the count changed when `diverge` was added, and a test that must be edited every time a skill
 * lands is testing the number, not the deployment. The manifest is also what the deployer reads.
 */
function deliberationSkillCount(): number {
  const manifest = readFileSync(join(SJEL_ROOT, "Packs", "deliberation", "pack.toml"), "utf8");
  const parsed = Bun.TOML.parse(manifest) as { skills?: string[] };
  return parsed.skills?.length ?? 0;
}

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), "axon-packs-pi-test-"));
  const p = paths();
  env = {
    // PI_SETTINGS_FILE, not SJEL_PI_SETTINGS_FILE: that is what packs-pi.ts reads, and
    // getting it wrong means every run edits the operator's real ~/.pi/agent/settings.json.
    // A first pass of this file did exactly that, which is why the name is called out here.
    PI_SETTINGS_FILE: p.settings,
    SJEL_PI_STATE_FILE: p.state,
    SJEL_PI_AGENTS_STATE_FILE: p.agentsState,
    SJEL_PI_AGENTS_DIR: p.agents,
  };
  writeFileSync(p.settings, "{}\n");
});

afterEach(() => rmSync(root, { recursive: true, force: true }));

describe("deploying a pack with agent files", () => {
  test("writes every agent file into the flat agents root", () => {
    const result = pi("deploy", "deliberation");
    expect(result.code).toBe(0);
    expect(result.out).toContain(`${deliberationAgentCount()} agent file(s)`);
    for (const name of [
      "cause-hypothesis",
      "council-advocate",
      "council-clerk",
      "council-cost",
      "council-evidence",
      "council-owner",
      "council-skeptic",
      "red-team-lens",
    ]) {
      expect(existsSync(join(paths().agents, `${name}.md`))).toBe(true);
    }
    // The list above and the count must agree, or a new agent is deployed without being asserted.
    expect(new Set([
      "cause-hypothesis",
      "council-advocate",
      "council-clerk",
      "council-cost",
      "council-evidence",
      "council-owner",
      "council-skeptic",
      "red-team-lens",
    ]).size).toBe(deliberationAgentCount());
  });

  test("no per-pack subdirectory, because pi would not read one", () => {
    pi("deploy", "deliberation");
    expect(existsSync(join(paths().agents, "deliberation"))).toBe(false);
  });

  test("the deployed clerk has pi tool names and no Claude Code model pin", () => {
    pi("deploy", "deliberation");
    const clerk = readFileSync(join(paths().agents, "council-clerk.md"), "utf8");
    // Frontmatter only. The body is prose that legitimately says "never write" and
    // "You have Read, Grep and Glob", so asserting over the whole file would fail on
    // the instructions that make the contract readable.
    const front = clerk.slice(4, clerk.indexOf("\n---", 3));
    expect(front).toContain("tools: read, grep, find");
    expect(front).not.toContain("Read");
    expect(front).not.toMatch(/^model:/m);
    // The read-only contract survives translation: no write tool in the allowlist.
    const tools = front.split("\n").find((line) => line.startsWith("tools:")) ?? "";
    expect(tools).not.toContain("write");
    expect(tools).not.toContain("bash");
  });

  test("status reports the agent rows as current, meaning the transform is not drift", () => {
    pi("deploy", "deliberation");
    const result = pi("status", "deliberation");
    expect(result.out).toContain("deliberation/agents/council-clerk.md: current");
    expect(result.out).not.toContain("drifted");
  });

  test("the settings ledger still records the skills, so both channels moved", () => {
    pi("deploy", "deliberation");
    const settings = JSON.parse(readFileSync(paths().settings, "utf8"));
    const deployed = settings.skills.filter((p: string) => p.includes("Packs/deliberation/skills"));
    expect(deployed.length).toBe(deliberationSkillCount());
  });
});

describe("the two extension shapes", () => {
  /**
   * Extensions the security Pack carries, derived from disk: one flat `*.ts` and one
   * directory with sidecars. Derived rather than listed for the reason the other counts
   * are — a test that has to be edited whenever an extension lands is testing the list.
   */
  const extensionDir = join(SJEL_ROOT, "Packs", "security", "extensions");

  test("a directory extension registers its index.ts, and nothing else from the directory", () => {
    const result = pi("deploy", "security");
    expect(result.code).toBe(0);
    const settings = JSON.parse(readFileSync(paths().settings, "utf8"));
    const registered: string[] = settings.extensions.filter((p: string) => p.includes("Packs/security/extensions"));
    expect(registered).toContain(join(extensionDir, "secrets-guard.ts"));
    expect(registered).toContain(join(extensionDir, "inference-keys", "index.ts"));
    // The sidecars are reached by the entry, not loaded: registering bw-key.mjs would ask
    // pi to load a vault helper as an extension.
    expect(registered.filter((p) => !p.endsWith(".ts"))).toEqual([]);
    expect(registered.filter((p) => p.includes("inference-keys")).length).toBe(1);
  });

  test("status names a directory extension by its own name, not by `index.ts`", () => {
    pi("deploy", "security");
    const result = pi("status", "security");
    expect(result.out).toContain("security/extensions/inference-keys: current");
    expect(result.out).not.toContain("inference-keys/index.ts");
  });

  test("removing the pack drops both shapes", () => {
    pi("deploy", "security");
    pi("remove", "security");
    const settings = JSON.parse(readFileSync(paths().settings, "utf8"));
    expect(settings.extensions.filter((p: string) => p.includes("inference-keys"))).toEqual([]);
  });
});

describe("removing a pack", () => {
  test("deletes every agent file it owns", () => {
    pi("deploy", "deliberation");
    const result = pi("remove", "deliberation");
    expect(result.code).toBe(0);
    expect(result.out).toContain(`${deliberationAgentCount()} agent file(s) removed`);
    expect(existsSync(join(paths().agents, "council-clerk.md"))).toBe(false);
    expect(existsSync(join(paths().agents, "red-team-lens.md"))).toBe(false);
  });

  test("leaves a file it does not own alone", () => {
    pi("deploy", "deliberation");
    writeFileSync(join(paths().agents, "hand-written.md"), "mine\n");
    pi("remove", "deliberation");
    expect(readFileSync(join(paths().agents, "hand-written.md"), "utf8")).toBe("mine\n");
  });

  test("refuses to delete a file that was edited at the destination", () => {
    pi("deploy", "deliberation");
    const clerk = join(paths().agents, "council-clerk.md");
    writeFileSync(clerk, "hand edited\n");
    const result = pi("remove", "deliberation");
    expect(result.code).not.toBe(0);
    expect(result.out).toContain("refusing to remove");
    expect(readFileSync(clerk, "utf8")).toBe("hand edited\n");
  });
});

describe("the vendored pi package channel", () => {
  /**
   * Every directory under Packs/harness/pi-packages, derived rather than listed: the set grows
   * whenever a package is vendored, and a test that has to be edited each time is testing the
   * list, not the wiring.
   */
  function vendoredPackages(): string[] {
    return readdirSync(join(SJEL_ROOT, "Packs", "harness", "pi-packages"), { withFileTypes: true })
      .filter((entry) => entry.isDirectory())
      .map((entry) => join(SJEL_ROOT, "Packs", "harness", "pi-packages", entry.name))
      .sort();
  }

  test("deploy harness registers every vendored package by path", () => {
    pi("deploy", "harness");
    const settings = JSON.parse(readFileSync(paths().settings, "utf8"));
    const registered = settings.packages.filter((p: string) => p.includes("Packs/harness/pi-packages")).sort();
    expect(registered).toEqual(vendoredPackages());
    expect(registered.length).toBeGreaterThanOrEqual(3);
  });

  test("every registered path exists and declares a pi manifest, so pi loads something from it", () => {
    pi("deploy", "harness");
    const settings = JSON.parse(readFileSync(paths().settings, "utf8"));
    const registered = settings.packages.filter((p: string) => p.includes("Packs/harness/pi-packages"));
    for (const path of registered) {
      const manifest = JSON.parse(readFileSync(join(path, "package.json"), "utf8"));
      expect(Array.isArray(manifest.pi?.extensions)).toBe(true);
      // The entry point must be one of this repo's files, not a remote URL: pi loads a path.
      for (const entry of manifest.pi.extensions) {
        expect(entry.startsWith("./")).toBe(true);
      }
    }
  });

  test("a non-path npm source in the same array is left alone", () => {
    writeFileSync(paths().settings, JSON.stringify({ packages: ["npm:pi-web-access"] }));
    pi("deploy", "harness");
    const settings = JSON.parse(readFileSync(paths().settings, "utf8"));
    expect(settings.packages).toContain("npm:pi-web-access");
  });

  test("a pack with no pi-packages directory registers none", () => {
    pi("deploy", "deliberation");
    const settings = JSON.parse(readFileSync(paths().settings, "utf8"));
    expect(settings.packages ?? []).toEqual([]);
  });

  test("remove drops the vendored path and keeps the npm source", () => {
    writeFileSync(paths().settings, JSON.stringify({ packages: ["npm:pi-web-access"] }));
    pi("deploy", "harness");
    pi("remove", "harness");
    const settings = JSON.parse(readFileSync(paths().settings, "utf8"));
    expect(settings.packages).toEqual(["npm:pi-web-access"]);
  });

  test("deploying twice does not duplicate the entries", () => {
    pi("deploy", "harness");
    const once = JSON.parse(readFileSync(paths().settings, "utf8")).packages;
    pi("deploy", "harness");
    const twice = JSON.parse(readFileSync(paths().settings, "utf8")).packages;
    expect(twice).toEqual(once);
    expect(twice.filter((p: string) => p.includes("pi-packages")).length).toBe(vendoredPackages().length);
  });
});

describe("a registry entry pointing at nothing", () => {
  // `deploy` only ever ADDED, so a skill retired from its Pack, or a whole Pack that
  // moved between roots, left its path in settings.json forever. On 2026-09-11 three of
  // twenty-seven entries were dead. These cases pin the prune AND its boundary, because
  // "remove anything that does not exist" would also delete an operator's own skills.
  const packRoot = join(SJEL_ROOT, "Packs");

  test("a dead path under a Pack root is pruned, live ones survive", () => {
    pi("deploy", "deliberation");
    const settings = paths().settings;
    const before = JSON.parse(readFileSync(settings, "utf8")).skills as string[];
    const retired = join(packRoot, "deliberation", "skills", "diverge-retired");
    writeFileSync(settings, JSON.stringify({ ...JSON.parse(readFileSync(settings, "utf8")), skills: [...before, retired] }));

    pi("deploy", "deliberation");
    const after = JSON.parse(readFileSync(settings, "utf8")).skills as string[];
    expect(after).not.toContain(retired);
    expect(after).toContain(join(packRoot, "deliberation", "skills", "council"));
  });

  test("a dead path OUTSIDE the Pack roots is left alone", () => {
    // The operator's own skill, kept somewhere else and temporarily moved. Ours to
    // ignore: pruning it would silently unregister work this deployment never owned.
    const stranger = join(root, "elsewhere", "my-own-skill");
    pi("deploy", "deliberation");
    const settings = paths().settings;
    const parsed = JSON.parse(readFileSync(settings, "utf8"));
    writeFileSync(settings, JSON.stringify({ ...parsed, skills: [...parsed.skills, stranger] }));

    pi("deploy", "deliberation");
    expect(JSON.parse(readFileSync(settings, "utf8")).skills).toContain(stranger);
  });

  test("a dead vendored package path is pruned", () => {
    pi("deploy", "harness");
    const settings = paths().settings;
    const parsed = JSON.parse(readFileSync(settings, "utf8"));
    const dead = join(packRoot, "harness", "pi-packages", "gone-package");
    writeFileSync(settings, JSON.stringify({ ...parsed, packages: [...parsed.packages, dead] }));

    pi("deploy", "harness");
    expect(JSON.parse(readFileSync(settings, "utf8")).packages).not.toContain(dead);
  });

  test("the settings ledger is trimmed too, so remove is not asked for a phantom", () => {
    pi("deploy", "deliberation");
    const ledger = JSON.parse(readFileSync(paths().state, "utf8"));
    const dead = join(packRoot, "deliberation", "skills", "diverge-retired");
    ledger.packs.deliberation = [...ledger.packs.deliberation, dead];
    writeFileSync(paths().state, JSON.stringify(ledger));

    pi("deploy", "deliberation");
    const after = JSON.parse(readFileSync(paths().state, "utf8"));
    expect(after.packs.deliberation).not.toContain(dead);
    expect(after.packs.deliberation.length).toBe(deliberationSkillCount());
  });
});

describe("two packs sharing the one flat agents root", () => {
  // The two Packs here used to be deliberation and academic-writing. academic-writing merged into
  // writing on 2026-09-17 and its agents/ directory came with it, so the pair is now
  // deliberation + writing — same property under test, different second Pack. The agent FILE
  // names are unchanged (they kept their academic-writing- prefix), which is why only the pack
  // argument moved.
  test("each deploys without colliding, and each removal is scoped", () => {
    pi("deploy", "deliberation");
    pi("deploy", "writing");
    expect(existsSync(join(paths().agents, "council-clerk.md"))).toBe(true);
    expect(existsSync(join(paths().agents, "academic-writing-logic-reviewer.md"))).toBe(true);

    pi("remove", "deliberation");
    expect(existsSync(join(paths().agents, "council-clerk.md"))).toBe(false);
    expect(existsSync(join(paths().agents, "academic-writing-logic-reviewer.md"))).toBe(true);
  });

  test("the bibliography auditor keeps its shell, because its own file declares it", () => {
    pi("deploy", "writing");
    const auditor = readFileSync(join(paths().agents, "academic-writing-bibliography-auditor.md"), "utf8");
    expect(auditor).toContain("tools: read, grep, find, bash");
  });
});
