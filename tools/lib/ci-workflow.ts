/** One shell step, including its checkout-relative working directory. */
export interface WorkflowStep {
  name: string;
  script: string;
  workingDirectory?: string;
}

/** A workflow job and its fail-closed local replay verdict. */
export interface WorkflowJob {
  id: string;
  label: string;
  steps: WorkflowStep[];
  refusal: string | null;
}

/** Jobs whose commands have an explicit local safety classification. */
export const RUNS_HERE: Record<string, string> = {
  "repo-gates": "file-based repository gates",
  "bun-tests": "installs locked dependencies, then runs Bun and shell tests",
};

/** Jobs that require a disposable runner or a separate hermetic command. */
export const REFUSED: Record<string, string> = {
  "rust-quality":
    "cargo against the shared target/ can replace a live service binary. Run: tools/cargo-hermetic clippy --workspace --all-targets --all-features --locked -- -D warnings",
  "cargo-tests":
    "cargo against the shared target/ and real overlay. Run: tools/cargo-hermetic test --workspace --locked",
  "ui-check":
    "overwrites the machine.toml overlay configuration and installs dependencies. Run the package's check and build scripts separately",
};

/** A new job requires an explicit safety classification. */
export const UNCLASSIFIED =
  "not classified — decide in tools/lib/ci-workflow.ts whether replaying it here is safe, and record why";

const adapters: Record<string, { script: string; condition: string }> = {
  "repo-gates": { script: "bun tools/ci-run.ts gates", condition: "needs.changes.outputs.gates == 'true'" },
  "bun-tests": { script: "bun tools/ci-run.ts bun-tests", condition: "needs.changes.outputs.bun == 'true'" },
};

function isPlanTransfer(step: unknown): boolean {
  if (!step || typeof step !== "object" || Object.keys(step).sort().join(",") !== "env,name,run") return false;
  const transfer = step as { name: unknown; run: unknown; env: unknown };
  if (!transfer.env || typeof transfer.env !== "object" || Object.keys(transfer.env).join(",") !== "GH_TOKEN") return false;
  return transfer.name === "Read verification plan" &&
    transfer.run === 'gh run download "$GITHUB_RUN_ID" --name verification-plan --dir "$RUNNER_TEMP"' &&
    (transfer.env as Record<string, unknown>).GH_TOKEN === "${{ github.token }}";
}

/** Parse replayable shell steps; reject unsupported execution semantics instead of dropping them.
 * @throws If the workflow has no jobs or declares a step without run or uses.
 */
export function parseJobs(yamlText: string): WorkflowJob[] {
  const doc = Bun.YAML.parse(yamlText) as {
    env?: unknown;
    defaults?: unknown;
    jobs?: Record<string, {
      name?: string;
      env?: unknown;
      defaults?: unknown;
      if?: unknown;
      needs?: unknown;
      strategy?: unknown;
      container?: unknown;
      services?: unknown;
      "continue-on-error"?: unknown;
      steps?: Array<{
        name?: string; run?: string; uses?: string; "working-directory"?: string;
        if?: unknown; env?: unknown; shell?: unknown; "continue-on-error"?: unknown;
      }>;
    }>;
  };
  if (!doc?.jobs || typeof doc.jobs !== "object") throw new Error("workflow declares no jobs");
  return Object.entries(doc.jobs).map(([id, job]) => {
    const steps: WorkflowStep[] = [];
    const adapter = adapters[id];
    const wrapperIndex = adapter ? (job.steps ?? []).findIndex((s) => s.run === adapter.script && s["working-directory"] === undefined) : -1;
    const wrapper = wrapperIndex >= 0;
    const condition = typeof job.if === "string" ? job.if.trim().replace(/^\$\{\{\s*|\s*\}\}$/g, "") : job.if;
    const changesOnly = job.needs === "changes" || (Array.isArray(job.needs) && job.needs.length === 1 && job.needs[0] === "changes");
    const adaptedCondition = wrapper && changesOnly && condition === adapter.condition;
    let unsupported = doc.env !== undefined || doc.defaults !== undefined || job.env !== undefined || job.defaults !== undefined ||
      job.strategy !== undefined || job.container !== undefined || job.services !== undefined || job["continue-on-error"] !== undefined ||
      ((job.if !== undefined || job.needs !== undefined) && !adaptedCondition)
      ? "workflow or job execution settings cannot be replayed faithfully"
      : null;
    for (const [i, step] of (job.steps ?? []).entries()) {
      // A full local wrapper recomputes its plan without the CI artifact or token.
      if (wrapper && i < wrapperIndex && isPlanTransfer(step)) continue;
      if (step?.if !== undefined || step?.env !== undefined || step?.shell !== undefined || step?.["continue-on-error"] !== undefined ||
        step?.run?.includes("${{") || (step?.["working-directory"] !== undefined && typeof step["working-directory"] !== "string")) {
        unsupported = `step ${i + 1} has execution settings that cannot be replayed faithfully`;
      }
      if (typeof step?.run === "string") {
        const workingDirectory = step["working-directory"];
        const script = adapter && step.run === adapter.script && workingDirectory === undefined
          ? `${adapter.script} --full` : step.run;
        steps.push({
          name: step.name ?? step.run.split("\n")[0], script,
          ...(typeof workingDirectory === "string" ? { workingDirectory } : {}),
        });
      } else if (typeof step?.uses !== "string") {
        throw new Error(`${id}: step ${i + 1} has neither run: nor uses:`);
      }
    }
    return { id, label: job.name ?? id, steps, refusal: id in RUNS_HERE ? unsupported : (REFUSED[id] ?? UNCLASSIFIED) };
  });
}
