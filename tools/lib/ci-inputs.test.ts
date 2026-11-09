import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { GATES, verificationInputs } from "./ci-inputs.ts";

let root: string;

function git(...args: string[]) {
  const result = Bun.spawnSync(["git", ...args], { cwd: root });
  if (result.exitCode) throw new Error(result.stderr.toString());
}

function plant(file: string, content: string, tracked = true) {
  const path = join(root, file);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
  if (tracked) git("add", "--", file);
}

function pkg(dir: string, extras: Record<string, unknown> = {}) {
  plant(`${dir}/package.json`, JSON.stringify({ name: dir.replaceAll("/", "-"), scripts: { check: "tsc --noEmit" }, ...extras }));
  plant(`${dir}/bun.lock`, "{}");
  plant(`${dir}/src/main.ts`, "export const value = 1;");
}

function select(files: string[], deleted: string[] = [], full = false) {
  return verificationInputs(root, files, deleted, full);
}

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), "ci-inputs-"));
  git("init", "-q");
});

afterEach(() => rmSync(root, { recursive: true, force: true }));

describe("Bun reverse dependency selection", () => {
  test("follows transitive imports, re-exports and test-to-test imports", () => {
    plant("src/value.ts", "export const value = 1;");
    plant("src/bridge.ts", "export { value } from './value';");
    plant("tests/first.test.ts", "import { value } from '../src/bridge.ts';");
    plant("tests/second.test.ts", "import './first.test.ts';");
    plant("elsewhere/unrelated.test.ts", "export const unrelated = 1;");
    expect(select(["src/value.ts"]).bun).toEqual(["tests/first.test.ts", "tests/second.test.ts"]);
  });

  test("shared test helpers select every reader, not only adjacent tests", () => {
    plant("tests/helper.ts", "export const helper = 1;");
    plant("a/a.test.ts", "import { helper } from '../tests/helper';");
    plant("b/b.spec.ts", "import { helper } from '../tests/helper';");
    plant("tests/adjacent.test.ts", "export const unrelated = 1;");
    expect(select(["tests/helper.ts"]).bun).toEqual(["a/a.test.ts", "b/b.spec.ts"]);
  });

  test("resolves JS-extension TS imports, directory indexes, require and static dynamic imports", () => {
    plant("shared/index.ts", "export const x = 1;");
    plant("source/value.ts", "export const x = 1;");
    plant("a.test.ts", "import { x } from './source/value.js';");
    plant("b.test.js", "const x = require('./shared');");
    plant("c.test.mts", "await import('./source/value.ts');");
    expect(select(["source/value.ts"]).bun).toEqual(["a.test.ts", "c.test.mts"]);
    expect(select(["shared/index.ts"]).bun).toEqual(["b.test.js"]);
  });

  test("new tests anywhere, including untracked local tests, need no list edit", () => {
    plant("new/deep/fresh.spec.tsx", "export const x = 1;");
    plant("new/deep/local_test.js", "export const x = 1;", false);
    expect(select(["new/deep/fresh.spec.tsx", "new/deep/local_test.js"]).bun).toEqual([
      "new/deep/fresh.spec.tsx", "new/deep/local_test.js",
    ]);
  });

  test("Bun discovery ignores and preload inputs are respected", () => {
    plant("bunfig.toml", '[test]\npathIgnorePatterns = ["vendor/**"]\npreload = ["./setup.ts"]\n');
    plant("setup.ts", "export const ready = true;");
    plant("a.test.ts", "export const x = 1;");
    plant("nested/b.test.ts", "export const x = 1;");
    plant("vendor/not-bun.test.ts", "export const x = 1;");
    expect(select(["setup.ts"]).bun).toEqual(["a.test.ts", "nested/b.test.ts"]);
    expect(select([], [], true).bun).toEqual(["a.test.ts", "nested/b.test.ts"]);
  });

  test("preload dependencies reach all tests", () => {
    plant("bunfig.toml", '[test]\npreload = ["./setup.ts"]\n');
    plant("setup.ts", "import './helper.ts';");
    plant("helper.ts", "export const x = 1;");
    plant("a.test.ts", "export const x = 1;");
    plant("b.test.ts", "export const x = 1;");
    expect(select(["helper.ts"]).bun).toEqual(["a.test.ts", "b.test.ts"]);
  });

  test("missing preload or invalid Bun config cannot produce an empty passing plan", () => {
    plant("bunfig.toml", '[test]\npreload = ["./missing.ts"]\n');
    expect(() => select([])).toThrow("Unresolved Bun preload");
    plant("bunfig.toml", "[test\n");
    expect(() => select([])).toThrow();
  });

  test("Git-ignored generated output and installed dependencies do not become test suites", () => {
    plant(".gitignore", "/target/\n/.claude/worktrees/\n");
    plant("ok.test.ts", "export const x = 1;");
    plant("node_modules/lib/x.test.ts", "export const x = 1;");
    plant("target/temporary.test.ts", "export const x = 1;", false);
    plant(".claude/worktrees/else/x.test.ts", "export const x = 1;", false);
    expect(select([], [], true).bun).toEqual(["ok.test.ts"]);
  });

  test("tracked build, dist and target directories retain tests and reverse imports", () => {
    plant("src/value.ts", "export const value = 1;");
    const tests = ["src/build/probe.test.ts", "src/dist/probe.test.ts", "src/target/probe.test.ts", "src/.svelte-kit/probe.test.ts"];
    for (const path of tests) plant(path, 'import { value } from "../value.ts";');
    expect(select([], [], true).bun).toEqual([...tests].sort());
    expect(select(["src/value.ts"]).bun).toEqual([...tests].sort());
  });
});

