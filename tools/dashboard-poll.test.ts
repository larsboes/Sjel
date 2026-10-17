// One place polls, and this file holds the two halves of that: what makes a view stale, and
// the ratchet that keeps the next page from writing its own timer.
//
// Measured 2026-10-17, before `lib/ui/poll.svelte.ts`: seventeen `setInterval` sites in
// sixteen files, thirteen of them fetching data on beats from 2 s to 30 s and none of the
// thirteen keeping the instant its data arrived. A poll that failed left the previous rows on
// screen with nothing to distinguish them from a reading taken a second ago, so "older than
// its refresh interval" — the rule in `Packs/design/skills/ui-craftsmanship/SKILL.md`
// ("Stale") — was unsayable on every page at once.
//
// The poller owns the beat, the arrival time and the visibility listener. `freshness.ts`
// owns the judgement, which is pure and therefore tested here rather than through a
// component: the threshold, the absent cases, and which of several sources names the line.

import { describe, expect, test } from "bun:test";
import {
  STALE_INTERVALS,
  ageSeconds,
  fastestInterval,
  freshnessLine,
  intervalLabel,
  isStale,
  staleSources,
  type FreshnessSource,
} from "../dashboard/src/lib/ui/freshness";
import { SRC, code, lineOf, read, relative, sources } from "./lib/dashboard-source";

/** A source that just heard from the server, unless the case says otherwise. */
function source(over: Partial<FreshnessSource> = {}): FreshnessSource {
  return { intervalMs: 5_000, lastArrived: 1_000, nowMs: 1_000, running: true, ...over };
}

describe("a view is stale only when a beat it declared did not land", () => {
  test("data that arrived on the beat is fresh, and so is data inside one beat", () => {
    expect(isStale(source({ lastArrived: 1_000, nowMs: 1_000 }))).toBe(false);
    expect(isStale(source({ lastArrived: 1_000, nowMs: 5_999 }))).toBe(false);
    expect(ageSeconds(source({ lastArrived: 1_000, nowMs: 6_000 }))).toBe(5);
  });

  test("one whole interval with nothing new is stale", () => {
    const five = source({ intervalMs: 5_000, lastArrived: 0 });
    expect(STALE_INTERVALS).toBe(1);
    expect(isStale({ ...five, nowMs: 4_999 })).toBe(false);
    expect(isStale({ ...five, nowMs: 5_000 })).toBe(true);
    expect(isStale({ ...five, nowMs: 60 * 60_000 })).toBe(true);
  });

  test("the threshold follows the interval the surface declared", () => {
    const thirty = source({ intervalMs: 30_000, lastArrived: 0, nowMs: 20_000 });
    const five = source({ intervalMs: 5_000, lastArrived: 0, nowMs: 20_000 });
    expect([isStale(thirty), isStale(five)]).toEqual([false, true]);
  });
  // A stopped poller expects nothing: the systems page's storage report is fetched once on
  // purpose, and a one-shot read has no beat to be late against.
  test("a surface that is not running is never stale, however old its data is", () => {
    const stopped = source({ running: false, lastArrived: 0, nowMs: 86_400_000 });
    expect(isStale(stopped)).toBe(false);
    expect(freshnessLine([stopped])).toBe("");
    expect(ageSeconds(stopped)).toBe(86_400);
  });

  test("nothing has arrived yet is not an age", () => {
    const empty = source({ lastArrived: null, nowMs: 9_999_999 });
    expect(isStale(empty)).toBe(false);
    expect(ageSeconds(empty)).toBeNull();
    expect(freshnessLine([empty])).toBe("");
  });
});

