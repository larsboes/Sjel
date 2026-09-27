import type { Component } from "svelte";
import { axonStatus } from "../api";
import { capabilities } from "../capabilities.svelte";
import type { Decision, DecisionKind, LoadContext, ScoreContext } from "./decisions";
import { score } from "./decisions";

/**
 * Discovery, not a list.
 *
 * Adding a decision kind used to mean five edits in a 1996-line file: the discriminated
 * union, a push block, a slot in one `Promise.allSettled`, a branch of `brief`, a branch
 * of `openDecision` and a branch of a 210-line snippet. That is what made two capabilities'
 * worth of ladder work serial. A kind is now one file under `kinds/`, and a row component
 * one file under `rows/`, and neither needs a line here.
 *
 * Two globs, joined on the kind's `view` STRING. A kind names its row rather than
 * importing it, which is what keeps every kind file readable by plain `bun test` outside
 * Vite — the same rule `nav.ts` follows for `$app/paths`. Nothing here imports a kind by
 * name either, so a kind that reaches a client module absent from one worktree does not
 * break the others.
 *
 * WARNING to whoever adds the next kind: `eager: true` puts every kind's transitive
 * imports into Home's statically reachable graph. `vite.config.ts` fails the build at a
 * 500 kB eager chunk and outright if MapLibre, Mermaid or Vega becomes eagerly reachable.
 * A kind may import `../../api` and `../../nav` and little else.
 * `tools/dashboard-home-registry.test.ts` is the guard.
 */

type AnyKind = DecisionKind<unknown, unknown>;

const kindModules = import.meta.glob<{ default: AnyKind }>("./kinds/*.ts", { eager: true });
type RowComponent = Component<Record<string, unknown>>;

const rowModules = import.meta.glob<{ default: RowComponent }>("./rows/*.svelte", {
  eager: true,
});