describe("runtime and missing inputs", () => {
  test("a fixture outside source ownership reaches readers", () => {
    plant("fixtures/shared.json", "{}");
    plant("tests/read.test.ts", 'import { readFileSync } from "node:fs"; readFileSync("fixtures/shared.json");');
    plant("other.test.ts", "export const x = 1;");
    const plan = select(["fixtures/shared.json"]);
    expect(plan.bun).toContain("tests/read.test.ts");
    expect(plan.reasons.join("\n")).toContain("runtime file reads");
  });

  test("path literals select prose readers without source ownership", () => {
    plant("docs/reference.md", "reference");
    plant("tests/reference.test.ts", 'export const reference = "docs/reference.md";');
    plant("other.test.ts", "export const x = 1;");
    expect(select(["docs/reference.md"]).bun).toEqual(["tests/reference.test.ts"]);
  });

  test("computed file reads widen beyond the containing test directory", () => {
    plant("tests/read.test.ts", 'import { readFileSync } from "node:fs"; readFileSync(process.env.INPUT!);');
    plant("elsewhere/other.test.ts", "export const x = 1;");
    plant("src/one.ts", "export const x = 1;");
    expect(select(["src/one.ts"]).bun).toEqual(["elsewhere/other.test.ts", "tests/read.test.ts"]);
  });

  test("computed imports and subprocess execution widen even for a known imported source", () => {
    plant("src/value.ts", "export const x = 1;");
    plant("known.test.ts", "import './src/value.ts';");
    plant("dynamic.test.ts", "await import(process.env.MODULE!);");
    plant("other.test.ts", "export const x = 1;");
    expect(select(["src/value.ts"]).bun).toEqual(["dynamic.test.ts", "known.test.ts", "other.test.ts"]);
    plant("dynamic.test.ts", 'import { execSync as run } from "node:child_process"; run("true");');
    expect(select(["src/value.ts"]).reasons.join("\n")).toContain("subprocess");
  });

  test("literal-prefixed computed imports still widen", () => {
    plant("src/value.ts", "export const x = 1;");
    plant("known.test.ts", "import './src/value.ts';");
    plant("dynamic.test.ts", 'const name = process.env.MODULE; await import("./" + name);');
    plant("other.test.ts", "export const x = 1;");
    const plan = select(["src/value.ts"]);
    expect(plan.bun).toEqual(["dynamic.test.ts", "known.test.ts", "other.test.ts"]);
    expect(plan.reasons.join("\n")).toContain("computed import");
  });

  test("a root package cannot hide unknown executable inputs", () => {
    plant("package.json", '{"name":"root-package"}');
    plant("a.test.ts", "export const x = 1;");
    expect(select(["unknown-executable"]).bun).toEqual(["a.test.ts"]);
  });

  test("unresolved relative imports cannot silently remove their tests", () => {
    plant("a.test.ts", "import './not-present.ts';");
    plant("b.test.ts", "export const x = 1;");
    plant("src/x.ts", "export const x = 1;");
    const plan = select(["src/x.ts"]);
    expect(plan.bun).toEqual(["a.test.ts", "b.test.ts"]);
    expect(plan.reasons.join("\n")).toContain("unresolved import");
  });

  test("deletions, including rename source sides, widen all affected suites", () => {
    plant("src/value.ts", "export const x = 1;");
    plant("a.test.ts", "import './src/value.ts';");
    plant("b.test.ts", "export const x = 1;");
    plant("tools/run.test.sh", "true\n");
    pkg("ui");
    unlinkSync(join(root, "src/value.ts"));
    const plan = select(["src/replacement.ts"], ["src/value.ts"]);
    expect(plan.bun).toEqual(["a.test.ts", "b.test.ts"]);
    expect(plan.shell).toEqual(["tools/run.test.sh"]);
    expect(plan.ui).toEqual(["ui"]);
    expect(plan.reasons.join("\n")).toContain("deleted or renamed");
  });

  test("unknown non-prose and executable inputs widen rather than use adjacency", () => {
    plant("a.test.ts", "export const x = 1;");
    plant("tools/run.test.sh", "true\n");
    pkg("ui");
    plant("fixtures/input.bin", "value");
    expect(select(["fixtures/input.bin"]).bun).toEqual(["a.test.ts"]);
    expect(select(["new-executable"]).shell).toEqual(["tools/run.test.sh"]);
    expect(select(["new-executable"]).ui).toEqual(["ui"]);
  });

  test("known inert prose saves test work even with unrelated runtime readers", () => {
    plant("docs/plans/example.md", "a plan");
    plant("a.test.ts", 'import { readFileSync } from "node:fs"; readFileSync(process.env.INPUT!);');
    plant("tools/run.test.sh", 'source "$UNKNOWN"\n');
    pkg("ui");
    const plan = select(["docs/plans/example.md"]);
    expect(plan.bun).toEqual([]);
    expect(plan.shell).toEqual([]);
    expect(plan.ui).toEqual([]);
    expect(plan.pages).toBe(false);
    expect(plan.security).toEqual({ deps: false, images: false, codeql: [] });
    expect(plan.gates).toContain("publication-hygiene");
  });

  test("text fixtures do not inherit the prose exemption", () => {
    plant("fixtures/expected.txt", "expected");
    plant("a.test.ts", 'import { readFileSync } from "node:fs"; readFileSync(process.env.INPUT!);');
    expect(select(["fixtures/expected.txt"]).bun).toEqual(["a.test.ts"]);
  });
});

