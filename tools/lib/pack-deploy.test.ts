// tools/lib/pack-deploy.test.ts — the two behaviours the shared engine grew when
// the Claude adapter stopped using symlinks on 2026-08-09: the whole-directory
// tree convention, and adoption of copies that already sit at the destination.
//
// The pre-existing engine behaviour is covered by tools/packs-codex.test.ts,
// which was deliberately left pointing at packs-codex.ts so the extraction had a
// check it could not quietly pass.
//
// Run: bun test tools/lib/pack-deploy.test.ts

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import {
  adoptPack,
  deployPack,
  reconcileUnit,
  resolveProfilePacks,
  syncPack,
  withStateLock,
  packUnits as unitsOf,
  getStatuses,
  packUnits,
  readState,
  removePack,
  treeKey,
  type DeployConfig,
} from "./pack-deploy.ts";

let root: string;
let config: DeployConfig;

function writeSkill(pack: string, skill: string, body = "shared instructions"): void {
  const dir = join(root, "Axon", "Packs", pack, "skills", skill);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "SKILL.md"), `---\nname: ${skill}\ndescription: does a thing\n---\n\n${body}\n`);
}

function writeAgents(pack: string, ...names: string[]): void {
  const dir = join(root, "Axon", "Packs", pack, "agents");
  mkdirSync(dir, { recursive: true });
  for (const name of names) writeFileSync(join(dir, `${name}.md`), `# ${name}\n`);
}

function writeManifest(pack: string, skills: string[]): void {
  const dir = join(root, "Axon", "Packs", pack);
  mkdirSync(dir, { recursive: true });
  writeFileSync(
    join(dir, "pack.toml"),
    `name = "${pack}"\ndescription = "test pack"\nskills = [${skills.map((s) => `"${s}"`).join(", ")}]\n`,
  );
}

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), "axon-pack-deploy-test-"));
  config = {
    axonRoot: join(root, "Axon"),
    destination: join(root, "home", ".claude", "skills"),
    stateFile: join(root, "state", "claude.json"),
    adapter: "claude",
    stateEnvVar: "SJEL_CLAUDE_STATE_FILE",
    treeConvention: {
      sourceDir: "agents",
      destinationRoot: join(root, "home", ".claude", "agents"),
    },
  };
  writeManifest("demo", ["demo-skill"]);
  writeSkill("demo", "demo-skill");
});

afterEach(() => rmSync(root, { recursive: true, force: true }));

describe("the tree convention", () => {
  test("a pack without the directory yields skills only", () => {
    expect(packUnits(config, "demo").map((u) => u.key)).toEqual(["demo-skill"]);
  });

  test("a pack carrying it gains one unit, not one per file", () => {
    writeAgents("demo", "reviewer", "skeptic", "auditor");
    const units = packUnits(config, "demo");
    expect(units.map((u) => u.key)).toEqual(["demo-skill", "agents/"]);
    expect(units[1].isSkill).toBe(false);
  });

  test("an adapter that declares no convention never sees the directory", () => {
    writeAgents("demo", "reviewer");
    const codexish: DeployConfig = { ...config, adapter: "codex", treeConvention: undefined };
    expect(packUnits(codexish, "demo").map((u) => u.key)).toEqual(["demo-skill"]);
  });

  test("the tree deploys to its own root, under the pack name", () => {
    writeAgents("demo", "reviewer");
    deployPack(config, "demo");
    expect(readFileSync(join(config.treeConvention!.destinationRoot, "demo", "reviewer.md"), "utf8")).toContain("reviewer");
  });

  test("its ledger key cannot collide with a skill name", () => {
    // Skill names are lowercase-hyphen-case; the trailing slash makes the tree key
    // unrepresentable as one, so the impossibility is structural.
    expect(treeKey("agents")).toBe("agents/");
    expect(/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(treeKey("agents"))).toBe(false);
  });

  test("remove takes the tree with it", () => {
    writeAgents("demo", "reviewer");
    deployPack(config, "demo");
    removePack(config, "demo");
    expect(getStatuses(config, "demo").map((r) => r.status)).toEqual(["not-deployed", "not-deployed"]);
  });
});

