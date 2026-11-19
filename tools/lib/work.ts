import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

export const STATES = ["backlog", "ready", "active", "verify", "done", "cancelled"] as const;
export const KINDS = ["feature", "fix", "maintenance", "documentation", "investigation"] as const;
export const AREAS = ["agent", "containment", "data", "finance", "gpui", "homes", "learning", "packs", "posts", "product", "scanning", "storage", "tooling", "transit", "website"] as const;
export type State = typeof STATES[number];
export interface Task {
  id: string; title: string; state: State; area: string; kind: string; priority: number;
  depends_on: string[]; evidence: string[]; assignee?: string; blocked_by?: string;
  issue?: string; spec?: string; target_release?: string; reason?: string;
  path: string; body: string;
}
export interface Outcome {
  id: string; title: string; summary: string; order: number; tasks: string[]; path: string; body: string;
}
export interface Release {
  version: string; title: string; tasks: string[]; required_assets: string[]; path: string; body: string;
}
export interface Receipt {
  version: string; commit: string; url: string; published_at: string;
  assets: Array<{ name: string; bytes: number; url: string }>;
}
export interface Work {
  tasks: Task[]; outcomes: Outcome[]; releases: Release[]; receipts: Receipt[];
}
const VERSION = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const ID = /^[a-z][a-z0-9-]{2,79}$/;
const TERMINAL = new Set<State>(["done", "cancelled"]);
export const isOpen = (task: Task): boolean => !TERMINAL.has(task.state);

/** Read frontmatter without rendering it as page content. Throws for malformed records. */
export function document(source: string, path: string): { meta: Record<string, unknown>; body: string } {
  const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n/.exec(source);
  if (!match) throw new Error(`${path}: expected YAML frontmatter`);
  const meta = Bun.YAML.parse(match[1]);
  if (!meta || typeof meta !== "object" || Array.isArray(meta)) throw new Error(`${path}: expected a property map`);
  return { meta: meta as Record<string, unknown>, body: source.slice(match[0].length).trim() };
}
function files(root: string, directory: string, extension: string): string[] {
  const dir = join(root, directory);
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { withFileTypes: true }).filter(e => e.isFile() && e.name.endsWith(extension) && e.name !== "index.md")
    .map(e => `${directory}/${e.name}`).sort();
}
function text(meta: Record<string, unknown>, key: string, path: string): string {
  const value = meta[key];
  if (typeof value !== "string" || !value.trim() || /[\r\n]/.test(value)) throw new Error(`${path}: ${key} must be a nonempty single line`);
  return value;
}
function list(meta: Record<string, unknown>, key: string, path: string, required = false): string[] {
  const value = meta[key];
  if (value === undefined && !required) return [];
  if (!Array.isArray(value) || value.some(x => typeof x !== "string" || !x.trim()) || new Set(value).size !== value.length || (required && !value.length)) {
    throw new Error(`${path}: ${key} must be a ${required ? "nonempty " : ""}list of unique strings`);
  }
  return value as string[];
}
function keys(meta: Record<string, unknown>, allowed: string[], path: string): void {
  for (const key of Object.keys(meta)) if (!allowed.includes(key)) throw new Error(`${path}: unknown field ${key}`);
}
function integer(meta: Record<string, unknown>, key: string, path: string): number {
  const value = meta[key];
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) throw new Error(`${path}: ${key} must be a nonnegative integer`);
  return value;
}
function id(meta: Record<string, unknown>, key: string, path: string): string {
  const value = text(meta, key, path);
  if (!ID.test(value)) throw new Error(`${path}: invalid ${key}`);
  return value;
}
function section(body: string, heading: string): string {
  return new RegExp(`^## ${heading}\\s*\\n([\\s\\S]*?)(?=^## |$(?![\\s\\S]))`, "m").exec(body)?.[1]?.trim() ?? "";
}

