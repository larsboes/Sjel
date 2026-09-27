#!/usr/bin/env bun
// collect-signals.ts — the deterministic half of suggest-skills.
//
// It gathers and counts. It never judges. Two runs over the same files return
// the same corpus, so a proposal can be argued with rather than trusted, and the
// model that reads this output is arguing with numbers instead of impressions.
//
// Everything it reads is local: the harness prompt history, and the skills already
// installed. Nothing leaves the machine and nothing is written.
//
// Usage:
//   bun collect-signals.ts [--days N] [--history PATH] [--skills DIR]... [--packs DIR]... [--json]
//
// Defaults: 30 days, $HOME/.claude/history.jsonl, $HOME/.claude/skills, and the
// Axon Packs root when $SJEL_ROOT is set.

import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { basename, join } from "node:path";

type Prompt = { at: number; project: string; session: string; text: string };
let dominant: { token: string; prompts: number }[] = [];

type Cluster = {
  tokens: string[];
  prompts: number;
  sessions: number;
  projects: string[];
  with: string[];
  spanDays: number;
  samples: string[];
};

const STOPWORDS = new Set(
  ("a an the and or but if then than that this these those it its is are was were be been being do does did done" +
    " you your i me my we our they them he she of in on at to for from with without into over under again more most" +
    " can could should would will just now not no yes what which who whom how why when where all any some each other" +
    " use used using make makes made get gets got let lets please thanks ok okay pls also still there here so as by up" +
    " out down off about like want need see look read write run runs ran new old one two have has had having with" +
    " when what will would could should been some more into over only very much many also need want know think tell" +
    " said says say sure good best work works working help maybe well back take give keep find look right thing" +
    " things stuff first last next same because before after both else even ever such dont doesnt cant" +
    " really actually maybe probably something anything everything nothing somebody anyone everyone please thanks" +
    " through continue currently bring wanna gonna whats thats heres lets pretty quite rather always never" +
    " everything already almost enough instead least might must shall since until while whether above below" +
    " done doing goes going came come went make making made start started stop stopped better worse full" +
    " sure yeah yep nope okay fine great nice cool thanks thank hello hey")
    .split(/\s+/),
);

// A prompt that repairs a previous one. The marker is a signal to READ the pair,
// never a verdict on its own — "still" appears in plenty of calm sentences.
const FRICTION = [
  /\bstill (not|doesn'?t|does not|broken|failing|fails|wrong|missing)\b/i,
  /\b(doesn'?t|does not|didn'?t|did not) work\b/i,
  /\b(that'?s|thats|this is) (wrong|not right|not what)\b/i,
  /\bnot what i (asked|meant|wanted)\b/i,
  /\b(again|another) (time|try)\b/i,
  /^(no|nope|nah)[,.! ]/i,
  /\bwhy (is|does|did|are) (it|this|that|you)\b/i,
  /\b(broken|failing|failed) again\b/i,
  /\bi (already|just) (said|told|asked)\b/i,
  /\bfix (it|this|that)\b/i,
];

function arg(name: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i === -1 ? undefined : process.argv[i + 1];
}
function args(name: string): string[] {
  const out: string[] = [];
  process.argv.forEach((a, i) => {
    if (a === `--${name}` && process.argv[i + 1]) out.push(process.argv[i + 1]);
  });
  return out;
}

const warnings: string[] = [];
const home = process.env.HOME ?? "";
const days = Number(arg("days") ?? 30);
if (!Number.isFinite(days) || days <= 0) {
  console.error("collect-signals: --days needs a positive number");
  process.exit(2);
}
const historyPath = arg("history") ?? join(home, ".claude", "history.jsonl");
const skillDirs = args("skills").length ? args("skills") : [join(home, ".claude", "skills")];
const packRoots = args("packs").length
  ? args("packs")
  : process.env.SJEL_ROOT
    ? [join(process.env.SJEL_ROOT, "Packs")]
    : [];

function readPrompts(): Prompt[] {
  if (!existsSync(historyPath)) {
    warnings.push(`no prompt history at ${historyPath}; every count below is zero`);
    return [];
  }
  const out: Prompt[] = [];
  let malformed = 0;
  for (const line of readFileSync(historyPath, "utf8").split("\n")) {
    if (!line.trim()) continue;
    try {
      const row = JSON.parse(line) as Record<string, unknown>;
      const text = typeof row.display === "string" ? row.display : "";
      const at = typeof row.timestamp === "number" ? row.timestamp : 0;
      if (!text || !at) { malformed++; continue; }
      out.push({
        at,
        project: typeof row.project === "string" ? row.project : "unknown",
        session: typeof row.sessionId === "string" ? row.sessionId : "unknown",
        text,
      });
    } catch { malformed++; }
  }
  if (malformed) warnings.push(`${malformed} history lines were unreadable and are excluded`);
  return out;
}

/** Harness artifacts, not things the user typed. Left in, they cluster into "pasted text lines". */
function stripArtifacts(text: string): string {
  return text
    .replace(/\[(Pasted text|Image|Pasted content)[^\]]*\]/gi, " ")
    .replace(/\[Request interrupted[^\]]*\]/gi, " ");
}