describe("adoptPack", () => {
  function placeIdenticalCopy(): void {
    mkdirSync(join(config.destination, "demo-skill"), { recursive: true });
    writeFileSync(
      join(config.destination, "demo-skill", "SKILL.md"),
      readFileSync(join(config.axonRoot, "Packs", "demo", "skills", "demo-skill", "SKILL.md")),
    );
  }

  test("an unowned copy is a collision until adopted", () => {
    placeIdenticalCopy();
    expect(getStatuses(config, "demo")[0].status).toBe("collision");
    adoptPack(config, "demo");
    expect(getStatuses(config, "demo")[0].status).toBe("current");
  });

  test("adoption writes the ledger without touching the destination", () => {
    placeIdenticalCopy();
    const before = readFileSync(join(config.destination, "demo-skill", "SKILL.md"), "utf8");
    adoptPack(config, "demo");
    expect(readFileSync(join(config.destination, "demo-skill", "SKILL.md"), "utf8")).toBe(before);
    expect(readState(config).packs.demo.skills["demo-skill"]).toBeDefined();
  });

  test("a destination that differs is refused, never claimed", () => {
    // The whole safety argument for adoption is the digest match. A copy that
    // differs is a hand edit or a stale deploy, and recording it would assert
    // something untrue about what is on disk.
    mkdirSync(join(config.destination, "demo-skill"), { recursive: true });
    writeFileSync(
      join(config.destination, "demo-skill", "SKILL.md"),
      `---\nname: demo-skill\ndescription: does a thing\n---\n\nEDITED BY HAND\n`,
    );
    expect(() => adoptPack(config, "demo")).toThrow("refusing to adopt");
    expect(readState(config).packs.demo).toBeUndefined();
  });

  test("nothing at the destination adopts nothing rather than failing", () => {
    expect(adoptPack(config, "demo")).toEqual(["= demo-skill (not deployed; nothing to adopt)"]);
  });

  test("re-adopting is a no-op, so it is safe to run twice", () => {
    placeIdenticalCopy();
    adoptPack(config, "demo");
    expect(adoptPack(config, "demo")).toEqual(["= demo-skill (already owned)"]);
  });

  test("a unit another pack owns is refused", () => {
    placeIdenticalCopy();
    adoptPack(config, "demo");
    writeManifest("rival", ["demo-skill"]);
    writeSkill("rival", "demo-skill");
    expect(() => adoptPack(config, "rival")).toThrow("already owned by Pack 'demo'");
  });

  test("the tree unit adopts on the same terms", () => {
    writeAgents("demo", "reviewer");
    placeIdenticalCopy();
    const treeDest = join(config.treeConvention!.destinationRoot, "demo");
    mkdirSync(treeDest, { recursive: true });
    writeFileSync(join(treeDest, "reviewer.md"), "# reviewer\n");
    expect(adoptPack(config, "demo")).toEqual(["✓ demo-skill adopted", "✓ agents/ adopted"]);
  });
});

describe("ownership is a claim on a destination", () => {
  // Two Packs each carrying agents/ share the ledger key `agents/` and nothing
  // else: their destinations differ by pack name. A key-based owner lookup read
  // that as a collision, so the SECOND pack to ship subagents could never be
  // deployed — the failure this repository actually hit on 2026-09-07, with
  // academic-writing installed and deliberation refused.
  test("two packs may each carry an agents/ tree", () => {
    writeAgents("demo", "reviewer");
    writeManifest("second", ["second-skill"]);
    writeSkill("second", "second-skill");
    writeAgents("second", "clerk");

    deployPack(config, "demo");
    expect(deployPack(config, "second")).toEqual(["✓ second-skill deployed", "✓ agents/ deployed"]);

    const treeRoot = config.treeConvention!.destinationRoot;
    expect(readFileSync(join(treeRoot, "demo", "reviewer.md"), "utf8")).toBe("# reviewer\n");
    expect(readFileSync(join(treeRoot, "second", "clerk.md"), "utf8")).toBe("# clerk\n");
    expect(Object.keys(readState(config).packs).sort()).toEqual(["demo", "second"]);
  });

  test("a second pack claiming the same skill destination is still refused", () => {
    deployPack(config, "demo");
    writeManifest("rival", ["demo-skill"]);
    writeSkill("rival", "demo-skill", "a different body");
    expect(() => deployPack(config, "rival")).toThrow("already owned by Pack 'demo'");
  });
});

describe("reconcileUnit", () => {
  // The accept path: an edit made to a deployed copy is worth keeping, the source
  // has just been updated FROM the destination, and the ledger still reports a
  // drift that no longer exists. sync refuses in that state and deploy would
  // overwrite the very edit being kept, so re-recording needs its own verb.
  function trimmedUnit() {
    return unitsOf(config, "demo").find((u) => u.key === "demo-skill")!;
  }

  test("re-records when the destination matches the source again", () => {
    deployPack(config, "demo");
    const unit = trimmedUnit();
    const edited = "---\nname: demo-skill\ndescription: does a thing\n---\n\nedited at the destination\n";
    writeFileSync(join(unit.destination, "SKILL.md"), edited);
    expect(getStatuses(config, "demo")[0].status).toBe("drifted");

    // What `accept` does: copy the destination back over the source.
    writeFileSync(join(root, "Axon", "Packs", "demo", "skills", "demo-skill", "SKILL.md"), edited);

    expect(reconcileUnit(config, "demo", unit)).toBe("✓ demo-skill re-recorded");
    expect(getStatuses(config, "demo")[0].status).toBe("current");
  });

  test("refuses while the destination still differs", () => {
    deployPack(config, "demo");
    const unit = trimmedUnit();
    writeFileSync(join(unit.destination, "SKILL.md"), "only at the destination\n");
    expect(() => reconcileUnit(config, "demo", unit)).toThrow("refusing to re-record");
  });

  test("refuses an accepted edit that broke the skill", () => {
    deployPack(config, "demo");
    const unit = trimmedUnit();
    const broken = "no frontmatter at all\n";
    writeFileSync(join(unit.destination, "SKILL.md"), broken);
    writeFileSync(join(root, "Axon", "Packs", "demo", "skills", "demo-skill", "SKILL.md"), broken);
    expect(() => reconcileUnit(config, "demo", unit)).toThrow();
  });

  test("refuses a unit this Pack does not own", () => {
    const unit = trimmedUnit();
    expect(() => reconcileUnit(config, "demo", unit)).toThrow("not owned by Pack 'demo'");
  });
});

