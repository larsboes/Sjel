import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import {
  deployPack,
  getStatuses,
  migrateGeneratedArtifacts,
  removePack,
  resolveProfilePacks,
  syncPack,
  validateCodexFiles,
  type DeployConfig,
} from "./packs-codex.ts";

let root: string;
let config: DeployConfig;

function write(path: string, content: string, mode?: number): void {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
  if (mode !== undefined) chmodSync(path, mode);
}

function makePack(): void {
  const pack = join(root, "Axon", "Packs", "demo");
  mkdirSync(join(pack, "skills", "demo-skill", "scripts"), { recursive: true });
  writeFileSync(
    join(pack, "pack.toml"),
    'name = "demo"\ndescription = "fixture"\nskills = ["demo-skill"]\nlicense = "MIT"\n',
  );
  writeFileSync(
    join(pack, "skills", "demo-skill", "SKILL.md"),
    "---\nname: demo-skill\ndescription: Shared description.\nallowed-tools: Bash\n---\n\n# Demo\n",
  );
  write(join(pack, "skills", "demo-skill", "scripts", "run"), "#!/bin/sh\necho shared\n", 0o755);
  write(
    join(pack, "codex", "demo-skill", "agents", "openai.yaml"),
    'interface:\n  display_name: "Demo Skill"\n  short_description: "Codex metadata overlay"\n',
  );
}

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), "axon-packs-codex-test-"));
  // `adapter` became required when the engine moved to lib/pack-deploy.ts: the
  // shared core may not carry a default harness name, so every caller states its
  // own. Only this fixture changed — every assertion below is the one that
  // guarded the pre-extraction behaviour.
  config = {
    axonRoot: join(root, "Axon"),
    destination: join(root, "home", ".agents", "skills"),
    stateFile: join(root, "state", "codex.json"),
    adapter: "codex",
    stateEnvVar: "SJEL_CODEX_STATE_FILE",
    validateAdapterFiles: validateCodexFiles,
  };
  makePack();
});

afterEach(() => rmSync(root, { recursive: true, force: true }));

