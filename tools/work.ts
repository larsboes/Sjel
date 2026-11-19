#!/usr/bin/env bun
import { randomUUID } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { AREAS, STATES, document, inventory, isOpen, loadWork, validateReceipt, type Receipt, type Release } from "./lib/work.ts";

const ROOT = resolve(import.meta.dir, "..");
export const USAGE = `tools/work <command>
  list [--area <area>] [--state <state>] [--all] [--json]
                             Tasks in priority order (open by default)
  show <id>                  Task, outcome or release source
  check                      Validate task graph and publication receipts offline
  inventory [--json]         Inventory unmigrated planning sections; no writes
  new <area> <title>         Create a backlog task with a collision-resistant ID
  notes <version>            Render accepted release scope and upgrade notes
  release-check <version> [ref]  Check acceptance at an exact source ref (default HEAD)
  reconcile <version>        Compare saved receipt with GitHub publication; no writes
  receipt <version>          Record verified GitHub publication locally; never publish
  render --out <directory>   Build static roadmap, delivery history and release views
No command creates a Git tag, publishes a release or modifies a GitHub issue.`;

function git(root: string, args: string[]): string {
  const result = Bun.spawnSync(["git", ...args], { cwd: root, stdout: "pipe", stderr: "pipe" });
  if (result.exitCode !== 0) throw new Error(`git ${args[0]} failed: ${result.stderr.toString().trim()}`);
  return result.stdout.toString().trim();
}
function gh(root: string, args: string[]): unknown {
  const result = Bun.spawnSync(["gh", ...args], { cwd: root, stdout: "pipe", stderr: "pipe" });
  if (result.exitCode !== 0) throw new Error(`GitHub verification unavailable: ${result.stderr.toString().trim()}`);
  return JSON.parse(result.stdout.toString());
}
function release(root: string, version: string): Release {
  const found = loadWork(root).releases.find(r => r.version === version);
  if (!found) throw new Error(`no release record for ${version}`);
  return found;
}

/** Require each selected task to have passed acceptance in the source being released. */
export function checkRelease(root: string, version: string, ref = "HEAD"): string {
  const selected = release(root, version);
  const commit = git(root, ["rev-parse", "--verify", `${ref}^{commit}`]);
  const snapshot = document(git(root, ["show", `${commit}:${selected.path}`]) + "\n", selected.path);
  if (JSON.stringify(snapshot.meta.tasks) !== JSON.stringify(selected.tasks) || JSON.stringify(snapshot.meta.required_assets) !== JSON.stringify(selected.required_assets)) {
    throw new Error(`${version}: release scope differs from ${commit}`);
  }
  const work = loadWork(root);
  for (const id of selected.tasks) {
    const task = work.tasks.find(t => t.id === id)!;
    const original = document(git(root, ["show", `${commit}:${task.path}`]) + "\n", task.path);
    if (original.meta.id !== id || original.meta.state !== "done" || !Array.isArray(original.meta.evidence) || !original.meta.evidence.length
      || original.meta.evidence.some(item => typeof item !== "string" || !item.trim())
      || !/^## Outcome\s*\n\S/m.test(original.body.replaceAll("\n\n", "\n"))
      || !/^## Acceptance\s*\n[-*] \S/m.test(original.body.replaceAll("\n\n", "\n"))) {
      throw new Error(`${version}: ${id} has not passed acceptance at ${commit}`);
    }
    if (work.releases.some(r => r.version !== version && r.tasks.includes(id) && work.receipts.some(p => p.version === r.version))) {
      throw new Error(`${version}: ${id} was already delivered`);
    }
  }
  return commit;
}

/** Read a published release and its exact tag; a draft or failed request never becomes a receipt. */
export function publication(root: string, version: string, request = gh): Receipt {
  const selected = release(root, version);
  const remote = request(root, ["api", `repos/larsboes/Sjel/releases/tags/${version}`]) as {
    draft: boolean; tag_name: string; published_at: string; html_url: string;
    assets: Array<{ name: string; size: number; browser_download_url: string }>;
  };
  if (remote.draft !== false || remote.tag_name !== version || !remote.published_at) throw new Error(`${version}: not a published release`);
  const commit = checkRelease(root, version, version);
  const remoteTag = request(root, ["api", `repos/larsboes/Sjel/git/ref/tags/${version}`]) as { object: { type: string; sha: string } };
  let object = remoteTag.object;
  for (let depth = 0; object.type === "tag" && depth < 5; depth++) {
    object = (request(root, ["api", `repos/larsboes/Sjel/git/tags/${object.sha}`]) as { object: typeof object }).object;
  }
  if (object.type !== "commit" || object.sha !== commit) throw new Error(`${version}: local and remote tag identity differ`);
  const receipt: Receipt = { version, commit, url: remote.html_url, published_at: remote.published_at,
    assets: remote.assets.map(a => ({ name: a.name, bytes: a.size, url: a.browser_download_url })).sort((a, b) => a.name.localeCompare(b.name)) };
  validateReceipt(receipt);
  for (const name of selected.required_assets) if (!receipt.assets.some(a => a.name === name)) throw new Error(`${version}: missing required asset ${name}`);
  return receipt;
}

