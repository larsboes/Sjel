/**
 * research/ as the /research routes read it. Pure functions over the text vite/research.ts
 * bundles, so `bun test` can check them without SvelteKit.
 *
 * The markdown files stay the source. An entry's title is its `# ` line; its slug is its file
 * name. research/README.md is the index page.
 */

export interface UpstreamRow {
  name: string;
  url: string;
  verdict: string;
  license: string;
  summary: string;
}

export interface SystemRow {
  name: string;
  url: string | null;
  kind: string;
  local: boolean;
  why: string;
}

export interface Entry {
  slug: string;
  file: string;
  title: string;
  /** The markdown without its `# ` title line, which the page header shows instead. */
  body: string;
}

const REPO = "https://github.com/larsboes/Sjel/blob/main";

export function toEntry(file: string, markdown: string): Entry {
  const title = markdown.match(/^# (.+)$/m)?.[1]?.trim() ?? file;
  return {
    slug: file === "README.md" ? "" : file.replace(/\.md$/, ""),
    file,
    title,
    body: markdown.replace(/^# .+\n+/m, ""),
  };
}

/**
 * Where a relative link in an entry points. A link to another entry stays in the app; any other
 * repository file goes to GitHub, where it can be read. `link` adds the configured base path.
 */
export function resolveResearchLink(href: string, link: (path: string) => string): string {
  if (/^[a-z]+:/i.test(href) || href.startsWith("#")) return href;
  const [path, anchor] = href.split("#");
  const hash = anchor ? `#${anchor}` : "";
  if (!path.includes("/") && path.endsWith(".md")) {
    return link(path === "README.md" ? `/research${hash}` : `/research/${path.slice(0, -3)}${hash}`);
  }
  const repoPath = `research/${path}`.replace(/^research\/\.\.\//, "");
  return `${REPO}/${repoPath}${hash}`;
}

/** How a project relates to Sjel, grouped for a reader rather than by audit verdict. */
export const PROJECT_GROUPS: { id: string; label: string; blurb: string; verdicts: string[] }[] = [
  {
    id: "built-on",
    label: "Built on",
    blurb: "Code Sjel uses, contributes to, or carries a reviewed delta for.",
    verdicts: ["adopt", "adapt", "contribute", "overlay", "fork", "build"],
  },
  {
    id: "learned-from",
    label: "Learned from",
    blurb: "Projects whose ideas or parts Sjel took, without depending on their code.",
    verdicts: ["inspiration", "quarry"],
  },
  {
    id: "watching",
    label: "Watching",
    blurb: "Noted and not yet read. A watch row grants nothing until an audit gives it a verdict.",
    verdicts: ["watch"],
  },
  {
    id: "declined",
    label: "Declined",
    blurb: "Read and declined, with the reason kept so nobody has to read them again.",
    verdicts: ["reject"],
  },
];

export function groupUpstreams(rows: UpstreamRow[]): { id: string; label: string; blurb: string; rows: UpstreamRow[] }[] {
  const byName = (a: UpstreamRow, b: UpstreamRow) => a.name.localeCompare(b.name);
  const known = new Set(PROJECT_GROUPS.flatMap((g) => g.verdicts));
  const groups = PROJECT_GROUPS.map((group) => ({
    ...group,
    rows: rows.filter((row) => group.verdicts.includes(row.verdict)).sort(byName),
  }));
  // A verdict this page predates still shows, under its own name, rather than vanishing.
  const other = rows.filter((row) => !known.has(row.verdict)).sort(byName);
  if (other.length) groups.push({ id: "other", label: "Other verdicts", blurb: "", verdicts: [], rows: other });
  return groups;
}

export function matches(query: string, ...fields: (string | null)[]): boolean {
  const q = query.trim().toLowerCase();
  return !q || fields.some((field) => field?.toLowerCase().includes(q));
}

/** One result file under research/benchmarks/<suite>/results/, as the run wrote it. */
export interface BenchmarkResult {
  suite: string;
  date: string;
  backend: string;
  model: string;
  host: string;
  n: number;
  correct: number;
  accuracy: number;
  latency_ms: { p50: number; p95: number };
  cases: { id: string; expected: unknown; got: unknown; confidence: number | null; ms: number }[];
}

export interface BenchmarkRun {
  suite: string;
  file: string;
  result: BenchmarkResult;
}

export interface Suite {
  name: string;
  /** Best accuracy first; ties go to the faster median. */
  runs: BenchmarkRun[];
  /** Every case id any run answered, in first-seen order. */
  caseIds: string[];
}

export function suites(runs: BenchmarkRun[]): Suite[] {
  const byName = new Map<string, BenchmarkRun[]>();
  for (const run of runs) byName.set(run.suite, [...(byName.get(run.suite) ?? []), run]);
  return [...byName.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([name, list]) => {
      const sorted = [...list].sort(
        (a, b) => b.result.accuracy - a.result.accuracy || a.result.latency_ms.p50 - b.result.latency_ms.p50,
      );
      const caseIds = [...new Set(sorted.flatMap((run) => run.result.cases.map((c) => c.id)))];
      return { name, runs: sorted, caseIds };
    });
}

/** Whether a run answered a case correctly, or null when the run did not include it. */
export function verdict(run: BenchmarkRun, id: string): boolean | null {
  const found = run.result.cases.find((c) => c.id === id);
  return found ? JSON.stringify(found.got) === JSON.stringify(found.expected) : null;
}

export const percent = (value: number): string => `${(value * 100).toFixed(1)}%`;
