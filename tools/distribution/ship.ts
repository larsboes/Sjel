#!/usr/bin/env bun
// Resumable release run for a pushed tag: clean worktree, CLI workflow artifacts, signed Mac DMG,
// notarization, TestFlight, draft release, publication and receipt. Each run does what it can and
// records it; a step that waits on GitHub or Apple reports pending instead of holding the terminal.
// tools/release owns the tag. Publication needs --publish.
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { notarize, PENDING_EXIT } from "./notarize.ts";

const root = resolve(import.meta.dir, "../..");
const CLI_WORKFLOW = "cli-distribution.yml";
const CLI_ARTIFACTS = ["sjel-cli-macos-15", "sjel-cli-ubuntu-24.04", "sjel-cli-ubuntu-24.04-arm"];

export const STEPS = ["worktree", "cli", "macos", "notarize", "testflight", "draft", "publish", "receipt"] as const;
export type Step = typeof STEPS[number];

export interface State {
  version: string;
  commit: string;
  identity: string;
  profile: string;
  cliRun?: number;
  done: Partial<Record<Step, string>>;
}

export interface Options {
  version: string;
  identity: string;
  profile: string;
  publish: boolean;
  skip: Step[];
  home: string;
}

export function parse(args: string[], home = process.env.SJEL_RELEASE_HOME || join(homedir(), "Library/Caches/Sjel/releases")): Options {
  const version = args.shift() ?? "";
  if (!/^v\d+\.\d+\.\d+$/.test(version)) throw new Error("usage: ship.ts vX.Y.Z [--identity ID] [--notary-profile NAME] [--skip STEP] [--publish]");
  const options: Options = { version, identity: "", profile: "", publish: false, skip: [], home };
  while (args.length) {
    const flag = args.shift()!;
    if (flag === "--publish") { options.publish = true; continue; }
    const value = args.shift();
    if (!value) throw new Error(`incomplete option: ${flag}`);
    if (flag === "--identity") options.identity = value;
    else if (flag === "--notary-profile") options.profile = value;
    else if (flag === "--skip") {
      if (!(STEPS as readonly string[]).includes(value) || value === "publish") throw new Error(`cannot skip: ${value}`);
      options.skip.push(value as Step);
    } else throw new Error(`unknown option: ${flag}`);
  }
  if (options.identity && !options.identity.startsWith("Developer ID Application:")) throw new Error("--identity must be a Developer ID Application identity");
  return options;
}

function capture(argv: string[], cwd = root) {
  const child = Bun.spawnSync(argv, { cwd, stdout: "pipe", stderr: "pipe" });
  if (child.exitCode !== 0) throw new Error(`${argv.slice(0, 3).join(" ")} failed: ${child.stderr.toString().trim()}`);
  return child.stdout.toString().trim();
}

function run(argv: string[], cwd = root, env: Record<string, string> = {}) {
  const child = Bun.spawnSync(argv, { cwd, stdout: "inherit", stderr: "inherit", env: { ...process.env, ...env } });
  if (child.exitCode !== 0) throw new Error(`${argv.slice(0, 3).join(" ")} failed (${child.exitCode})`);
}

type Result = "done" | { pending: string };

function step(state: State, name: Step, save: () => void, body: () => Result) {
  if (state.done[name]) return true;
  const result = body();
  if (result === "done") {
    state.done[name] = new Date().toISOString();
    save();
    console.log(`done     ${name}`);
    return true;
  }
  console.log(`pending  ${name}: ${result.pending}`);
  return false;
}

function cliStep(state: State, dir: string, save: () => void): Result {
  const out = join(dir, "cli");
  if (state.cliRun === undefined) {
    const runs = JSON.parse(capture(["gh", "run", "list", "--workflow", CLI_WORKFLOW, "--event", "workflow_dispatch",
      "--commit", state.commit, "--json", "databaseId,conclusion,status", "-L", "20"])) as Array<{ databaseId: number; conclusion: string; status: string }>;
    const usable = runs.find((r) => r.conclusion === "success") ?? runs.find((r) => r.status !== "completed");
    if (usable) state.cliRun = usable.databaseId;
    else {
      run(["gh", "workflow", "run", CLI_WORKFLOW, "--ref", state.version, "-f", `version=${state.version}`]);
      return { pending: `dispatched ${CLI_WORKFLOW} on ${state.version}; rerun to pick up the run` };
    }
    save();
  }
  const view = JSON.parse(capture(["gh", "run", "view", String(state.cliRun), "--json", "status,conclusion,url"]));
  if (view.status !== "completed") return { pending: `CLI run ${state.cliRun} is ${view.status}: ${view.url}` };
  if (view.conclusion !== "success") {
    const failed = state.cliRun;
    delete state.cliRun;
    save();
    throw new Error(`CLI run ${failed} ended as ${view.conclusion}; rerun ship.ts to dispatch a new one`);
  }
  rmSync(out, { recursive: true, force: true });
  run(["gh", "run", "download", String(state.cliRun), "--dir", out, ...CLI_ARTIFACTS.flatMap((name) => ["--name", name])]);
  for (const name of CLI_ARTIFACTS) {
    const catalogue = JSON.parse(readFileSync(join(out, name, "release.json"), "utf8"));
    if (catalogue.version !== state.version || catalogue.source?.commit !== state.commit) {
      throw new Error(`${name} does not describe ${state.version} at ${state.commit}`);
    }
  }
  return "done";
}

