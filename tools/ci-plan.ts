import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { changedFiles, rootPackages, rustImpact, type Impact, type Package } from "./ci-impact.ts";
import { verificationInputs } from "./lib/ci-inputs.ts";

export interface PlatformCheck {
  name: string;
  os: string;
  target: string;
  check: string;
}

export interface CIPlan extends ReturnType<typeof verificationInputs> {
  version: 1;
  head: string;
  base: string | null;
  full: boolean;
  files: string[];
  rust: Impact;
  gpui: { include: PlatformCheck[] };
}

function command(root: string, args: string[]): string {
  const result = Bun.spawnSync(args, { cwd: root, stdout: "pipe", stderr: "pipe" });
  if (result.exitCode !== 0) throw new Error(`${args[0]} ${args[1]} failed: ${result.stderr.toString().trim()}`);
  return result.stdout.toString().trim();
}

/** Platform checks reached by selected GPUI members; unknown package roles widen to all. */
export function platformChecks(selected: string[], members: string[]): PlatformCheck[] {
  const known = ["sjel-gpui", "sjel-gpui-core", "sjel-gpui-ui", "sjel-gpui-ios"];
  const packages = selected.some((name) => !known.includes(name)) ? members : selected;
  for (const name of packages) if (!/^[a-zA-Z0-9_-]+$/.test(name)) throw new Error("invalid Cargo package name");
  const has = new Set(packages);
  const flags = (names: string[]) => names.filter((name) => has.has(name)).map((name) => `-p ${name}`).join(" ");
  const desktop = flags(members.filter((name) => name !== "sjel-gpui-ios"));
  const shared = flags(members.filter((name) => name !== "sjel-gpui-ios" && name !== "sjel-gpui"));
  const mac = [
    desktop && `cargo check --locked ${desktop} --all-targets`,
    has.has("sjel-gpui-ios") && "cargo check --locked -p sjel-gpui-ios --target aarch64-apple-ios",
  ].filter(Boolean).join(" && ");
  return [
    { name: "macos + ios", os: "xcode-27", target: has.has("sjel-gpui-ios") ? "aarch64-apple-ios" : "", check: mac },
    { name: "linux", os: "ubuntu-latest", target: "", check: desktop && `cargo check --locked ${desktop} --all-targets` },
    { name: "windows", os: "windows-latest", target: "", check: shared && `cargo check --locked ${shared} --all-targets` },
    { name: "windows shell", os: "windows-latest", target: "", check: has.has("sjel-gpui") ? "cargo check --locked -p sjel-gpui --all-targets" : "" },
    { name: "android", os: "ubuntu-latest", target: "aarch64-linux-android", check: shared && `cargo check --locked ${shared} --target aarch64-linux-android` },
  ].filter((entry): entry is PlatformCheck => Boolean(entry.check));
}

const plannerInput = (file: string) => file.startsWith(".github/workflows/ci.") ||
  /^tools\/(ci-(plan|run|impact|changes)(\.|$)|lib\/ci-(inputs|workflow))/.test(file);

/** Create the shared local/CI plan. Missing bases and failed selection widen to full verification.
 * @throws If full input discovery also fails; incomplete coverage must not report success.
 */
export function createPlan(root: string, base: string | undefined, full = false, local = false): CIPlan {
  const reasons: string[] = [];
  let files: string[] = [];
  let deleted: string[] = [];
  if (!base && !local) {
    full = true;
    reasons.push("No verified comparison base: full verification");
  }
  if (base || local) {
    try {
      if (base && !local) command(root, ["git", "merge-base", "--is-ancestor", base, "HEAD"]);
      const change = changedFiles(root, base, local);
      files = [...change.files].sort();
      deleted = [...change.deleted].sort();
      if (files.some(plannerInput)) {
        full = true;
        reasons.push("Verification definition changed: full verification");
      }
    } catch (error) {
      full = true;
      reasons.push(`Comparison unavailable: ${error}`);
    }
  }
  const rootMembers = rootPackages(root);
  const gpuiMembers = rootPackages(root, "apps/gpui/Cargo.toml");
  let inputs: ReturnType<typeof verificationInputs>;
  try {
    inputs = verificationInputs(root, files, deleted, full);
  } catch (error) {
    full = true;
    reasons.push(`Input selection failed: ${error}; full verification`);
    inputs = verificationInputs(root, files, deleted, true);
  }
  const all = (packages: Package[]): Impact => ({ mode: "all", packages: packages.map((p) => p.name).sort(), skipped: [], reasons: ["Full verification"] });
  const rust = full ? all(rootMembers) : rustImpact(files, rootMembers, deleted);
  const gpuiFiles = files.map((file) => /^(apps\/gpui\/)?(Cargo\.(toml|lock)|rust-toolchain\.toml|clippy\.toml)$/.test(file) || file.startsWith("apps/gpui/.cargo/") ? "Cargo.toml" : file);
  const combined = [...rootMembers, ...gpuiMembers];
  const gpuiImpact = full ? all(gpuiMembers) : rustImpact(gpuiFiles, combined, deleted);
  const gpuiSelected = gpuiMembers.filter((p) => gpuiImpact.packages.includes(p.name)).map((p) => p.name);
  return {
    ...inputs, version: 1, head: command(root, ["git", "rev-parse", "HEAD"]), base: base ?? null,
    full, files, rust, gpui: { include: platformChecks(gpuiSelected, gpuiMembers.map((p) => p.name)) },
    reasons: [...reasons, ...inputs.reasons, ...rust.reasons, ...gpuiImpact.reasons],
  };
}