describe("the ledger lock", () => {
  // writeState is atomic, so no reader sees half a file — that was never the race.
  // Every mutator reads the whole ledger once and writes it back one or more times,
  // so two overlapping processes each hold a pre-other snapshot and the last writer
  // erases the other's rows. The skill stays on disk with no ledger entry: an
  // unowned collision that only a hand-run adopt can repair.
  function lockPath(): string {
    return `${config.stateFile}.lock`;
  }

  test("the lock is released after a successful mutation", () => {
    deployPack(config, "demo");
    expect(existsSync(lockPath())).toBe(false);
  });

  test("the lock is released after a failed mutation", () => {
    expect(() => syncPack(config, "demo")).toThrow("not deployed");
    expect(existsSync(lockPath())).toBe(false);
  });

  test("a live holder is refused rather than overwritten", () => {
    mkdirSync(dirname(config.stateFile), { recursive: true });
    // pid 1 is alive on every POSIX host and is not us; kill(1, 0) answers EPERM,
    // which the holder check reads as alive on purpose.
    writeFileSync(lockPath(), JSON.stringify({ pid: 1, at: new Date().toISOString() }));
    process.env.SJEL_PACK_LOCK_WAIT_MS = "10";
    try {
      expect(() => deployPack(config, "demo")).toThrow("ledger is locked by pid 1");
    } finally {
      delete process.env.SJEL_PACK_LOCK_WAIT_MS;
      rmSync(lockPath(), { force: true });
    }
  });

  test("a lock whose holder is gone is stolen, not waited on", () => {
    mkdirSync(dirname(config.stateFile), { recursive: true });
    // A pid that cannot exist: the holder crashed and left the file behind. Without
    // the steal, one killed process would break the tool until somebody found the
    // lock file by hand.
    writeFileSync(lockPath(), JSON.stringify({ pid: 2147483646, at: new Date().toISOString() }));
    expect(deployPack(config, "demo")).toContain("✓ demo-skill deployed");
    expect(existsSync(lockPath())).toBe(false);
  });

  test("it is re-entrant, because activateProfile calls the mutators it wraps", () => {
    let inner = "";
    const outer = withStateLock(config, () => {
      inner = withStateLock(config, () => "reached");
      return "done";
    });
    expect([outer, inner]).toEqual(["done", "reached"]);
    expect(existsSync(lockPath())).toBe(false);
  });
});

describe("a profile's except list", () => {
  // "What to deploy, and what not" is the operator's decision, and this is where it is
  // written. It replaced a `# pi: REFUSED` line a Pack carried in its own pack.toml
  // (retired 2026-09-17), which put a harness-specific veto in a manifest the schema keeps
  // harness-neutral — and made `full` mean something other than "all packs".
  beforeEach(() => {
    writeManifest("dormant", ["dormant-skill"]);
    writeSkill("dormant", "dormant-skill");
  });

  test("removes the named Pack from a wildcard profile", () => {
    const packs = resolveProfilePacks(config, { name: "full", packs: ["*"], except: ["dormant"] });
    expect(packs).not.toContain("dormant");
    // And the rest of the sweep is untouched, so the exclusion is not a filter over all.
    expect(packs).toContain("demo");
  });

  test("a wildcard profile with no except still takes everything", () => {
    expect(resolveProfilePacks(config, { name: "everything", packs: ["*"] })).toContain("dormant");
  });

  test("a name that matches no Pack throws, rather than silently excluding nothing", () => {
    // The typo case is the one that matters: an exclusion that does nothing looks exactly
    // like a Pack that was deployed on purpose.
    expect(() =>
      resolveProfilePacks(config, { name: "full", packs: ["*"], except: ["dormat"] }),
    ).toThrow("except names unknown pack 'dormat'");
  });

  test("except on a list that is not a wildcard is refused as meaningless", () => {
    expect(() =>
      resolveProfilePacks(config, { name: "x", packs: ["demo"], except: ["dormant"] }),
    ).toThrow("except is only meaningful with packs = [\"*\"]");
  });
});
