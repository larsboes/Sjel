// tools/ci-cache.ts — what the repository's GitHub Actions caches hold, and which are dead weight.
//
// GitHub evicts the least recently used cache once a repository passes its 10 GB limit, so dead
// entries push out the ones the next run needs. An entry is dead when:
//   superseded  an entry on the same ref with the same key, hash suffix removed, was restored
//               more recently; CI restores that one, so the other is never read again
//   closed-pr   its ref is a pull request that is closed or merged
//
//   tools/ci-cache                  report: totals, every key prefix, the dead entries
//   tools/ci-cache prune            print what prune would delete
//   tools/ci-cache prune --apply    delete those entries

export interface Cache {
  id: number;
  key: string;
  ref: string;
  size_in_bytes: number;
  last_accessed_at: string;
}

export type Verdict = "superseded" | "closed-pr";

export const LIMIT_BYTES = 10 * 1024 ** 3;

export function prefix(key: string): string {
  return key.replace(/(-[0-9a-f]{8,})+$/, "");
}

export function prNumber(ref: string): number | undefined {
  const m = /^refs\/pull\/(\d+)\//.exec(ref);
  return m ? Number(m[1]) : undefined;
}

/** Dead entries, by id. `openPRs` holds every pull request number that is still open. */
export function dead(caches: Cache[], openPRs: Set<number>): Map<number, Verdict> {
  const out = new Map<number, Verdict>();
  const newest = new Map<string, Cache>();
  for (const c of caches) {
    const pr = prNumber(c.ref);
    if (pr !== undefined && !openPRs.has(pr)) out.set(c.id, "closed-pr");
    const group = `${c.ref}\0${prefix(c.key)}`;
    const seen = newest.get(group);
    if (!seen || c.last_accessed_at > seen.last_accessed_at) newest.set(group, c);
  }
  for (const c of caches) {
    if (out.has(c.id)) continue;
    if (newest.get(`${c.ref}\0${prefix(c.key)}`)!.id !== c.id) out.set(c.id, "superseded");
  }
  return out;
}

function gh(args: string[]): string {
  const run = Bun.spawnSync(["gh", ...args]);
  if (run.exitCode !== 0) throw new Error(`gh ${args.join(" ")}: ${run.stderr.toString().trim()}`);
  return run.stdout.toString();
}

function lines(text: string): string[] {
  return text.split("\n").filter(Boolean);
}

const gb = (bytes: number) => `${(bytes / 1e9).toFixed(2)} GB`;

function main(argv: string[]): number {
  const [verb, ...flags] = argv;
  if (verb !== undefined && verb !== "prune") {
    console.error("usage: tools/ci-cache [prune [--apply]]");
    return 2;
  }
  const caches: Cache[] = lines(
    gh(["api", "--paginate", "repos/{owner}/{repo}/actions/caches?per_page=100", "--jq", ".actions_caches[]"]),
  ).map((l) => JSON.parse(l));
  const openPRs = new Set(lines(gh(["pr", "list", "--state", "open", "--limit", "1000", "--json", "number", "--jq", ".[].number"])).map(Number));
  const verdicts = dead(caches, openPRs);
  const doomed = caches.filter((c) => verdicts.has(c.id));
  const total = caches.reduce((s, c) => s + c.size_in_bytes, 0);
  const freed = doomed.reduce((s, c) => s + c.size_in_bytes, 0);

  if (verb === undefined) {
    console.log(`Actions caches: ${caches.length} entries, ${gb(total)} of ${gb(LIMIT_BYTES)}${total > LIMIT_BYTES ? " — over the limit, GitHub is evicting" : ""}`);
    const groups = new Map<string, { n: number; bytes: number }>();
    for (const c of caches) {
      const g = `${c.ref.replace(/^refs\/heads\//, "")}  ${prefix(c.key)}`;
      const e = groups.get(g) ?? { n: 0, bytes: 0 };
      groups.set(g, { n: e.n + 1, bytes: e.bytes + c.size_in_bytes });
    }
    for (const [g, e] of [...groups].sort((a, b) => b[1].bytes - a[1].bytes)) {
      console.log(`  ${gb(e.bytes).padStart(8)}  ×${e.n}  ${g}`);
    }
  }
  for (const c of doomed) console.log(`  ${verdicts.get(c.id)!.padEnd(10)}  ${gb(c.size_in_bytes).padStart(8)}  ${c.key}`);
  console.log(`${doomed.length} dead entr${doomed.length === 1 ? "y" : "ies"}, ${gb(freed)}`);

  if (verb === "prune") {
    if (!flags.includes("--apply")) {
      console.log("dry run: nothing deleted. Pass --apply to delete them.");
      return 0;
    }
    for (const c of doomed) gh(["api", "-X", "DELETE", `repos/{owner}/{repo}/actions/caches/${c.id}`]);
    console.log(`deleted ${doomed.length}`);
  } else if (doomed.length > 0) {
    console.log("tools/ci-cache prune shows what would be deleted; --apply deletes it.");
  }
  return 0;
}

if (import.meta.main) process.exit(main(process.argv.slice(2)));