export interface WorkflowRun { id: number; head_sha: string; event: string; conclusion: string; html_url: string }

/** Pick a successful ancestor, never the previous push merely because it finished. */
export function successfulBase(runs: WorkflowRun[], ancestor: (sha: string) => boolean): string | undefined {
  return runs.find((run) => run.conclusion === "success" && ["push", "schedule", "workflow_dispatch"].includes(run.event) && ancestor(run.head_sha))?.head_sha;
}

function api(root: string, endpoint: string): any {
  return JSON.parse(command(root, ["gh", "api", endpoint]));
}

function repository(root: string): string {
  const name = process.env.GITHUB_REPOSITORY ?? command(root, ["gh", "repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner"]);
  if (!/^[\w.-]+\/[\w.-]+$/.test(name)) throw new Error("invalid repository name");
  return name;
}

function runs(root: string, repo: string, workflow: string, status = "success"): WorkflowRun[] {
  if (!/^[\w.-]+\.yml$/.test(workflow)) throw new Error("invalid workflow filename");
  return api(root, `repos/${repo}/actions/workflows/${workflow}/runs?branch=main&status=${status}&per_page=100`).workflow_runs;
}

/** Find a successful full-verification marker for an exact release commit.
 * @throws On API errors or when no successful full verification exists.
 */
export function requireFull(root: string, sha: string): void {
  if (!/^[a-f0-9]{40}$/.test(sha)) throw new Error("--require-full needs a full commit SHA");
  const repo = repository(root);
  const candidates: WorkflowRun[] = api(root, `repos/${repo}/actions/workflows/ci.yml/runs?head_sha=${sha}&status=success&per_page=100`).workflow_runs;
  for (const run of candidates) {
    if (run.head_sha !== sha || run.conclusion !== "success") continue;
    const jobs = api(root, `repos/${repo}/actions/runs/${run.id}/jobs?per_page=100`).jobs;
    if (jobs.some((job: any) => job.name === "full verification result" && job.conclusion === "success")) {
      console.log(`Full verification passed for ${sha}: ${run.html_url}`);
      return;
    }
  }
  throw new Error(`No successful full verification for ${sha}. Dispatch ci.yml with full=true at this commit before releasing.`);
}

/** Stable GitHub outputs used by all selective workflows. */
export function outputs(plan: CIPlan): Record<string, string> {
  return {
    full: String(plan.full), rust: String(plan.rust.packages.length > 0),
    quality: String(plan.rust.packages.length > 0 || plan.gpui.include.length > 0),
    bun: String(plan.bun.length + plan.shell.length > 0), ui: String(plan.ui.length > 0),
    gates: String(plan.gates.length > 0), gpui: String(plan.gpui.include.length > 0),
    gpui_matrix: JSON.stringify(plan.gpui), pages: String(plan.pages), deps: String(plan.security.deps),
    images: String(plan.security.images), codeql_run: String(plan.security.codeql.length > 0),
    codeql: JSON.stringify({ include: plan.security.codeql.map((language) => ({ language })) }),
  };
}