function tokenize(text: string): string[] {
  const words = stripArtifacts(text)
    .toLowerCase()
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/[^a-z0-9/_-]+/g, " ")
    .split(/\s+/)
    .filter((w) => w.length >= 4 && !STOPWORDS.has(w) && !/^\d+$/.test(w));
  return [...new Set(words)];
}

function cluster(prompts: Prompt[]): Cluster[] {
  const byToken = new Map<string, Set<number>>();
  prompts.forEach((p, i) => {
    for (const t of tokenize(p.text)) {
      if (!byToken.has(t)) byToken.set(t, new Set());
      byToken.get(t)!.add(i);
    }
  });
  // A token in more than one prompt in twelve names the corpus, not a topic inside
  // it: the repository, the harness, the word this user says in every sentence.
  // Those are reported separately as `dominant` rather than clustered, because a
  // cluster of them is just the corpus with extra steps. The threshold is a guess
  // that survived one real corpus (530 prompts, 16 days) and is the first thing to
  // re-tune if the clusters come back generic.
  const ceiling = Math.max(3, Math.floor(prompts.length * 0.08));
  const entries = [...byToken.entries()];
  dominant = entries
    .filter(([, set]) => set.size > ceiling)
    .sort((a, b) => b[1].size - a[1].size)
    .slice(0, 12)
    .map(([token, set]) => ({ token, prompts: set.size }));
  const seeds = entries
    .filter(([, set]) => set.size >= 3 && set.size <= ceiling)
    .sort((a, b) => b[1].size - a[1].size);

  // Merge seeds whose prompt sets are mostly the same set: two words for one topic.
  const merged: { tokens: string[]; set: Set<number> }[] = [];
  for (const [token, set] of seeds) {
    const hit = merged.find((m) => jaccard(m.set, set) >= 0.6);
    if (hit) {
      hit.tokens.push(token);
      for (const i of set) hit.set.add(i);
    } else {
      merged.push({ tokens: [token], set: new Set(set) });
    }
  }

  return merged
    .map((m) => {
      const rows = [...m.set].map((i) => prompts[i]);
      const sessions = new Set(rows.map((r) => r.session));
      const times = rows.map((r) => r.at);
      // What else these prompts talk about. A seed token alone is often a generic
      // verb that survived the stoplist; the company it keeps is what names the topic.
      const near = new Map<string, number>();
      for (const row of rows) {
        for (const t of tokenize(row.text)) {
          if (m.tokens.includes(t)) continue;
          near.set(t, (near.get(t) ?? 0) + 1);
        }
      }
      return {
        tokens: m.tokens.slice(0, 6),
        with: [...near.entries()]
          .filter(([, n]) => n >= Math.max(2, Math.ceil(rows.length * 0.25)))
          .sort((a, b) => b[1] - a[1])
          .slice(0, 6)
          .map(([t, n]) => `${t}(${n})`),
        prompts: rows.length,
        sessions: sessions.size,
        projects: [...new Set(rows.map((r) => basename(r.project)))].slice(0, 5),
        spanDays: Math.round((Math.max(...times) - Math.min(...times)) / 86_400_000),
        samples: rows.slice(0, 3).map((r) => truncate(r.text)),
      };
    })
    // A topic that recurs across sessions is a candidate. One long session about
    // one thing is a task, and a skill built from it fires once and never again.
    .filter((c) => c.sessions >= 2)
    .sort((a, b) => b.sessions - a.sessions || b.prompts - a.prompts)
    .slice(0, 25);
}

function jaccard(a: Set<number>, b: Set<number>): number {
  let shared = 0;
  for (const v of b) if (a.has(v)) shared++;
  return shared / (a.size + b.size - shared);
}

function truncate(text: string, max = 160): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length <= max ? flat : `${flat.slice(0, max - 1)}…`;
}