describe("the line a stale view shows", () => {
  test("it is empty while every source is fresh, which is what makes it worth reading", () => {
    expect(freshnessLine([source(), source({ intervalMs: 30_000 })])).toBe("");
    expect(freshnessLine([])).toBe("");
  });

  // A page reading a 3 s beat and a 5 s beat is as old as the slower of the two, and naming
  // the fresher one would be the more comfortable lie.
  test("several sources resolve to the oldest, not the newest", () => {
    const slow = source({ intervalMs: 5_000, lastArrived: 0, nowMs: 12 * 60_000 });
    const quick = source({ intervalMs: 3_000, lastArrived: 11 * 60_000, nowMs: 12 * 60_000 });
    expect(staleSources([quick, slow]).length).toBe(2);
    expect(freshnessLine([quick, slow])).toBe("as of 12m ago");
  });

  test("one stale source is not quieted by a fresh neighbour", () => {
    const fresh = source({ lastArrived: 11 * 60_000, nowMs: 12 * 60_000, intervalMs: 30_000 });
    const stale = source({ lastArrived: 0, nowMs: 12 * 60_000, intervalMs: 3_000 });
    expect(freshnessLine([fresh, stale])).toBe("as of 12m ago");
  });

  test("a source with nothing to report is not stale, so a neighbour still names the line", () => {
    const empty = source({ lastArrived: null, nowMs: 12 * 60_000 });
    const stale = source({ lastArrived: 0, nowMs: 6 * 60_000 });
    expect(freshnessLine([empty, stale])).toBe("as of 6m ago");
  });
});

describe("an interval reads as a rate, not as an age", () => {
  test("seconds under a minute, whole minutes above it", () => {
    expect(intervalLabel(500)).toBe("500 ms");
    expect(intervalLabel(2_000)).toBe("2 s");
    expect(intervalLabel(30_000)).toBe("30 s");
    expect(intervalLabel(60_000)).toBe("1 min");
    expect(intervalLabel(90_000)).toBe("90 s");
    expect(intervalLabel(120_000)).toBe("2 min");
  });

  test("a beat that is not a positive number has no words", () => {
    expect(intervalLabel(0)).toBe("");
    expect(intervalLabel(-5)).toBe("");
    expect(intervalLabel(Number.NaN)).toBe("");
  });

  test("the tip names the fastest beat behind the view", () => {
    expect(fastestInterval([source({ intervalMs: 5_000 })])).toBe(5_000);
    expect(fastestInterval([source({ intervalMs: 30_000 }), source({ intervalMs: 3_000 })])).toBe(3_000);
    expect(fastestInterval([source({ running: false, intervalMs: 3_000 })])).toBeNull();
    expect(fastestInterval([])).toBeNull();
  });
});

// The two modules that may hold a timer, and what each is for. Everything else asks one of
// them: `poll` for data that can go stale, `clock` for the time.
const POLL_MODULE = "lib/ui/poll.svelte.ts";
const CLOCK_MODULE = "lib/ui/clock.svelte.ts";

/** Every `setInterval` call in the tree, as `path:line: text`. Type positions are not calls. */
function timers(): string[] {
  const found: string[] = [];
  for (const file of sources(SRC)) {
    const text = code(read(file));
    for (const match of text.matchAll(/setInterval\s*\(/g)) {
      found.push(`${relative(file)}:${lineOf(text, match.index ?? 0)}`);
    }
  }
  return found.sort();
}

describe("a page does not start its own timer", () => {
  test("the two modules that may poll are still there", () => {
    const present = sources(SRC).map(relative);
    expect([POLL_MODULE, CLOCK_MODULE].map((module) => present.includes(module))).toEqual([true, true]);
  });

  test("only the poller and the clock hold a setInterval", () => {
    const offenders = timers().filter(
      (line) => !line.startsWith(`${POLL_MODULE}:`) && !line.startsWith(`${CLOCK_MODULE}:`),
    );
    expect(offenders).toEqual([]);
  });

  test("the scan still finds timers, so a passing run means something", () => {
    const found = timers();
    expect(found.filter((line) => line.startsWith(`${POLL_MODULE}:`)).length).toBeGreaterThanOrEqual(1);
    expect(found.filter((line) => line.startsWith(`${CLOCK_MODULE}:`)).length).toBeGreaterThanOrEqual(1);
    // Seventeen sites moved here on 2026-10-17; the count is the point, not the exact number,
    // so this fails if a refactor quietly empties the tree of the thing being counted.
    expect(found.length).toBeGreaterThanOrEqual(2);
  });
});