/** Human-readable coverage, with selected targets and reasons rather than a green empty sweep. */
export function summary(plan: CIPlan): string {
  const line = (name: string, values: string[]) => `| ${name} | ${values.length ? values.map((v) => `\`${v.replaceAll("|", "\\|").replaceAll("`", "") }\``).join(", ") : "Skipped: no affected input"} |`;
  return [
    "## Verification plan", `Mode: **${plan.full ? "full" : "affected"}**. Commit: \`${plan.head}\`. Base: \`${plan.base ?? "none"}\`.`,
    "", "| Surface | Selected targets |", "|---|---|", line("Rust", plan.rust.packages),
    line("GPUI", plan.gpui.include.map((p) => p.name)), line("Bun tests", plan.bun), line("Shell tests", plan.shell),
    line("UI", plan.ui), line("Gates", plan.gates), line("CodeQL", plan.security.codeql),
    `| Pages / dependencies / images | ${plan.pages} / ${plan.security.deps} / ${plan.security.images} |`,
    "", "<details><summary>Selection reasons</summary>", "", ...plan.reasons.map((r) => `- ${r.replaceAll("\n", " ")}`), "", "</details>", "",
  ].join("\n");
}

async function main() {
  const args = Bun.argv.slice(2);
  const arg = (name: string) => {
    const at = args.indexOf(name);
    if (at < 0) return undefined;
    if (!args[at + 1] || args[at + 1].startsWith("--")) throw new Error(`${name} requires a value`);
    return args[at + 1];
  };
  const root = resolve(arg("--root") ?? join(import.meta.dir, ".."));
  const required = arg("--require-full");
  if (required) return requireFull(root, required);
  const event = process.env.GITHUB_EVENT_PATH ? JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, "utf8")) : {};
  const eventName = process.env.GITHUB_EVENT_NAME;
  let full = args.includes("--full") || ["schedule", "release", "workflow_dispatch", "workflow_call"].includes(eventName ?? "");
  let base = arg("--base") ?? event.pull_request?.base?.sha;
  const notes: string[] = [];
  if (!full && !base && !args.includes("--local") && eventName === "push") {
    try {
      const candidates = runs(root, repository(root), arg("--workflow") ?? "ci.yml");
      base = successfulBase(candidates, (sha) => Bun.spawnSync(["git", "merge-base", "--is-ancestor", sha, "HEAD"], { cwd: root }).exitCode === 0);
      notes.push(base ? "Compared against the last successful ancestor workflow run; cancelled and failed pushes remain in scope" : "No successful ancestor in the last 100 runs: full verification");
    } catch (error) {
      notes.push(`Baseline lookup unavailable: ${error}; full verification`);
      full = true;
    }
  }
  const plan = createPlan(root, base, full, args.includes("--local"));
  plan.reasons.unshift(...notes);
  const output = arg("--output");
  if (output) writeFileSync(output, JSON.stringify(plan, null, 2) + "\n");
  const githubOutput = arg("--github-output");
  if (githubOutput) appendFileSync(githubOutput, Object.entries(outputs(plan)).map(([key, value]) => `${key}=${value}\n`).join(""));
  let report = summary(plan);
  if (eventName && process.env.GITHUB_STEP_SUMMARY) {
    try {
      const recent = runs(root, repository(root), "ci.yml", "completed").filter((r) => ["schedule", "workflow_dispatch", "release"].includes(r.event));
      const latest = recent[0];
      report += latest ? `\nLatest full-run attempt: **${latest.conclusion}** — ${latest.html_url}\n` : "\nNo scheduled/manual full run found in the last 100 completed runs.\n";
      const repo = repository(root);
      const artifacts = api(root, `repos/${repo}/actions/artifacts?name=full-verification&per_page=20`).artifacts;
      let verified: WorkflowRun | undefined;
      for (const artifact of artifacts) {
        if (artifact.expired || artifact.workflow_run?.head_branch !== "main") continue;
        const run: WorkflowRun = api(root, `repos/${repo}/actions/runs/${artifact.workflow_run.id}`);
        if (run.conclusion === "success" && Bun.spawnSync(["git", "merge-base", "--is-ancestor", run.head_sha, "HEAD"], { cwd: root }).exitCode === 0) { verified = run; break; }
      }
      report += verified ? `Last successful full verification: ${verified.html_url}\n` : "No successful ancestor full-verification receipt found among the last 20 retained receipts.\n";
    } catch (error) {
      report += `\nFull-verification history unavailable: ${error}\n`;
    }
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, report);
  }
  console.log(report);
}

if (import.meta.main) main().catch((error) => { console.error(`ci-plan: ${error}`); process.exitCode = 1; });
