// Retrieval over one case's virtual corpus: vector seeds, optionally expanded along structural
// edges. The variant under test in PLAN.md "Sjel questions over documents: a small GraphRAG".
// It indexes only the case's files, so a retrieval tool cannot leak anything the workspace lacks.
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { digest } from "./agent-eval.ts";

export type RetrievalMode = "vector" | "graph";

export interface Section {
  id: number;
  path: string;
  line: number;
  heading: string;
  text: string;
}

export interface Hit {
  path: string;
  line: number;
  heading: string;
  score: number;
  via: "seed" | "contains" | "references";
  text: string;
}

/** Text embedded per section; the full section stays retrievable. */
const EMBED_CHARS = 4_000;
const RETURN_CHARS = 1_500;
const SEEDS = 5;
const EXPANSIONS = 5;

/** Markdown splits at headings, TOML at `[table]` rows, anything else is one section. */
export function sections(files: Record<string, string>): Section[] {
  const out: Section[] = [];
  for (const [path, content] of Object.entries(files)) {
    const lines = content.split("\n");
    const start = (line: string) => (path.endsWith(".md") ? /^#{1,6} /.test(line) : path.endsWith(".toml") ? /^\[[^\]]+\]\s*$/.test(line) : false);
    let begin = 0;
    const flush = (end: number) => {
      const text = lines.slice(begin, end).join("\n").trim();
      if (text) out.push({ id: out.length, path, line: begin + 1, heading: start(lines[begin] ?? "") ? lines[begin].trim() : path, text });
    };
    for (let i = 1; i < lines.length; i++) if (start(lines[i])) { flush(i); begin = i; }
    flush(lines.length);
  }
  return out;
}

/** Edges Graphify also extracts: a section contains the deeper sections under it, and a section
 * references a file path or an upstreams.toml row it names. Undirected for expansion. */
export function edges(all: Section[]): { kind: "contains" | "references"; a: number; b: number }[] {
  const out: { kind: "contains" | "references"; a: number; b: number }[] = [];
  const depth = (s: Section) => (s.heading.match(/^(#+) /)?.[1].length ?? 0);
  const stack: Section[] = [];
  for (const s of all) {
    while (stack.length && (stack.at(-1)!.path !== s.path || depth(stack.at(-1)!) >= depth(s))) stack.pop();
    if (stack.length && depth(s) > 0) out.push({ kind: "contains", a: stack.at(-1)!.id, b: s.id });
    if (depth(s) > 0) stack.push(s);
  }
  const firstOf = new Map<string, number>();
  for (const s of all) if (!firstOf.has(s.path)) firstOf.set(s.path, s.id);
  const rows = new Map(all.filter((s) => s.path.endsWith(".toml") && /^\[/.test(s.heading)).map((s) => [s.heading, s.id]));
  for (const s of all) {
    for (const [path, id] of firstOf) if (path !== s.path && s.text.includes(path)) out.push({ kind: "references", a: s.id, b: id });
    for (const [row, id] of rows) if (id !== s.id && s.text.includes(row)) out.push({ kind: "references", a: s.id, b: id });
  }
  return out;
}

async function embed(binary: string, role: string, texts: string[], kind: "query" | "document"): Promise<number[][]> {
  const child = spawn(binary, ["--embed", role], { stdio: ["pipe", "pipe", "inherit"] });
  child.stdin.end(JSON.stringify({ texts, kind }) + "\n");
  let out = "";
  for await (const chunk of child.stdout) out += chunk;
  const reply = JSON.parse(out);
  if (reply.type !== "embeddings") throw new Error(`embedding failed: ${reply.message ?? out.slice(0, 200)}`);
  return reply.vectors;
}

function cosine(a: number[], b: number[]): number {
  let dot = 0, na = 0, nb = 0;
  for (let i = 0; i < a.length; i++) { dot += a[i] * b[i]; na += a[i] * a[i]; nb += b[i] * b[i]; }
  return dot / Math.sqrt(na * nb);
}

export class Retriever {
  private constructor(
    private readonly all: Section[],
    private readonly vectors: number[][],
    private readonly neighbors: Map<number, { id: number; kind: "contains" | "references" }[]>,
    private readonly binary: string,
    private readonly role: string,
  ) {}

  /** Embeds every section once; vectors are cached by section digest under `cacheDir`.
   * ponytail: linear cosine scan; a ~1,000-section corpus does not need an index. */
  static async build(files: Record<string, string>, binary: string, role: string, cacheDir: string): Promise<Retriever> {
    const all = sections(files);
    mkdirSync(cacheDir, { recursive: true });
    const cacheFile = join(cacheDir, `embeddings-${role}.json`);
    const cache: Record<string, number[]> = existsSync(cacheFile) ? JSON.parse(readFileSync(cacheFile, "utf8")) : {};
    const keys = all.map((s) => digest(s.text.slice(0, EMBED_CHARS)));
    const missing = all.filter((_, i) => !cache[keys[i]]);
    if (missing.length) {
      const fresh = await embed(binary, role, missing.map((s) => s.text.slice(0, EMBED_CHARS)), "document");
      missing.forEach((s, i) => (cache[keys[s.id]] = fresh[i]));
      writeFileSync(cacheFile, JSON.stringify(cache));
    }
    const neighbors = new Map<number, { id: number; kind: "contains" | "references" }[]>();
    for (const e of edges(all)) {
      for (const [from, to] of [[e.a, e.b], [e.b, e.a]]) {
        if (!neighbors.has(from)) neighbors.set(from, []);
        neighbors.get(from)!.push({ id: to, kind: e.kind });
      }
    }
    return new Retriever(all, keys.map((k) => cache[k]), neighbors, binary, role);
  }

  get size(): number {
    return this.all.length;
  }

  async search(query: string, mode: RetrievalMode): Promise<Hit[]> {
    const [q] = await embed(this.binary, this.role, [query], "query");
    const scores = this.vectors.map((v) => cosine(q, v));
    const ranked = scores.map((score, id) => ({ id, score })).sort((a, b) => b.score - a.score);
    const hits = new Map<number, Hit>();
    const hit = (id: number, via: Hit["via"]): Hit => {
      const s = this.all[id];
      return { path: s.path, line: s.line, heading: s.heading, score: Number(scores[id].toFixed(4)), via, text: s.text.slice(0, RETURN_CHARS) };
    };
    // Both modes return the same number of hits, so a difference is the expansion, not the budget.
    for (const { id } of ranked.slice(0, mode === "graph" ? SEEDS : SEEDS + EXPANSIONS)) hits.set(id, hit(id, "seed"));
    if (mode === "graph") {
      // One hop from every seed; the best-scoring neighbors fill the expansion budget.
      const candidates = new Map<number, Hit["via"]>();
      for (const seed of [...hits.keys()]) for (const n of this.neighbors.get(seed) ?? []) if (!hits.has(n.id) && !candidates.has(n.id)) candidates.set(n.id, n.kind);
      [...candidates].sort(([a], [b]) => scores[b] - scores[a]).slice(0, EXPANSIONS).forEach(([id, via]) => hits.set(id, hit(id, via)));
    }
    return [...hits.values()];
  }
}

export const RETRIEVE_TOOL = {
  name: "retrieve",
  description: "Semantic search over the workspace documents. Returns the most relevant sections as [{path,line,heading,score,via,text}]; cite path:line from them.",
  parameters: { type: "object", properties: { query: { type: "string", description: "what you are looking for, in words" } }, required: ["query"], additionalProperties: false },
};