describe("shell suite", () => {
  test("literal runtime refs reach shell readers", () => {
    plant("tools/worker", "true\n");
    plant("tools/first.test.sh", '"tools/worker"\n');
    plant("tools/second.test.sh", "true\n");
    expect(select(["tools/worker"]).shell).toEqual(["tools/first.test.sh"]);
  });

  test("shell sources and shared tools/lib changes widen conservatively", () => {
    plant("tools/lib/shared.sh", "true\n");
    plant("tools/a.test.sh", 'source "$ROOT/tools/lib/shared.sh"\n');
    plant("tools/b.test.sh", "true\n");
    expect(select(["tools/lib/shared.sh"]).shell).toEqual(["tools/a.test.sh", "tools/b.test.sh"]);
  });

  test("new shell suites are discovered without a list", () => {
    plant("tools/invented.test.sh", "true\n", false);
    expect(select(["tools/invented.test.sh"]).shell).toEqual(["tools/invented.test.sh"]);
  });
});

describe("UI package discovery and external dependencies", () => {
  test("selects check scripts, not an explicit package list", () => {
    pkg("apps/a");
    pkg("apps/b");
    plant("apps/skipped/package.json", '{"name":"no-check"}');
    expect(select(["apps/a/src/main.ts"]).ui).toEqual(["apps/a"]);
    expect(select([], [], true).ui).toEqual(["apps/a", "apps/b"]);
  });

  test("recursively follows relative imports outside a package", () => {
    pkg("apps/a");
    pkg("apps/b");
    plant("shared/value.ts", "export const x = 1;");
    plant("shared/bridge.ts", "export { x } from './value';");
    plant("apps/a/src/main.ts", "import { x } from '../../../shared/bridge.ts';");
    expect(select(["shared/value.ts"]).ui).toEqual(["apps/a"]);
  });

  test("local workspace dependencies include transitive package configs and sources", () => {
    pkg("apps/ui", { dependencies: { middle: "workspace:*" } });
    plant("packages/middle/package.json", '{"name":"middle","dependencies":{"base":"workspace:*"}}');
    plant("packages/middle/src/main.ts", "export const x = 1;");
    plant("packages/base/package.json", '{"name":"base"}');
    plant("packages/base/src/main.ts", "export const x = 1;");
    plant("packages/base/tsconfig.json", "{}");
    expect(select(["packages/base/src/main.ts"]).ui).toEqual(["apps/ui"]);
    expect(select(["packages/base/tsconfig.json"]).ui).toEqual(["apps/ui"]);
  });

  test("file dependencies are resolved even when the dependency key differs from package name", () => {
    pkg("apps/ui", { dependencies: { alias: "file:../../shared" } });
    plant("shared/package.json", '{"name":"actual-name"}');
    plant("shared/value.ts", "export const x = 1;");
    expect(select(["shared/value.ts"]).ui).toEqual(["apps/ui"]);
  });

  test("package locks/configs and root configs select their checks", () => {
    pkg("apps/a");
    pkg("apps/b");
    plant("apps/a/tsconfig.json", "{}");
    expect(select(["apps/a/bun.lock"]).ui).toEqual(["apps/a"]);
    expect(select(["apps/a/tsconfig.json"]).ui).toEqual(["apps/a"]);
    expect(select(["tsconfig.json"]).ui).toEqual(["apps/a", "apps/b"]);
  });

  test("external tsconfig extends chains select the reader package", () => {
    pkg("apps/a");
    pkg("apps/b");
    plant("apps/a/tsconfig.json", '{"extends":"../../config/shared.json"}');
    plant("config/shared.json", '{"extends":"./base.json"}');
    plant("config/base.json", "{}");
    expect(select(["config/base.json"]).ui).toEqual(["apps/a"]);
  });

  test("bare local imports carry dependencies outside the imported package", () => {
    pkg("apps/a");
    pkg("apps/b");
    plant("packages/shared/package.json", '{"name":"shared"}');
    plant("packages/shared/value.ts", "export { x } from '../../external/value.ts';");
    plant("external/value.ts", "export const x = 1;");
    plant("apps/a/src/main.ts", 'import { x } from "shared";');
    expect(select(["external/value.ts"]).ui).toEqual(["apps/a"]);
  });

  test("untracked UI manifests fail the same discovery contract as the CLI", () => {
    plant("ui/package.json", '{"scripts":{"check":"tsc"}}', false);
    plant("ui/bun.lock", "{}", false);
    expect(() => select([], [], true)).toThrow("must be tracked");
  });

  test("UI discovery violations and unresolved workspace dependencies fail closed", () => {
    plant("ui/package.json", '{"scripts":{"check":"tsc"}}');
    expect(() => select([], [], true)).toThrow("lockfile");
    plant("ui/bun.lock", "{}");
    plant("ui/package.json", '{"dependencies":{"svelte":"*"}}');
    expect(() => select([], [], true)).toThrow("no check script");
    plant("ui/package.json", '{"scripts":{"check":"tsc"},"dependencies":{"missing":"workspace:*"}}');
    expect(() => select([], [], true)).toThrow("unresolved local dependency");
  });
});