/** Load and validate the canonical work graph. No network or generated state is consulted. */
export function loadWork(root: string): Work {
  const tasks: Task[] = files(root, "docs/tasks", ".md").map(path => {
    const { meta, body } = document(readFileSync(join(root, path), "utf8"), path);
    keys(meta, ["id", "title", "state", "area", "kind", "priority", "depends_on", "evidence", "assignee", "blocked_by", "issue", "spec", "target_release", "reason"], path);
    const state = text(meta, "state", path) as State;
    const area = text(meta, "area", path), kind = text(meta, "kind", path);
    if (!STATES.includes(state)) throw new Error(`${path}: unknown state ${state}`);
    if (!(AREAS as readonly string[]).includes(area)) throw new Error(`${path}: unknown area ${area}`);
    if (!(KINDS as readonly string[]).includes(kind)) throw new Error(`${path}: unknown kind ${kind}`);
    const task: Task = { id: id(meta, "id", path), title: text(meta, "title", path), state, area, kind,
      priority: integer(meta, "priority", path), depends_on: list(meta, "depends_on", path), evidence: list(meta, "evidence", path), path, body };
    if (task.priority > 4) throw new Error(`${path}: priority must be 0–4`);
    for (const key of ["assignee", "blocked_by", "issue", "spec", "target_release", "reason"] as const) {
      if (meta[key] !== undefined) task[key] = text(meta, key, path);
    }
    if (!section(body, "Outcome") || !/^[-*] \S/m.test(section(body, "Acceptance"))) throw new Error(`${path}: Outcome and Acceptance checks are required`);
    if (state === "active" && !task.assignee) throw new Error(`${path}: active work needs an assignee`);
    if (state === "done" && !task.evidence.length) throw new Error(`${path}: done work needs acceptance evidence`);
    if (state === "done" && task.blocked_by) throw new Error(`${path}: blocked work cannot be done`);
    if (state !== "backlog" && body.includes("Replace this draft check with an observable pass condition.")) throw new Error(`${path}: define acceptance before selecting a draft task`);
    if (state === "cancelled" && !task.reason) throw new Error(`${path}: cancelled work needs a reason`);
    if (task.target_release && !VERSION.test(task.target_release)) throw new Error(`${path}: invalid target_release`);
    if (task.issue && !/^https:\/\/github\.com\/larsboes\/Sjel\/issues\/[1-9]\d*$/.test(task.issue)) throw new Error(`${path}: issue must link to public Sjel intake`);
    if (task.spec && (!task.spec.startsWith("docs/") || task.spec.split("/").includes("..") || !existsSync(join(root, task.spec.split("#")[0])))) throw new Error(`${path}: spec must name an existing docs path`);
    return task;
  });
  const outcomes: Outcome[] = files(root, "docs/roadmap", ".md").map(path => {
    const { meta, body } = document(readFileSync(join(root, path), "utf8"), path);
    keys(meta, ["id", "title", "summary", "order", "tasks"], path);
    return { id: id(meta, "id", path), title: text(meta, "title", path), summary: text(meta, "summary", path), order: integer(meta, "order", path), tasks: list(meta, "tasks", path, true), path, body };
  });
  const releases: Release[] = files(root, "docs/releases", ".md").map(path => {
    const { meta, body } = document(readFileSync(join(root, path), "utf8"), path);
    keys(meta, ["version", "title", "tasks", "required_assets"], path);
    const version = text(meta, "version", path);
    if (!VERSION.test(version) || path !== `docs/releases/${version}.md`) throw new Error(`${path}: filename must match vMAJOR.MINOR.PATCH`);
    const required_assets = list(meta, "required_assets", path, true);
    if (required_assets.some(name => !/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(name))) throw new Error(`${path}: invalid asset name`);
    return { version, title: text(meta, "title", path), tasks: list(meta, "tasks", path, true), required_assets, path, body };
  });
  const receipts: Receipt[] = files(root, "docs/releases/receipts", ".json").map(path => {
    const receipt = JSON.parse(readFileSync(join(root, path), "utf8")) as Receipt;
    validateReceipt(receipt, path);
    if (path !== `docs/releases/receipts/${receipt.version}.json`) throw new Error(`${path}: receipt filename differs from version`);
    return receipt;
  });
  const graph = { tasks, outcomes, releases, receipts };
  validateGraph(graph);
  return graph;
}