function frontmatterField(body: string, field: string): string {
  const fm = body.startsWith("---") ? body.slice(3, body.indexOf("\n---", 3)) : "";
  const line = fm.split("\n").find((l) => l.trim().startsWith(`${field}:`));
  if (!line) return "";
  return line.slice(line.indexOf(":") + 1).trim().replace(/^["']|["']$/g, "");
}

function registry(): { name: string; description: string; source: string }[] {
  const found: { name: string; description: string; source: string }[] = [];
  const add = (dir: string, source: string) => {
    const md = join(dir, "SKILL.md");
    if (!existsSync(md)) return;
    const body = readFileSync(md, "utf8");
    found.push({
      name: frontmatterField(body, "name") || basename(dir),
      description: truncate(frontmatterField(body, "description"), 240),
      source,
    });
  };
  for (const root of skillDirs) {
    if (!existsSync(root)) { warnings.push(`no skills directory at ${root}`); continue; }
    for (const entry of readdirSync(root)) {
      const dir = join(root, entry);
      try { if (statSync(dir).isDirectory()) add(dir, root); } catch { /* dangling link */ }
    }
  }
  for (const root of packRoots) {
    if (!existsSync(root)) { warnings.push(`no Packs root at ${root}`); continue; }
    for (const pack of readdirSync(root)) {
      const skills = join(root, pack, "skills");
      if (!existsSync(skills)) continue;
      for (const skill of readdirSync(skills)) add(join(skills, skill), `${basename(root)}/${pack}`);
    }
  }
  if (!found.length) warnings.push("the registry is empty; every proposal below will look uncovered");
  return found.sort((a, b) => a.name.localeCompare(b.name));
}

const all = readPrompts();
// A bare slash command is a harness invocation, not a sentence about a problem.
// Counted, never clustered: "/model typed 20 times" is a fact about the UI.
const isCommand = (text: string) => /^\/[a-z0-9:-]+\s*$/i.test(text.trim());
const cutoff = Date.now() - days * 86_400_000;
const inWindow = all.filter((p) => p.at >= cutoff && !isCommand(p.text));
const commandCounts: Record<string, number> = {};
for (const p of all.filter((p) => p.at >= cutoff && isCommand(p.text))) {
  const name = p.text.trim().toLowerCase();
  commandCounts[name] = (commandCounts[name] ?? 0) + 1;
}
if (all.length && !inWindow.length) {
  warnings.push(`no prompts inside ${days} days; the newest is ${new Date(Math.max(...all.map((p) => p.at))).toISOString().slice(0, 10)}`);
}

const bySession = new Map<string, Prompt[]>();
for (const p of inWindow) {
  if (!bySession.has(p.session)) bySession.set(p.session, []);
  bySession.get(p.session)!.push(p);
}
const friction: { prompt: string; before: string; project: string; at: string }[] = [];
for (const rows of bySession.values()) {
  rows.sort((a, b) => a.at - b.at);
  rows.forEach((row, i) => {
    if (!FRICTION.some((re) => re.test(row.text))) return;
    friction.push({
      prompt: truncate(row.text),
      before: i > 0 ? truncate(rows[i - 1].text) : "(first prompt of the session)",
      project: basename(row.project),
      at: new Date(row.at).toISOString().slice(0, 16).replace("T", " "),
    });
  });
}

const projects: Record<string, number> = {};
for (const p of inWindow) projects[basename(p.project)] = (projects[basename(p.project)] ?? 0) + 1;

const clusters = cluster(inWindow);

const report = {
  generatedAt: new Date().toISOString(),
  window: {
    days,
    from: new Date(cutoff).toISOString().slice(0, 10),
    to: new Date().toISOString().slice(0, 10),
    history: historyPath,
  },
  prompts: {
    total: all.length,
    inWindow: inWindow.length,
    sessions: bySession.size,
    projects,
  },
  commands: Object.fromEntries(Object.entries(commandCounts).sort((a, b) => b[1] - a[1]).slice(0, 15)),
  clusters: clusters,
  dominant,
  friction: friction.slice(0, 40),
  frictionTotal: friction.length,
  registry: registry(),
  warnings,
};

if (process.argv.includes("--json") || !process.stdout.isTTY) {
  console.log(JSON.stringify(report, null, 2));
} else {
  console.log(`window        ${report.window.from} → ${report.window.to} (${days}d)`);
  console.log(`prompts       ${report.prompts.inWindow} of ${report.prompts.total}, ${report.prompts.sessions} sessions`);
  console.log(`skills known  ${report.registry.length}`);
  console.log(`friction      ${report.frictionTotal} prompts`);
  console.log(`dominant      ${report.dominant.map((d) => `${d.token}(${d.prompts})`).join(" ")}`);
  console.log("");
  for (const c of report.clusters) {
    console.log(`${String(c.sessions).padStart(3)} sessions ${String(c.prompts).padStart(3)} prompts  ${c.tokens.join(" ")}  [${c.with.join(" ")}]`);
    console.log(`               ${c.samples[0] ?? ""}`);
  }
  for (const w of report.warnings) console.log(`warning: ${w}`);
}