describe("gates and full verification", () => {
  test("registry contains every supported repo-gates command", () => {
    expect(Object.values(GATES).map((g) => g.cmd.join(" ")).sort()).toEqual([
      "tools/check-generator-inputs-tracked.sh", "tools/check-architecture-fresh.sh",
      "tools/check-service-tomls.sh", "tools/check-route-schemas.sh", "tools/check-manifest-integrity.sh",
      "tools/check-doc-self-references.sh", "tools/check-skill-metadata.sh", "tools/check-skill-metadata.test.sh",
      "tools/check-presentations-theme-contract.sh", "tools/check-presentations-theme-contract.test.sh", "tools/skill-eval check --all",
      "tools/check-publication-hygiene.sh", "tools/check-bun-install-policy.sh", "tools/check-store-transactions.sh",
      "tools/check-obsidian-plugins.sh", "tools/self check", "tools/persistence-orphans.test.sh",
    ].sort());
  });

  test("unrelated docs skip non-reading gates while preserving doc and publication checks", () => {
    plant("docs/plans/unrelated.md", "a plan");
    plant("readers/doc.test.ts", 'export const path = "docs/plans/unrelated.md";');
    plant("unrelated.test.ts", "export const x = 1;");
    const plan = select(["docs/plans/unrelated.md"]);
    expect(plan.gates).toEqual(["doc-self-references", "publication-hygiene"]);
    expect(plan.bun).toEqual(["readers/doc.test.ts"]);
    for (const id of ["architecture-fresh", "service-tomls", "store-transactions", "persistence-orphans", "self"]) {
      expect(plan.gates).not.toContain(id);
    }
  });

  test.each([
    ["crates/thing/README.md", "architecture-fresh"],
    ["ARCHITECTURE.md", "architecture-fresh"],
    ["Packs/demo/pack.toml", "architecture-fresh"],
    ["crates/thing/service.toml", "service-tomls"],
    ["Packs/demo/skills/example/SKILL.md", "skill-metadata"],
    ["Packs/demo/skills/example/SKILL.md", "skill-metadata-test"],
    ["Packs/presentations/skills/slide-deck/assets/themes/new.json", "presentations-theme-contract"],
    ["Packs/presentations/skills/slide-deck/assets/themes/new.json", "presentations-theme-contract-test"],
    ["apps/checkable/bunfig.toml", "bun-install-policy"],
    ["plugins/anywhere/manifest.json", "obsidian-plugins"],
    ["self.json", "self"],
    ["tools/check-store-transactions.sh", "store-transactions"],
    ["tools/route-schema-baseline.txt", "route-schemas"],
    ["crates/thing/service.toml", "manifest-integrity"],
    ["Packs/demo/skills/example/evals/fixture.md", "skill-eval"],
  ])("gate input %s selects %s", (file, gate) => {
    expect(select([file]).gates).toContain(gate);
  });

  test("a gate script edit does not select unrelated gates", () => {
    expect(select(["tools/check-store-transactions.sh"]).gates).toEqual(["publication-hygiene", "self", "store-transactions"]);
  });

  test("shared tooling changes reach every gate", () => {
    expect(select(["tools/lib/toml.sh"]).gates).toEqual(Object.keys(GATES).sort());
  });

  test("Cargo and sjel-cli implementation changes reach all CLI-backed gates", () => {
    for (const file of ["Cargo.toml", "Cargo.lock", "crates/sjel-cli/src/main.rs", ".cargo/config.toml"]) {
      const gates = select([file]).gates;
      for (const id of ["self", "persistence-orphans"]) expect(gates).toContain(id);
    }
  });

  test("unknown executable changes widen gates rather than rely on path ownership guesses", () => {
    plant("scripts/unregistered-tool.py", "print('ready')");
    const plan = select(["scripts/unregistered-tool.py"]);
    expect(plan.gates).toEqual(Object.keys(GATES).sort());
    expect(plan.reasons.join("\n")).toContain("unknown executable input");
  });

  test("eval fixture references outside the skill select the suite reader", () => {
    plant("Packs/demo/skills/example/evals/evals.json", JSON.stringify({ evals: [{ files: ["../../../../docs/example.md"] }] }));
    plant("docs/example.md", "fixture");
    const gates = select(["docs/example.md"]).gates;
    expect(gates).toEqual(["doc-self-references", "publication-hygiene", "skill-eval"]);
  });

  test("publication hygiene is selected even for no changes and inert docs", () => {
    expect(select([]).gates).toEqual(["publication-hygiene"]);
    expect(select(["docs/plan.md"]).gates).toContain("publication-hygiene");
    expect(GATES["publication-hygiene"].cmd).toEqual(["tools/check-publication-hygiene.sh"]);
  });

  test("unregistered new gates fail closed; site payload is not a standalone repo gate", () => {
    plant("tools/check-site-payload.sh", "true\n");
    expect(() => select([], [], true)).not.toThrow();
    plant("tools/check-new-invariant.sh", "true\n");
    expect(() => select([], [], true)).toThrow("Unregistered repository gate");
    expect(() => select(["README.md"])).toThrow("Unregistered repository gate");
  });

  test("full includes every target and every security language", () => {
    plant("one.test.ts", "export const x = 1;");
    plant("deep/two.spec.js", "export const x = 1;");
    plant("tools/new.test.sh", "true\n");
    pkg("ui");
    const plan = select([], [], true);
    expect(plan.bun).toEqual(["deep/two.spec.js", "one.test.ts"]);
    expect(plan.shell).toEqual(["tools/new.test.sh"]);
    expect(plan.ui).toEqual(["ui"]);
    expect(plan.gates).toEqual(Object.keys(GATES).sort());
    expect(plan.pages).toBe(true);
    expect(plan.security).toEqual({ deps: true, images: true, codeql: ["actions", "javascript-typescript", "python", "rust"] });
  });

  test.each(["tools/ci-plan.ts", "tools/ci-run.ts", "tools/lib/ci-inputs.ts", ".github/workflows/ci.yml"])("%s selects all verification work", (file) => {
    plant("a.test.ts", "export const x = 1;");
    pkg("ui");
    const plan = select([file]);
    expect(plan.bun).toEqual(["a.test.ts"]);
    expect(plan.ui).toEqual(["ui"]);
    expect(plan.pages).toBe(true);
    expect(plan.security.codeql).toHaveLength(4);
  });
});