export function ship(options: Options) {
  const dir = join(options.home, options.version);
  mkdirSync(dir, { recursive: true });
  const statePath = join(dir, "state.json");
  const commit = capture(["git", "rev-list", "-n", "1", options.version]);
  const remote = capture(["git", "ls-remote", "origin", `refs/tags/${options.version}^{}`]).split(/\s/)[0]
    || capture(["git", "ls-remote", "origin", `refs/tags/${options.version}`]).split(/\s/)[0];
  if (remote !== commit) throw new Error(`${options.version} is not pushed to origin at ${commit.slice(0, 8)}; push it with tools/release`);
  const state: State = existsSync(statePath)
    ? JSON.parse(readFileSync(statePath, "utf8"))
    : { version: options.version, commit, identity: "", profile: "", done: {} };
  if (state.commit !== commit) throw new Error(`saved state is for ${state.commit.slice(0, 8)}, the tag is ${commit.slice(0, 8)}`);
  state.identity = options.identity || state.identity;
  state.profile = options.profile || state.profile;
  const save = () => writeFileSync(statePath, JSON.stringify(state, null, 2) + "\n");
  for (const name of options.skip) state.done[name] ??= `skipped ${new Date().toISOString()}`;
  save();
  console.log(`release  ${options.version} at ${commit.slice(0, 8)}; state in ${dir}`);

  const src = join(dir, "src");
  const macos = join(dir, "macos");
  step(state, "worktree", save, () => {
    if (!existsSync(src)) run(["git", "worktree", "add", "--detach", src, options.version]);
    if (capture(["git", "rev-parse", "HEAD"], src) !== commit) throw new Error(`${src} is not at ${options.version}`);
    if (capture(["git", "-c", "core.fsmonitor=false", "status", "--porcelain"], src)) throw new Error(`${src} has local changes`);
    return "done";
  });

  const cli = step(state, "cli", save, () => cliStep(state, dir, save));
  step(state, "macos", save, () => {
    if (!state.identity) throw new Error("the first run needs --identity 'Developer ID Application: …'");
    if (existsSync(macos) && !existsSync(join(macos, "release.json"))) rmSync(macos, { recursive: true, force: true });
    if (!existsSync(macos)) {
      const target = join(homedir(), "Library/Caches/Sjel/macos-release-target");
      run(["bun", "tools/distribution/prepare.ts", "macos", "--version", options.version, "--out", macos,
        "--build", "--target-dir", target, "--identity", state.identity], src);
    }
    return "done";
  });
  const mac = step(state, "notarize", save, () => {
    if (!state.profile) throw new Error("the first run needs --notary-profile NAME");
    return notarize(macos, state.identity, state.profile) === "accepted" ? "done" : { pending: "Apple is processing the DMG" };
  });
  step(state, "testflight", save, () => {
    run([join(root, "sjel"), "testflight", "ship", "--source", src]);
    return "done";
  });
  if (!cli || !mac) return PENDING_EXIT;

  step(state, "draft", save, () => {
    const existing = Bun.spawnSync(["gh", "release", "view", options.version, "--json", "isDraft"], { cwd: root, stdout: "pipe", stderr: "pipe" });
    if (existing.exitCode === 0) return "done";
    const includes = readdirSync(join(dir, "cli")).sort().flatMap((name) => ["--include", join(dir, "cli", name)]);
    run(["bun", "tools/distribution/publish-draft.ts", "--dir", macos, ...includes, "--confirm-draft"]);
    return "done";
  });
  const published = step(state, "publish", save, () => {
    const view = JSON.parse(capture(["gh", "release", "view", options.version, "--json", "isDraft,url"]));
    if (!view.isDraft) return "done";
    if (!options.publish) return { pending: `review the draft at ${view.url}, then rerun with --publish` };
    run(["gh", "release", "edit", options.version, "--draft=false", "--latest"]);
    return "done";
  });
  if (!published) return PENDING_EXIT;
  step(state, "receipt", save, () => {
    const path = join(root, "docs/releases/receipts", `${options.version}.json`);
    if (!existsSync(path)) run(["tools/work", "receipt", options.version]);
    console.log(`commit ${path} with the release task's evidence`);
    return "done";
  });
  return 0;
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  if (!args.length || args.includes("--help")) {
    console.log("bun tools/distribution/ship.ts vX.Y.Z [--identity 'Developer ID Application: …'] [--notary-profile NAME] [--skip STEP] [--publish]");
    console.log(`Steps: ${STEPS.join(", ")}. Exit ${PENDING_EXIT}: a step waits on GitHub, Apple or review; run again later.`);
    process.exit(args.length ? 0 : 2);
  }
  try {
    process.exitCode = ship(parse(args));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