/** Validate a saved public publication receipt. Throws if its identity or asset metadata is invalid. */
export function validateReceipt(receipt: Receipt, path = "receipt"): void {
  if (!receipt || !VERSION.test(receipt.version) || !/^[0-9a-f]{40}$/.test(receipt.commit)
    || receipt.url !== `https://github.com/larsboes/Sjel/releases/tag/${receipt.version}`
    || typeof receipt.published_at !== "string" || !/^\d{4}-\d\d-\d\dT/.test(receipt.published_at) || !Number.isFinite(Date.parse(receipt.published_at))
    || !Array.isArray(receipt.assets) || !receipt.assets.length) throw new Error(`${path}: invalid publication receipt`);
  const seen = new Set<string>();
  for (const asset of receipt.assets) {
    if (!asset || !/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(asset.name) || !Number.isSafeInteger(asset.bytes) || asset.bytes < 0
      || asset.url !== `https://github.com/larsboes/Sjel/releases/download/${receipt.version}/${asset.name}` || seen.has(asset.name)) throw new Error(`${path}: invalid or duplicate release asset`);
    seen.add(asset.name);
  }
}
function unique<T>(records: T[], get: (r: T) => string): void {
  const seen = new Set<string>();
  for (const record of records) { const key = get(record); if (seen.has(key)) throw new Error(`duplicate record: ${key}`); seen.add(key); }
}
function validateGraph(work: Work): void {
  unique([...work.tasks, ...work.outcomes], x => x.id);
  unique(work.releases, x => x.version);
  unique(work.receipts, x => x.version);
  const tasks = new Map(work.tasks.map(t => [t.id, t]));
  const visiting = new Set<string>(), visited = new Set<string>();
  const visit = (task: Task) => {
    if (visiting.has(task.id)) throw new Error(`${task.path}: dependency cycle`);
    if (visited.has(task.id)) return;
    visiting.add(task.id);
    for (const dependency of task.depends_on) {
      const other = tasks.get(dependency);
      if (!other) throw new Error(`${task.path}: missing dependency ${dependency}`);
      if (["ready", "done"].includes(task.state) && other.state !== "done") throw new Error(`${task.path}: ${task.state} task depends on unfinished ${dependency}`);
      visit(other);
    }
    visiting.delete(task.id); visited.add(task.id);
  };
  work.tasks.forEach(visit);
  const delivered = new Set<string>();
  for (const record of [...work.outcomes, ...work.releases]) {
    for (const id of record.tasks) if (!tasks.has(id)) throw new Error(`${record.path}: missing task ${id}`);
  }
  for (const receipt of work.receipts) {
    const release = work.releases.find(r => r.version === receipt.version);
    if (!release) throw new Error(`${receipt.version}: receipt has no release record`);
    for (const name of release.required_assets) if (!receipt.assets.some(a => a.name === name)) throw new Error(`${receipt.version}: required asset absent: ${name}`);
    for (const id of release.tasks) {
      if (delivered.has(id)) throw new Error(`${receipt.version}: task ${id} delivered more than once`);
      delivered.add(id);
    }
  }
}

/** Derive roadmap state from tasks; cancelled work never counts as delivered. */
export function outcomeState(work: Work, outcome: Outcome): "active" | "planned" | "awaiting-release" | "delivered" | "cancelled" {
  const tasks = outcome.tasks.map(id => work.tasks.find(t => t.id === id)!);
  if (tasks.every(t => t.state === "cancelled")) return "cancelled";
  const delivered = new Set(work.releases.filter(r => work.receipts.some(p => p.version === r.version)).flatMap(r => r.tasks));
  if (tasks.every(t => t.state === "done") && outcome.tasks.every(id => delivered.has(id))) return "delivered";
  if (tasks.every(t => t.state === "done")) return "awaiting-release";
  if (tasks.some(t => ["active", "verify"].includes(t.state))) return "active";
  return "planned";
}

/** Inventory every heading section of existing plans without interpreting prose as completed work. */
export function inventory(root: string): Array<{ source: string; heading: string; line: number; text: string }> {
  const sources = ["PLAN.md", ...files(root, "docs/plans", ".md")];
  return sources.filter(path => existsSync(join(root, path))).flatMap(source => {
    const lines = readFileSync(join(root, source), "utf8").split("\n");
    const sections: Array<{ source: string; heading: string; line: number; text: string }> = [];
    let fence = "";
    for (let i = 0; i < lines.length; i++) {
      const marker = /^\s*(`{3,}|~{3,})/.exec(lines[i]);
      if (marker) { if (!fence) fence = marker[1][0]; else if (marker[1][0] === fence) fence = ""; }
      if (!fence && /^#{1,6} /.test(lines[i])) sections.push({ source, heading: lines[i].replace(/^#+ /, ""), line: i + 1, text: "" });
      if (!sections.length) sections.push({ source, heading: "Preamble", line: 1, text: "" });
      sections.at(-1)!.text += lines[i] + "\n";
    }
    return sections;
  });
}