/** Execute local planning operations. Errors are reported by the process boundary. */
export async function run(args: string[], root = ROOT): Promise<string> {
  const [command = "list", ...rest] = args;
  if (["--help", "-h", "help"].includes(command)) return USAGE;
  if (command === "inventory") {
    if (rest.some(arg => arg !== "--json")) throw new Error(USAGE);
    const sections = inventory(root);
    return rest.includes("--json") ? JSON.stringify(sections, null, 2) : sections.map(s => `${s.source}:${s.line}\t${s.heading}`).join("\n");
  }
  if (command === "new") {
    const [area, ...words] = rest;
    if (!(AREAS as readonly string[]).includes(area) || !words.length) throw new Error(`new requires an area (${AREAS.join(", ")}) and a title`);
    const title = words.join(" ").trim();
    if (!title || /[\r\n]/.test(title)) throw new Error("title must be a nonempty single line");
    const id = `task-${randomUUID()}`;
    const directory = join(root, "docs/tasks");
    mkdirSync(directory, { recursive: true });
    const path = join(directory, `${id}.md`);
    writeFileSync(path, `---\nid: ${id}\ntitle: ${JSON.stringify(title)}\nstate: backlog\narea: ${area}\nkind: maintenance\npriority: 1\n---\n\n# ${title}\n\n## Outcome\n\nDefine the observable result before selecting this task.\n\n## Acceptance\n\n- Replace this draft check with an observable pass condition.\n`, { flag: "wx" });
    return `Created docs/tasks/${id}.md (backlog; define acceptance before ready).`;
  }
  const work = loadWork(root);
  if (command === "list") {
    const { values: { all, json, area, state } } = parseArgs({ args: rest, options: {
      all: { type: "boolean" }, json: { type: "boolean" }, area: { type: "string" }, state: { type: "string" },
    } });
    if (area !== undefined && !AREAS.some(candidate => candidate === area)) throw new Error(`Unknown area '${area}'. Choose from ${AREAS.join(", ")}.`);
    if (state !== undefined && !STATES.some(candidate => candidate === state)) throw new Error(`Unknown state '${state}'. Choose from ${STATES.join(", ")}.`);
    const tasks = work.tasks.filter(task => (area === undefined || task.area === area)
      && (state === undefined ? all || isOpen(task) : task.state === state))
      .sort((a, b) => a.priority - b.priority || a.id.localeCompare(b.id));
    if (json) return JSON.stringify(tasks, null, 2);
    if (!tasks.length) return area !== undefined || state !== undefined || all ? "No matching tasks." : "No open tasks.";
    return tasks.map(t => `P${t.priority}\t${t.state}${t.blocked_by ? " (blocked)" : ""}\t${t.id}\t${t.title}`).join("\n");
  }
  if (command === "show" && rest.length === 1) {
    const record = [...work.tasks, ...work.outcomes, ...work.releases].find(r => ("id" in r ? r.id : r.version) === rest[0]);
    if (!record) throw new Error(`unknown record ${rest[0]}`);
    return readFileSync(join(root, record.path), "utf8");
  }
  if (command === "check" && !rest.length) {
    for (const receipt of work.receipts) {
      const actual = checkRelease(root, receipt.version, receipt.version);
      if (actual !== receipt.commit) throw new Error(`${receipt.version}: receipt commit differs from tag`);
    }
    return `Work check passed: ${work.tasks.length} tasks, ${work.outcomes.length} outcomes, ${work.releases.length} release records, ${work.receipts.length} publication receipts.`;
  }
  if (command === "notes" && rest.length === 1) {
    const selected = release(root, rest[0]);
    for (const id of selected.tasks) if (work.tasks.find(t => t.id === id)?.state !== "done") throw new Error(`${rest[0]}: ${id} is not accepted`);
    return `${selected.body}\n\n## Accepted work\n\n${selected.tasks.map(id => { const task = work.tasks.find(t => t.id === id)!; return `- ${task.title} (${id})`; }).join("\n")}\n`;
  }
  if (command === "release-check" && rest.length >= 1 && rest.length <= 2) return `Release acceptance passed at ${checkRelease(root, rest[0], rest[1])}.`;
  if (["receipt", "reconcile"].includes(command) && rest.length === 1) {
    const receipt = publication(root, rest[0]);
    const path = join(root, `docs/releases/receipts/${receipt.version}.json`);
    if (command === "reconcile" || existsSync(path)) {
      const saved = work.receipts.find(r => r.version === receipt.version);
      if (!saved || JSON.stringify(saved) !== JSON.stringify(receipt)) throw new Error(`${receipt.version}: publication differs from saved receipt`);
      return `${receipt.version}: publication matches its saved receipt.`;
    }
    mkdirSync(join(root, "docs/releases/receipts"), { recursive: true });
    writeFileSync(path, JSON.stringify(receipt, null, 2) + "\n", { flag: "wx" });
    return `Recorded ${receipt.version}; review and commit the receipt to update delivered history.`;
  }
  if (command === "render" && rest.length === 2 && rest[0] === "--out") {
    const { build } = await import("../website/scripts/roadmap.ts");
    build(resolve(rest[1]), root);
    return "Rendered roadmap, delivered history and releases.";
  }
  throw new Error(USAGE);
}

if (import.meta.main) run(process.argv.slice(2)).then(output => console.log(output)).catch(error => {
  console.error(`work: ${error instanceof Error ? error.message : String(error)}`); process.exitCode = 1;
});
