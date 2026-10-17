/**
 * One answer to "how old is this", so no surface phrases it again.
 *
 * Measured 2026-10-17: seven surfaces each wrote their own relative-age string, in four
 * shapes and three granularities. `routes/backup` reported seconds ("45s ago") while
 * `routes/capabilities` rounded the same minute up to "1m ago"; `systems/updates` said
 * "just now" for the same case and returned nothing for a clock-skewed future; and three
 * counted days three ways — `Math.floor(ms / 86_400_000)`, `Math.round`, and local
 * midnights — so a commit at 23:00 last night read "today" in one place and "yesterday" in
 * another. `home/kinds/trip-retrospective` phrased a day count the capability had already
 * worked out.
 *
 * This module owns the wording. A caller still owns the policy: which end is coarse enough
 * to name a day instead of a duration, what takes an old value's place, and how a day older
 * than the ladder is spelled. `dateLabel` in `home/format.ts`, the feed's `weekdayLong`, and
 * the `never` of a capability that has never run are those policies, unchanged.
 */

/** Anything a caller can hand over as an instant. A date-only string is read as local. */
export type AgeInput = Date | number | string | null | undefined;

const DAY_MS = 86_400_000;
const DATE_ONLY = /^\d{4}-\d{2}-\d{2}$/;

/**
 * A duration in seconds as words: "just now", "6m ago", "5h ago", "2d ago".
 *
 * Under a minute is "just now" rather than "45s ago". A second is a value that changes
 * between two reads of the same screen, and no decision has ever turned on it; a
 * capability reporting an age in seconds is reporting recency, which is what this says.
 * `missing` is what a caller shows when there is no age to show at all — a backup that has
 * never run says `never`.
 */
export function ageLabel(
  seconds: number | null | undefined,
  options: { missing?: string } = {},
): string {
  if (seconds === null || seconds === undefined || !Number.isFinite(seconds)) {
    return options.missing ?? "";
  }
  // Negative is a clock that disagrees with the record, not a value in the future.
  const elapsed = Math.max(0, seconds);
  if (elapsed < 60) return "just now";
  const minutes = Math.floor(elapsed / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

/** The same words for an instant. `missing` covers an absent or unparseable value. */
export function sinceLabel(
  at: AgeInput,
  options: { now?: number; missing?: string } = {},
): string {
  const ms = instant(at);
  if (ms === null) return options.missing ?? "";
  return ageLabel(((options.now ?? Date.now()) - ms) / 1000, { missing: options.missing });
}

/**
 * Whole local days from `at` to `now`, or null when `at` is absent or unreadable.
 *
 * Calendar days, not 24-hour spans: something that happened at 23:00 last night was
 * yesterday, whatever the hour this is read at. Midnight against midnight also survives a
 * DST boundary, where a day is 23 or 25 hours long.
 */
export function daysSince(at: AgeInput, now: number = Date.now()): number | null {
  const ms = instant(at);
  if (ms === null) return null;
  return Math.round((localMidnight(now) - localMidnight(ms)) / DAY_MS);
}

/**
 * A day count as words: "today", "yesterday", "3 days ago".
 *
 * A day that has not happened has no age, so a negative count is empty and the caller
 * decides what stands there — the feed names the weekday, the ladder names the date.
 */
export function dayLabel(days: number): string {
  if (!Number.isFinite(days) || days < 0) return "";
  if (days < 1) return "today";
  if (days < 2) return "yesterday";
  return `${Math.floor(days)} days ago`;
}

function localMidnight(ms: number): number {
  const date = new Date(ms);
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

function instant(at: AgeInput): number | null {
  if (at === null || at === undefined) return null;
  if (typeof at === "number") return Number.isFinite(at) ? at : null;
  if (at instanceof Date) return Number.isFinite(at.getTime()) ? at.getTime() : null;
  const text = at.trim();
  if (!text) return null;
  // `Date.parse` reads a bare `2026-10-17` as UTC midnight, which is the previous local
  // day west of Greenwich. A day key is a local day, and this is the one place that knows.
  if (DATE_ONLY.test(text)) {
    const [year, month, day] = text.split("-").map(Number);
    const local = new Date(year, month - 1, day);
    return Number.isFinite(local.getTime()) ? local.getTime() : null;
  }
  const ms = Date.parse(text);
  return Number.isFinite(ms) ? ms : null;
}
