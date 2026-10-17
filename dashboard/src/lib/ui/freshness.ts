/**
 * One answer to "is this view as old as its own refresh", so no panel decides for itself.
 *
 * The wording of an age is `age.ts`'s. This module answers the other half: a surface that
 * polls declares how often it asks, records when data last arrived, and the difference
 * between those two is a state — fresh or stale — rather than a second phrasing of an age.
 *
 * Measured 2026-10-17: seventeen `setInterval` sites in sixteen files, thirteen of them
 * fetching data on beats from 2 s to 30 s, and none of the thirteen kept its arrival time.
 * So "older than its refresh interval" was unsayable: a failed poll left the previous rows
 * on screen with nothing on the page to distinguish them from a reading taken one second
 * ago. That is the defect this exists to remove, not the wording.
 *
 * What counts as stale. The rule is `Packs/design/skills/ui-craftsmanship/SKILL.md`
 * ("Stale"): data older than its refresh interval shows its age. A whole interval with
 * nothing new means the beat that should have brought fresh data did not land, so the
 * threshold is one interval — `STALE_INTERVALS`. It is not larger because a beat already
 * carries the read's own latency: the next beat is the one that replaces it.
 *
 * A source that is not running expects nothing, so it is never stale. A one-shot read has
 * no beat to be late against, so it is not a `FreshnessSource` at all — the systems page's
 * storage report is fetched once on purpose, and calling it stale would be a lie about a
 * decision the page made.
 */

import { ageLabel } from "./age";

/** What a polling surface must expose for its age to be shown. `Poller` satisfies it. */
export type FreshnessSource = {
  /** The beat the surface declares, in ms. */
  readonly intervalMs: number;
  /** When data last arrived, or null before the first arrival. */
  readonly lastArrived: number | null;
  /** The instant that age is measured against — advanced by the surface's own beat. */
  readonly nowMs: number;
  /** True while something is subscribed. A stopped surface expects no new data. */
  readonly running: boolean;
};

/** Intervals that may pass before a surface counts as stale. */
export const STALE_INTERVALS = 1;

/** How late the data is, in seconds, or null before the first arrival. */
export function ageSeconds(source: FreshnessSource): number | null {
  if (source.lastArrived === null) return null;
  return Math.max(0, Math.round((source.nowMs - source.lastArrived) / 1000));
}

/** True when a running surface has missed a beat it declared. */
export function isStale(source: FreshnessSource): boolean {
  if (!source.running || source.lastArrived === null) return false;
  return source.nowMs - source.lastArrived >= STALE_INTERVALS * source.intervalMs;
}

/** The sources among `sources` whose data is older than their own beat. */
export function staleSources(sources: readonly FreshnessSource[]): FreshnessSource[] {
  return sources.filter(isStale);
}

/**
 * The line a stale view shows: "as of 12m ago".
 *
 * Empty while every source is fresh, which is what makes it worth reading when it appears:
 * a page that always names its age trains the reader to skip the place the warning comes
 * from. There is no `missing` option — staleness requires an arrival to measure against, so
 * a surface with nothing to report has nothing to say here.
 *
 * Several sources resolve to the oldest one. A page that reads a 3 s beat and a 4 s beat
 * is as old as the slower of the two, and naming the fresher one would be the more
 * comfortable lie.
 */
export function freshnessLine(sources: readonly FreshnessSource[]): string {
  const stale = staleSources(sources);
  if (!stale.length) return "";
  const age = Math.max(...stale.map((source) => ageSeconds(source) ?? 0));
  return `as of ${ageLabel(age)}`;
}

/** The fastest declared beat behind a view, for the tip that names it. Null with none. */
export function fastestInterval(sources: readonly FreshnessSource[]): number | null {
  const running = sources.filter((source) => source.running).map((source) => source.intervalMs);
  return running.length ? Math.min(...running) : null;
}

/** A beat as words: "2 s", "90 s", "2 min". An interval is a rate, not an age, so no "ago". */
export function intervalLabel(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "";
  if (ms < 1000) return `${Math.round(ms)} ms`;
  const seconds = ms / 1000;
  // A whole number of minutes is how a slow beat is declared and how it reads best; anything
  // else stays in seconds, because "2 min" for 90 s is a different beat from the one declared.
  if (seconds >= 60 && seconds % 60 === 0) {
    const minutes = seconds / 60;
    return minutes === 1 ? "1 min" : `${minutes} min`;
  }
  return `${Math.round(seconds)} s`;
}