describe("materialized Codex Pack deployment", () => {
  test("deploy copies the complete skill and merges Codex metadata without symlinks", () => {
    expect(deployPack(config, "demo")).toEqual(["✓ demo-skill deployed"]);
    const installed = join(config.destination, "demo-skill");
    expect(readFileSync(join(installed, "SKILL.md"), "utf8")).toContain("Shared description.");
    expect(readFileSync(join(installed, "agents", "openai.yaml"), "utf8")).toContain("Demo Skill");
    expect(readFileSync(join(installed, "scripts", "run"), "utf8")).toContain("echo shared");
    expect(Bun.file(join(installed, "scripts", "run")).stat().then((stat) => stat.mode & 0o111)).resolves.toBe(0o111);
    expect(readdirSync(config.destination).some((name) => name.startsWith(".axon-"))).toBe(false);
    expect(getStatuses(config, "demo")[0].status).toBe("current");
  });

  test("sync updates a clean deployment and refuses destination-side drift", () => {
    deployPack(config, "demo");
    const source = join(config.axonRoot, "Packs", "demo", "skills", "demo-skill", "SKILL.md");
    writeFileSync(source, readFileSync(source, "utf8").replace("# Demo", "# Updated"));
    const metadata = join(config.axonRoot, "Packs", "demo", "codex", "demo-skill", "agents", "openai.yaml");
    writeFileSync(metadata, readFileSync(metadata, "utf8").replace("Demo Skill", "Updated Demo"));
    expect(getStatuses(config, "demo")[0].status).toBe("outdated");
    expect(syncPack(config, "demo")).toEqual(["✓ demo-skill synced"]);
    expect(readFileSync(join(config.destination, "demo-skill", "SKILL.md"), "utf8")).toContain("# Updated");
    expect(readFileSync(join(config.destination, "demo-skill", "agents", "openai.yaml"), "utf8")).toContain(
      "Updated Demo",
    );

    writeFileSync(join(config.destination, "demo-skill", "SKILL.md"), "local edit\n");
    expect(getStatuses(config, "demo")[0].status).toBe("drifted");
    expect(() => syncPack(config, "demo")).toThrow("local changes");
  });

  test("generated Python artifacts are neither deployed nor hashed as drift", () => {
    const source = join(config.axonRoot, "Packs", "demo", "skills", "demo-skill");
    write(join(source, "scripts", "__pycache__", "helper.cpython-314.pyc"), "compiled source\n");
    write(join(source, "scripts", "helper.pyo"), "optimized source\n");
    deployPack(config, "demo");

    const installed = join(config.destination, "demo-skill");
    expect(existsSync(join(installed, "scripts", "__pycache__"))).toBe(false);
    expect(existsSync(join(installed, "scripts", "helper.pyo"))).toBe(false);

    write(join(installed, "scripts", "__pycache__", "helper.cpython-314.pyc"), "runtime cache\n");
    expect(getStatuses(config, "demo")[0].status).toBe("current");

    const skillMd = join(source, "SKILL.md");
    writeFileSync(skillMd, readFileSync(skillMd, "utf8").replace("# Demo", "# Updated"));
    expect(getStatuses(config, "demo")[0].status).toBe("outdated");
    expect(syncPack(config, "demo")).toEqual(["✓ demo-skill synced"]);
    expect(existsSync(join(installed, "scripts", "__pycache__"))).toBe(false);

    writeFileSync(join(installed, "SKILL.md"), "genuine local edit\n");
    expect(getStatuses(config, "demo")[0].status).toBe("drifted");
  });

  test("legacy ambiguous digests require an explicit bounded migration", () => {
    deployPack(config, "demo");
    const state = JSON.parse(readFileSync(config.stateFile, "utf8"));
    delete state.packs.demo.skills["demo-skill"].digestPolicy;
    writeFileSync(config.stateFile, `${JSON.stringify(state, null, 2)}\n`);

    const installed = join(config.destination, "demo-skill");
    write(join(installed, "scripts", "__pycache__", "helper.cpython-314.pyc"), "runtime cache\n");
    const sourceSkill = join(config.axonRoot, "Packs", "demo", "skills", "demo-skill", "SKILL.md");
    writeFileSync(sourceSkill, readFileSync(sourceSkill, "utf8").replace("# Demo", "# Updated"));

    expect(getStatuses(config, "demo")[0].status).toBe("migration-required");
    expect(() => migrateGeneratedArtifacts(config, "demo", false)).toThrow("--accept-current");
    expect(migrateGeneratedArtifacts(config, "demo", true)).toEqual([
      "✓ demo-skill migrated (1 generated artifact(s) removed)",
    ]);
    expect(existsSync(join(installed, "scripts", "__pycache__"))).toBe(false);
    expect(getStatuses(config, "demo")[0].status).toBe("outdated");
  });

  test("generated-artifact migration refuses unknown cache-directory content", () => {
    deployPack(config, "demo");
    const state = JSON.parse(readFileSync(config.stateFile, "utf8"));
    delete state.packs.demo.skills["demo-skill"].digestPolicy;
    writeFileSync(config.stateFile, `${JSON.stringify(state, null, 2)}\n`);

    const unknown = join(config.destination, "demo-skill", "scripts", "__pycache__", "notes.txt");
    write(unknown, "keep me\n");
    expect(() => migrateGeneratedArtifacts(config, "demo", true)).toThrow("unknown content");
    expect(readFileSync(unknown, "utf8")).toBe("keep me\n");
  });

  test("sync removes an owned retired skill when a Pack renames it", () => {
    deployPack(config, "demo");
    const pack = join(config.axonRoot, "Packs", "demo");
    renameSync(join(pack, "skills", "demo-skill"), join(pack, "skills", "renamed-skill"));
    const skill = join(pack, "skills", "renamed-skill", "SKILL.md");
    writeFileSync(skill, readFileSync(skill, "utf8").replace("name: demo-skill", "name: renamed-skill"));
    writeFileSync(
      join(pack, "pack.toml"),
      'name = "demo"\ndescription = "fixture"\nskills = ["renamed-skill"]\nretired_skills = ["demo-skill"]\nlicense = "MIT"\n',
    );

    expect(getStatuses(config, "demo")).toContainEqual({
      pack: "demo",
      skill: "demo-skill",
      status: "outdated",
      detail: "removed from pack manifest",
    });
    expect(syncPack(config, "demo")).toEqual(["✓ demo-skill removed", "✓ renamed-skill synced"]);
    expect(existsSync(join(config.destination, "demo-skill"))).toBe(false);
    expect(existsSync(join(config.destination, "renamed-skill", "SKILL.md"))).toBe(true);
  });

  test("deploy refuses an unowned destination collision", () => {
    mkdirSync(join(config.destination, "demo-skill"), { recursive: true });
    writeFileSync(join(config.destination, "demo-skill", "SKILL.md"), "unrelated\n");
    expect(() => deployPack(config, "demo")).toThrow("not owned");
    expect(readFileSync(join(config.destination, "demo-skill", "SKILL.md"), "utf8")).toBe("unrelated\n");
  });

  test("remove deletes only a clean owned deployment", () => {
    deployPack(config, "demo");
    expect(removePack(config, "demo")).toEqual(["✓ demo-skill removed"]);
    expect(existsSync(join(config.destination, "demo-skill"))).toBe(false);
  });

  test("source symlinks are rejected instead of leaking into the deployment", () => {
    const skill = join(config.axonRoot, "Packs", "demo", "skills", "demo-skill");
    symlinkSync("SKILL.md", join(skill, "linked-skill.md"));
    expect(() => deployPack(config, "demo")).toThrow("must be materialized");
    expect(existsSync(join(config.destination, "demo-skill"))).toBe(false);
  });

  test("a Codex overlay cannot replace canonical SKILL.md", () => {
    write(
      join(config.axonRoot, "Packs", "demo", "codex", "demo-skill", "SKILL.md"),
      "---\nname: demo-skill\ndescription: Override.\n---\n",
    );
    expect(() => deployPack(config, "demo")).toThrow("may not override canonical SKILL.md");
  });

  // Ownership: a pack naming its own deployer leaves this sweep, or the same
  // destination gets claimed by two mechanisms reporting different verdicts.
  describe("a pack that names its own deployer", () => {
    const manifest = () => join(config.axonRoot, "Packs", "demo", "pack.toml");
    const claim = () =>
      writeFileSync(
        manifest(),
        'name = "demo"\ndescription = "fixture"\nskills = ["demo-skill"]\nlicense = "MIT"\ndeployer = "packs-demo"\n',
      );

    test("drops out of the unselected sweep", () => {
      expect(getStatuses(config).some((row) => row.pack === "demo")).toBe(true);
      claim();
      expect(getStatuses(config).some((row) => row.pack === "demo")).toBe(false);
    });

    test("stays reachable when its deployer asks for it by name", () => {
      claim();
      const rows = getStatuses(config, "demo");
      expect(rows.map((row) => row.pack)).toEqual(["demo"]);
      expect(deployPack(config, "demo")).toEqual(["✓ demo-skill deployed"]);
      expect(getStatuses(config, "demo")[0]).toMatchObject({ status: "current" });
    });

    test("names its owner when a profile claims it, rather than 'unknown pack'", () => {
      claim();
      expect(() => resolveProfilePacks(config, { name: "everything", packs: ["demo"] })).toThrow(
        "deployed by packs-demo",
      );
    });

    test("drops out of a wildcard profile", () => {
      claim();
      expect(resolveProfilePacks(config, { name: "all", packs: ["*"] })).not.toContain("demo");
    });
  });
});
