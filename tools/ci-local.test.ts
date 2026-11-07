import { afterEach, describe, expect, setDefaultTimeout, test } from "bun:test";
import { mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { REFUSED, RUNS_HERE, UNCLASSIFIED, parseJobs } from "./lib/ci-workflow.ts";

setDefaultTimeout(30_000);

const workflow = (body: string) => `name: CI\non: [push]\njobs:\n${body}`;
const adapterJob = (id: string, task: string, condition: string, script = `bun tools/ci-run.ts ${task}`) => workflow(`  ${id}:
    needs: changes
    if: ${condition}
    steps:
      - uses: actions/checkout@v5
      - run: ${script}
`);

describe("parseJobs", () => {
  test("setup actions do not count as shell work", () => {
    const jobs = parseJobs(workflow(`  repo-gates:
    steps:
      - uses: actions/checkout@v5
      - name: Publication hygiene
        run: tools/check-publication-hygiene.sh
`));
    expect(jobs).toHaveLength(1);
    expect(jobs[0].steps).toEqual([{ name: "Publication hygiene", script: "tools/check-publication-hygiene.sh" }]);
  });

  test("a step with neither run nor uses throws", () => {
    expect(() => parseJobs(workflow("  repo-gates:\n    steps:\n      - name: nothing\n"))).toThrow("repo-gates: step 1 has neither run: nor uses:");
  });

  test("an unnamed multiline step uses its first line as the title", () => {
    const job = parseJobs(workflow(`  repo-gates:
    steps:
      - run: |
          tools/check-publication-hygiene.sh
          echo done
`))[0];
    expect(job.steps[0].name).toBe("tools/check-publication-hygiene.sh");
    expect(job.steps[0].script).toContain("echo done");
  });

  test("working directories are preserved, absent directories stay absent", () => {
    const jobs = parseJobs(workflow(`  bun-tests:
    steps:
      - run: bun run test
        working-directory: package
      - run: bun test
`));
    expect(jobs[0].steps[0].workingDirectory).toBe("package");
    expect("workingDirectory" in jobs[0].steps[1]).toBe(false);
  });

  test("unsupported step and workflow semantics refuse replay", () => {
    for (const setting of ["if: false", "env: { MODE: ci }", "shell: python", "continue-on-error: true"]) {
      expect(parseJobs(workflow(`  repo-gates:\n    steps:\n      - run: echo ok\n        ${setting}\n`))[0].refusal).toContain("cannot be replayed faithfully");
    }
    expect(parseJobs(workflow("  repo-gates:\n    steps:\n      - run: echo '${{ github.ref }}'\n"))[0].refusal).not.toBeNull();
    expect(parseJobs("env: { MODE: ci }\n" + workflow("  repo-gates:\n    steps:\n      - run: echo ok\n"))[0].refusal).not.toBeNull();
    expect(parseJobs(workflow("  repo-gates:\n    steps:\n      - uses: actions/checkout@v5\n        if: false\n      - run: echo ok\n"))[0].refusal).not.toBeNull();
  });

  test("a workflow with no jobs throws", () => {
    expect(() => parseJobs("name: CI\non: [push]\n")).toThrow("declares no jobs");
  });
});

const planTransfer = `      - name: Read verification plan
        env:
          GH_TOKEN: \${{ github.token }}
        run: gh run download "$GITHUB_RUN_ID" --name verification-plan --dir "$RUNNER_TEMP"
`;

describe("owned full local adapters", () => {
  const cases = [["repo-gates", "gates", "gates"], ["bun-tests", "bun-tests", "bun"]];
  test("only exact owned wrappers adapt recognized changes conditions to full replay", () => {
    for (const [id, task, output] of cases) {
      for (const condition of [`needs.changes.outputs.${output} == 'true'`, `\${{ needs.changes.outputs.${output} == 'true' }}`]) {
        const job = parseJobs(adapterJob(id, task, condition))[0];
        expect(job.refusal).toBeNull();
        expect(job.steps[0].script).toBe(`bun tools/ci-run.ts ${task} --full`);
      }
      const plain = parseJobs(workflow(`  ${id}:\n    steps:\n      - run: bun tools/ci-run.ts ${task}\n`))[0];
      expect(plain.steps[0].script).toBe(`bun tools/ci-run.ts ${task} --full`);
    }
  });

  test("exact CI artifact transfer is omitted only before an owned full runner", () => {
    for (const [id, task, output] of cases) {
      const text = adapterJob(id, task, `needs.changes.outputs.${output} == 'true'`).replace("      - uses: actions/checkout@v5\n", planTransfer);
      const job = parseJobs(text)[0];
      expect(job.refusal).toBeNull();
      expect(job.steps).toEqual([{ name: `bun tools/ci-run.ts ${task}`, script: `bun tools/ci-run.ts ${task} --full` }]);
      for (const altered of [
        text.replace("Read verification plan", "Download other artifact"),
        text.replace("--name verification-plan", "--name something-else"),
        text.replace("GH_TOKEN: \${{ github.token }}", "GH_TOKEN: \${{ secrets.OTHER_TOKEN }}"),
        text.replace("        env:\n", "        if: false\n        env:\n"),
        text.replace("        env:\n", "        timeout-minutes: 2\n        env:\n"),
        text.replace("          GH_TOKEN:", "          EXTRA: value\n          GH_TOKEN:"),
        text.replace(`bun tools/ci-run.ts ${task}`, "echo unrelated"),
        text.replace(planTransfer, "") + planTransfer,
      ]) expect(parseJobs(altered)[0].refusal).not.toBeNull();
    }
  });

  test("unknown conditions, extra arguments, extra needs and scoped wrappers remain refused", () => {
    for (const [id, task, output] of cases) {
      const condition = `needs.changes.outputs.${output} == 'true'`;
      for (const text of [
        adapterJob(id, task, "github.ref == 'refs/heads/main'"),
        adapterJob(id, task, condition, `bun tools/ci-run.ts ${task} --unexpected`),
        adapterJob(id, task, condition).replace("needs: changes", "needs: [changes, other]"),
        adapterJob(id, task, condition).replace("    steps:", "    env: { MODE: ci }\n    steps:"),
        adapterJob(id, task, condition).replace(`      - run: bun tools/ci-run.ts ${task}`, `      - working-directory: package\n        run: bun tools/ci-run.ts ${task}`),
        adapterJob(id, task, condition).replace(`      - run: bun tools/ci-run.ts ${task}`, `      - if: false\n        run: bun tools/ci-run.ts ${task}`),
        adapterJob(id, task, condition, "echo unrelated"),
      ]) expect(parseJobs(text)[0].refusal).not.toBeNull();
    }
  });
});

describe("local safety classifications", () => {
  const jobsOf = (...ids: string[]) => parseJobs(workflow(ids.map((id) => `  ${id}:\n    steps:\n      - run: "true"\n`).join("")));
  test("safe jobs run; unknown jobs and destructive jobs refuse", () => {
    expect(jobsOf("repo-gates")[0].refusal).toBeNull();
    expect(jobsOf("some-new-job")[0].refusal).toBe(UNCLASSIFIED);
    expect(jobsOf("cargo-tests")[0].refusal).toContain("tools/cargo-hermetic");
    expect(jobsOf("ui-check")[0].refusal).toContain("machine.toml");
    for (const id of Object.keys(RUNS_HERE)) expect(REFUSED[id]).toBeUndefined();
  });
  test("real workflow parses without green empty jobs", async () => {
    const jobs = parseJobs(await Bun.file(`${import.meta.dir}/../.github/workflows/ci.yml`).text());
    for (const id of ["repo-gates", "bun-tests"]) {
      const job = jobs.find((j) => j.id === id);
      expect(job).toBeDefined();
      expect(job!.refusal).toBeNull();
      expect(job!.steps.some((step) => step.script.includes("gh run download"))).toBe(false);
      expect(job!.steps.some((step) => step.script.endsWith(" --full"))).toBe(true);
    }
    for (const job of jobs) if (!job.refusal) expect(job.steps.length).toBeGreaterThan(0);
  });
});

const scratch: string[] = [];
afterEach(() => { for (const dir of scratch.splice(0)) rmSync(dir, { recursive: true, force: true }); });

function fastFixture() {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "ci-local-fast-")));
  scratch.push(root);
  const put = (path: string, text: string, mode = 0o644) => {
    mkdirSync(resolve(root, path, ".."), { recursive: true });
    writeFileSync(join(root, path), text, { mode });
  };
  const env = { ...process.env, PATH: `${root}/bin:${process.env.PATH}`, FIXTURE_ROOT: root };
  const git = (...args: string[]) => {
    const result = Bun.spawnSync(["git", ...args], { cwd: root, env });
    if (result.exitCode !== 0) throw new Error(result.stderr.toString());
  };
  put(".gitignore", "calls\nbin/\n");
  put("Cargo.toml", '[workspace]\nmembers = ["crates/root"]\n');
  put("crates/root/Cargo.toml", '[package]\nname = "fixture-root"\nversion = "0.0.0"\n');
  put("crates/root/src/lib.rs", "pub fn root() {}\n");
  put("apps/gpui/Cargo.toml", '[workspace]\nmembers = ["core"]\n');
  put("apps/gpui/core/Cargo.toml", '[package]\nname = "sjel-gpui-core"\nversion = "0.0.0"\n');
  put("apps/gpui/core/src/lib.rs", "pub fn core() {}\n");
  put("tools/ci-run.ts", 'import { appendFileSync } from "node:fs"; appendFileSync(`${import.meta.dir}/../calls`, `gates ${process.argv.slice(2).join(" ")}\\n`); if (process.argv[2] !== "gates" || !process.argv.includes("--ids")) process.exit(1);\n');
  put("tools/check-publication-hygiene.sh", '#!/bin/sh\nprintf "publication\\n" >> calls\n', 0o755);
  put("tools/poison.test.sh", '#!/bin/sh\nprintf "TEST-RAN\\n" >> calls\nexit 1\n', 0o755);
  put("poison.test.ts", 'throw new Error("TEST-RAN");\n');
  put("bin/cargo", `#!/bin/sh
if [ "$1" = metadata ]; then
  case "$*" in
    *apps/gpui/Cargo.toml*) name=sjel-gpui-core; dir=apps/gpui/core ;;
    *) name=fixture-root; dir=crates/root ;;
  esac
  printf '{"workspace_members":["%s"],"packages":[{"id":"%s","name":"%s","manifest_path":"%s/%s/Cargo.toml","dependencies":[]}]}\\n' "$name" "$name" "$name" "$FIXTURE_ROOT" "$dir"
elif [ "$1" = fmt ]; then
  printf 'cargo %s\\n' "$*" >> "$FIXTURE_ROOT/calls"
  exit "\${FMT_EXIT:-0}"
elif [ "$1" = --version ]; then
  printf 'cargo fixture\\n'
else
  printf 'FORBIDDEN cargo %s\\n' "$*" >> "$FIXTURE_ROOT/calls"
  exit 1
fi
`, 0o755);
  put("bin/rustfmt", "#!/bin/sh\nprintf 'rustfmt fixture\\n'\n", 0o755);
  git("init", "-q"); git("add", ".");
  git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false", "commit", "-qm", "fixture");
  const run = (...args: string[]) => Bun.spawnSync([process.execPath, join(import.meta.dir, "ci-local.ts"), "fast", "--root", root, "--base", "HEAD", ...args], { env, stdout: "pipe", stderr: "pipe" });
  return { root, put, env, run, calls: () => readFileSync(join(root, "calls"), "utf8") };
}

