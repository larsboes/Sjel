import { createHash } from "node:crypto";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, copyFileSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";

const root = resolve(import.meta.dir, "..");
const fixtures = join(root, "research/benchmarks/skill-intake");
const spec = JSON.parse(readFileSync(join(fixtures, "cases.json"), "utf8"));
const [source, cli, output] = process.argv.slice(2);
if (!source || !cli || !output) throw new Error("usage: bun --no-env-file tools/skill-scanner-eval.ts <reviewed-source-clone> <sjel-cli> <output.json>");
const scratch = mkdtempSync(join(tmpdir(), "sjel-skill-scanner-"));
function run(args: string[], cwd = root, allowed = [0]): string {
  const result = Bun.spawnSync(args, { cwd, stdout: "pipe", stderr: "pipe", timeout: 900_000 });
  if (!allowed.includes(result.exitCode)) throw new Error(`${args[0]} exited ${result.exitCode}: ${result.stderr.toString().slice(-3000)}`);
  return result.stdout.toString();
}
try {
  if (run(["git", "rev-parse", "HEAD"], source).trim() !== spec.source_revision) throw new Error("source revision differs from the reviewed fixture pin");
  const context = join(scratch, "context");
  mkdirSync(join(context, "scanner"), { recursive: true });
  const archive = join(scratch, "source.tar");
  run(["git", "archive", "--format=tar", "--output", archive, spec.source_revision], source);
  run(["tar", "-xf", archive, "-C", join(context, "scanner")]);
  run(["git", "apply", "--unidiff-zero", join(fixtures, "upstream-lock.patch")], join(context, "scanner"));
  run(["osv-scanner", "scan", "source", "--lockfile", join(context, "scanner/uv.lock")]);
  const lockDigest = createHash("sha256").update(readFileSync(join(context, "scanner/uv.lock"))).digest("hex");
  if (lockDigest !== spec.lock_digest) throw new Error("patched lockfile differs from the reviewed fixture pin");
  copyFileSync(join(fixtures, "Containerfile"), join(context, "Containerfile"));
  copyFileSync(join(fixtures, "runner.py"), join(context, "runner.py"));
  const iid = join(scratch, "image-id");
  process.stderr.write("Building the reviewed scanner in a disposable Docker context.\n");
  run(["docker", "build", "--iidfile", iid, "-f", join(context, "Containerfile"), context]);
  const image = readFileSync(iid, "utf8").trim();
  const corpus = join(scratch, "cases");
  mkdirSync(corpus);
  const rows = [];
  for (const test of spec.cases) {
    if (!/^[a-z-]+$/.test(test.id)) throw new Error("invalid fixture name");
    const dir = join(corpus, test.id);
    mkdirSync(dir);
    writeFileSync(join(dir, "SKILL.md"), `---\nname: ${test.id}\ndescription: Check the supplied synthetic fixture.\n---\n\n${test.body}\n`);
    if (test.script) {
      mkdirSync(join(dir, "scripts"));
      writeFileSync(join(dir, "scripts/check.sh"), test.script);
    }
    const baseline = Bun.spawnSync([resolve(cli), "intake", "scan", dir], { stdout: "pipe", stderr: "pipe", timeout: 10_000 });
    if (![0, 2].includes(baseline.exitCode)) throw new Error(`Sjel scan failed for ${test.id}`);
    const started = performance.now();
    const result = JSON.parse(run(["docker", "run", "--rm", "--pull=never", "--network=none", "--read-only", "--cap-drop=ALL",
      "--security-opt=no-new-privileges", "--pids-limit=64", "--memory=2g", "--cpus=2", "--tmpfs=/tmp:rw,nosuid,size=256m",
      "--mount", `type=bind,src=${corpus},dst=/fixtures,readonly`, image, `/fixtures/${test.id}`]));
    if (result.status !== "reported" || !Array.isArray(result.report?.findings)) throw new Error(`scanner coverage missing for ${test.id}: ${JSON.stringify(result)}`);
    const hold = result.report.findings.some((f: any) => ["MEDIUM", "HIGH", "CRITICAL"].includes(String(f.severity).toUpperCase()));
    rows.push({ id: test.id, expected_hold: test.hold, sjel_hold: baseline.exitCode === 2, scanner_hold: hold,
      elapsed_ms: Math.round(performance.now() - started), analyzers: result.report.analyzers_used,
      findings: result.report.findings.map((f: any) => ({ rule: f.rule_id, severity: f.severity })) });
  }
  writeFileSync(resolve(output), JSON.stringify({ source_revision: spec.source_revision, lock_digest: lockDigest,
    image, network: "none", corpus: "synthetic smoke cases; not a held-out efficacy benchmark", rows }, null, 2) + "\n");
  process.stderr.write(`Wrote ${rows.length} cases to ${resolve(output)}\n`);
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