describe("Pages and security boundaries", () => {
  test.each([
    "website/build", "website/content/guide.md", "crates/thing/README.md", "crates/thing/service.toml", "Packs/a/pack.toml",
    "self.json", "tools/self", "tools/lib/sjel-cli.sh", "crates/sjel-cli/src/self_model/mod.rs", "Cargo.toml", "Cargo.lock",
    "apps/gpui/core/src/lib.rs", ".github/workflows/pages.yml", "upstreams.toml", "brand/sjel-hedgehog.svg",
  ])("Pages includes %s", (file) => expect(select([file]).pages).toBe(true));

  test.each(["Cargo.lock", "apps/ui/package.json", "apps/ui/bun.lock", "apps/ui/bunfig.toml", "osv-scanner.toml", "pyproject.toml", "uv.lock"])("dependency scanner includes %s", (file) => {
    expect(select([file]).security.deps).toBe(true);
  });

  test.each(["services/image/service.toml", "config/service.schema.json", "schemas/service.toml.example", "tools/check-service-tomls.sh", ".grype.yaml", ".github/workflows/security.yml"])("image scanner includes %s", (file) => {
    expect(select([file]).security.images).toBe(true);
  });

  test.each([
    ["crates/a/src/lib.rs", "rust"], ["Cargo.lock", "rust"], ["src/a.tsx", "javascript-typescript"], ["package.json", "javascript-typescript"],
    ["scripts/a.py", "python"], ["requirements-dev.txt", "python"], [".github/workflows/pages.yml", "actions"], [".github/actions/a/action.yml", "actions"],
  ])("CodeQL %s selects %s", (file, language) => expect(select([file]).security.codeql).toContain(language));

  test("CodeQL configuration selects all languages and scanner workflow reaches both scanner jobs", () => {
    expect(select([".github/codeql/config.yml"]).security.codeql).toEqual(["actions", "javascript-typescript", "python", "rust"]);
    expect(select([".github/workflows/security.yml"]).security).toEqual({ deps: true, images: true, codeql: ["actions"] });
  });
});