describe("fast checks over synthetic roots", () => {
  test("no Rust impact means no formatting, tests, clippy or build; publication always runs", () => {
    const f = fastFixture();
    f.put("README.md", "fixture prose\n");
    const p = f.run();
    expect(p.exitCode).toBe(0);
    expect(p.stdout.toString()).toContain("defer tests, clippy, builds");
    expect(f.calls()).toContain("publication");
    expect(f.calls()).not.toMatch(/cargo fmt|TEST-RAN|FORBIDDEN|clippy|cargo test/);
  });

  test("root Rust impact runs format and cheap gate IDs, not tests or clippy", () => {
    const f = fastFixture();
    f.put("crates/root/src/lib.rs", "pub fn changed() {}\n");
    expect(f.run().exitCode).toBe(0);
    expect(f.calls()).toContain("cargo fmt --all --check");
    expect(f.calls()).toContain("gates gates --ids");
    expect(f.calls()).not.toMatch(/TEST-RAN|FORBIDDEN|clippy|cargo test|architecture-fresh|marketplace-fresh|skill-eval/);
  });

  test("GPUI impact selects its format check, and format failure blocks", () => {
    const f = fastFixture();
    f.put("apps/gpui/core/src/lib.rs", "pub fn changed() {}\n");
    const p = Bun.spawnSync([process.execPath, join(import.meta.dir, "ci-local.ts"), "fast", "--root", f.root, "--base", "HEAD"], { env: { ...f.env, FMT_EXIT: "1" }, stdout: "pipe", stderr: "pipe" });
    expect(p.exitCode).toBe(1);
    expect(f.calls()).toContain("cargo fmt --manifest-path apps/gpui/Cargo.toml --all --check");
    expect(f.calls()).toContain("publication");
    expect(f.calls()).not.toMatch(/TEST-RAN|FORBIDDEN|clippy|cargo test/);
  });

  test("unknown comparison bases widen format checks instead of reporting empty success", () => {
    const f = fastFixture();
    const p = f.run("--base", "missing-base");
    expect(p.exitCode).toBe(0);
    expect(f.calls()).toContain("cargo fmt --all --check");
    expect(f.calls()).toContain("cargo fmt --manifest-path apps/gpui/Cargo.toml --all --check");
  });
});