const fileName = (path: string): string => path.replace(/^.*\//, "").replace(/\.\w+$/, "");

/** Every kind on this machine, in the order the ladder's bands put them. */
export const KINDS: AnyKind[] = Object.entries(kindModules)
  .map(([path, module]) => {
    const kind = module.default;
    if (kind.key !== fileName(path)) {
      // A mismatch silently breaks `${key}:${id}`, `dismissed` and `dependsOn` at once.
      throw new Error(`home/kinds/${fileName(path)}.ts declares key "${kind.key}"`);
    }
    return kind;
  })
  .sort((a, b) => b.band - a.band || (a.key < b.key ? -1 : 1));

export const ROWS: Record<string, RowComponent> = Object.fromEntries(
  Object.entries(rowModules).map(([path, module]) => [fileName(path), module.default]),
);

const reportedMissingRows = new Set<string>();

/**
 * The row component a kind names, or null when the file is not in this checkout.
 *
 * Null rather than a throw, and the reason is the whole point of the registry. This
 * resolves inside Home's render snippet, which sits under no `<svelte:boundary>`, so a
 * throw here blanks the flagship page — and it would be thrown by a kind file some other
 * stream added, on the first row that kind ever produced. A kind whose row is absent is
 * skipped and named once in the console, which is the same rule the demo index states for
 * a capability the recording could not include: absence degrades, it does not fail.
 */
export const rowComponent = (kind: AnyKind): RowComponent | null => {
  const component = ROWS[kind.view];
  if (component) return component;
  if (!reportedMissingRows.has(kind.key)) {
    reportedMissingRows.add(kind.key);
    console.warn(
      `home/kinds/${kind.key}.ts names rows/${kind.view}.svelte, which does not exist; its rows are not shown`,
    );
  }
  return null;
};

/** What Home knows about one kind at a moment in time. */
export interface KindState {
  status: "loading" | "ready" | "failed";
  source: unknown;
  rows: unknown[];
}

/**
 * Starts a stopped capability before reading it, once per capability per run.
 *
 * Carried over from the page's own `readCapability`, including its two reasons: the
 * capability list has to be refreshed first (on a cold load it is empty and every "is it
 * up" test is vacuously false), and an unknown capability is skipped rather than started,
 * because a demo build has no such route. Keyed on the capability rather than the kind, so
 * comms is started once although both `mail` and `feed` read it.
 *
 * The PROMISE is memoised, not a flag. A flag records the attempt before the POST answers,
 * and `POST …/capabilities/{name}/start` blocks until the capability replies
 * (capabilities/sjel-status/src/main.rs, "Start one capability and wait for it to answer"),
 * so the second kind on the same capability — `feed`, after `mail` — would read while comms
 * was still booting and get the proxy's 502 for a stopped capability. That failed its whole
 * lane behind "Unavailable: Feed" on exactly the cold machine this function exists for.
 * Both kinds now await the one start.
 *
 * `demo` is the same rule +layout.svelte states for its own autostart: a demo build has no
 * start route, and posting to it answers 501 and logs an error a visitor cannot act on.
 */
export function createStarter(demo = false): (capability: string | null) => Promise<void> {
  const inflight = new Map<string, Promise<void>>();
  return (capability) => {
    if (!capability || demo) return Promise.resolve();
    let started = inflight.get(capability);
    if (!started) {
      started = (async () => {
        const view = capabilities.byName(capability);
        if (!view || view.up === true) return;
        await axonStatus.start(capability).catch(() => {
          // Swallowed: the read below reports the real failure, and a start that could not
          // even be attempted is not a second thing to tell the operator about.
        });
      })();
      inflight.set(capability, started);
    }
    return started;
  };
}

export interface RunOptions {
  /** Everything a context needs except the two peer accessors, which the run owns. */
  base: Omit<LoadContext, "peer" | "peerSource">;
  /** Called as each kind settles, so the ladder paints per kind instead of per page. */
  onSettled(key: string, state: KindState): void;
  start: (capability: string | null) => Promise<void>;
}

/**
 * Runs every kind, each on its own promise.
 *
 * The defect this closes: one `await Promise.allSettled` over seven reads held `loading`
 * true until the slowest capability settled, so the flagship page rendered its heading and
 * nothing else. The 2026-09-03 baseline capture at four seconds shows exactly that.
 *
 * A kind that declares `dependsOn` waits for those kinds and no others. Without the wait,
 * an opportunity's rank would depend on when the calendar happened to answer and the
 * ladder would visibly re-order — a regression the single `allSettled` did not have.
 */
export async function runKinds(options: RunOptions): Promise<void> {
  const sources = new Map<string, unknown>();
  const rowsByKind = new Map<string, unknown[]>();

  const ctx: LoadContext = {
    ...options.base,
    peer: <Row,>(key: string) => (rowsByKind.get(key) ?? []) as readonly Row[],
    peerSource: <Source,>(key: string) => (sources.get(key) as Source | undefined) ?? null,
  };

  // Gates are created for EVERY kind before any of them runs, so a dependency declared on
  // a lower band still resolves. Keying off insertion order would make the graph's
  // correctness depend on the band table, which is a different fact entirely.
  const gates = new Map<string, { done: Promise<void>; settle: () => void }>();
  for (const kind of KINDS) {
    let settle!: () => void;
    const done = new Promise<void>((resolve) => {
      settle = resolve;
    });
    gates.set(kind.key, { done, settle });
  }

  const run = async (kind: AnyKind): Promise<void> => {
    let state: KindState = { status: "failed", source: null, rows: [] };
    try {
      await options.start(kind.capability);
      const source = await kind.load(ctx);
      sources.set(kind.key, source);

      for (const dependency of kind.dependsOn ?? []) {
        // Settled, not fulfilled: a failed dependency yields no rows and this kind ranks
        // without it, which is a stated outcome rather than a hang.
        await gates.get(dependency)?.done;
      }

      const rows = kind.rows(source, ctx);
      rowsByKind.set(kind.key, rows);
      state = { status: "ready", source, rows };
    } catch {
      rowsByKind.set(kind.key, []);
    } finally {
      gates.get(kind.key)?.settle();
    }
    options.onSettled(kind.key, state);
  };

  await Promise.all(KINDS.map(run));
}

/** Builds the ranked ladder from whatever has arrived so far. */
export function decisionsFrom(
  states: Record<string, KindState>,
  ctx: ScoreContext,
  dismissed: ReadonlySet<string>,
): Decision[] {
  const out: Decision[] = [];
  for (const kind of KINDS) {
    for (const row of states[kind.key]?.rows ?? []) {
      const key = `${kind.key}:${kind.id(row)}`;
      if (dismissed.has(key)) continue;
      out.push({
        key,
        kind,
        row,
        priority: score(kind.band, kind.urgency(row, ctx)),
        startOrDueAt: kind.startOrDueAt(row),
      });
    }
  }
  return out;
}
