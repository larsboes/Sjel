// One place phrases an age, and this file holds the two halves of that: the ladder's
// boundaries, and the ratchet that keeps the next page from writing a fourth one.
//
// Measured 2026-10-17, before `lib/ui/age.ts`: seven surfaces each phrased their own
// relative age, in four shapes. `routes/backup` printed seconds ("45s ago") while
// `routes/capabilities` rounded the same minute up to "1m ago" and `lib/systems/updates`
// said "just now"; three of them counted days three ways — `Math.floor(ms / 86_400_000)`,
// `Math.round`, and local midnights — so a commit from 23:00 last night read "today" in
// `RepoStatusCard` and "yesterday" in `home/format`'s rows. `home/kinds/trip-retrospective`
// took a day count from the capability and phrased it itself, and the feed's day buckets
// had a seventh wording for the same two words.
//
// The module owns the wording. The callers kept their policies, which are not the same
// question: how old is too old to name a day, what replaces an old value, and what a
// capability that has never run says. Those live at the call sites still — `dateLabel` in
// `home/format.ts`, the feed's `weekdayLong`, the `never` of a backup contract.
//
// Two visible strings changed, both deliberate: a duration under a minute is "just now"
// rather than a second count, and a value whose clock disagrees with the record reads as
// recency instead of printing nothing.

import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { extname, join } from "node:path";
import { ageLabel, dayLabel, daysSince, sinceLabel } from "../dashboard/src/lib/ui/age";

const SRC = join(import.meta.dir, "../dashboard/src");

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) return sources(full);
    const extension = extname(name);
    return extension === ".ts" || extension === ".svelte" ? [full] : [];
  });
}

const relative = (file: string) => file.slice(SRC.length + 1);

/**
 * Source with its comments blanked out, newlines kept.
 *
 * Blanked rather than deleted so every offset is still the offset in the file and a
 * failure names the line an editor opens at. The comments matter here: two of them quote
 * the very words the scan looks for — the feed's "last run 3 days ago" and the retro row's
 * old phrasing — and a scan that reads them reports the defect it was written to prevent.
 */
function code(text: string): string {
  const blank = (region: string) => region.replace(/[^\n]/g, " ");
  return text
    .replace(/<!--[\s\S]*?-->/g, blank)
    .replace(/\/\*[\s\S]*?\*\//g, blank)
    .replace(/(^|[^:])\/\/[^\n]*/g, (_match, before: string) => before);
}

/** Every string literal: a backtick literal may span lines, a quoted one does not. */
const LITERALS = /`([^`]*)`|'([^'\n]*)'|"([^"\n]*)"/g;

/**
 * What counts as a phrase rather than a word.
 *
 * A bare `ago` is not one: `cv` names a local helper `ago(...)` and one page holds a
 * variable called `tagOptions`, and both are inside literals that the first version of
 * this scan reported as defects. An age phrase has a unit in front of it — "40s ago",
 * "3 days ago" — or is one of the two phrases the ladder owns outright.
 */
const AGE_PHRASE =
  /\b(?:(?:s|m|h|d|secs?|seconds?|mins?|minutes?|hours?|days?|weeks?|months?)\s+ago|yesterday|just now)\b/i;

const MODULE = "lib/ui/age.ts";
const lineOf = (text: string, index: number) => text.slice(0, index).split("\n").length;

/** Every age phrase in the tree, as `path:line: text`. */
function ageStrings(): string[] {
  const found = new Map<string, string>();
  for (const file of sources(SRC)) {
    const text = code(readFileSync(file, "utf8"));
    for (const match of text.matchAll(LITERALS)) {
      const content = match[1] ?? match[2] ?? match[3] ?? "";
      if (!AGE_PHRASE.test(content)) continue;
      found.set(
        `${relative(file)}:${lineOf(text, match.index)}`,
        content.trim().replace(/\s+/g, " "),
      );
    }
  }
  return [...found.entries()].map(([where, text]) => `${where}: ${text}`).sort();
}

describe("a duration says how long ago, in the units a person reads", () => {
  test("the ladder climbs seconds to minutes to hours to days", () => {
    expect(ageLabel(0)).toBe("just now");
    expect(ageLabel(59)).toBe("just now");
    expect(ageLabel(60)).toBe("1m ago");
    expect(ageLabel(3_599)).toBe("59m ago");
    expect(ageLabel(3_600)).toBe("1h ago");
    expect(ageLabel(86_399)).toBe("23h ago");
    expect(ageLabel(86_400)).toBe("1d ago");
    expect(ageLabel(8 * 86_400)).toBe("8d ago");
  });

  test("no age at all takes the caller's word for it", () => {
    expect(ageLabel(null)).toBe("");
    expect(ageLabel(undefined)).toBe("");
    expect(ageLabel(Number.NaN)).toBe("");
    expect(ageLabel(null, { missing: "never" })).toBe("never");
  });

  // The old `updates` formatter returned "" here, so a record written a minute in the
  // future left an empty slot where the line reads "applying, started …".
  test("a clock that disagrees with the record is recency, not a negative", () => {
    expect(ageLabel(-60)).toBe("just now");
    expect(sinceLabel("2026-10-01T12:00:00Z", { now: Date.parse("2026-10-01T11:59:00Z") })).toBe(
      "just now",
    );
  });
});

describe("an instant reads as the same words", () => {
  const at = "2026-10-01T12:00:00Z";
  const t = Date.parse(at);

  test("a moment, minutes, hours and days", () => {
    expect(sinceLabel(at, { now: t })).toBe("just now");
    expect(sinceLabel(at, { now: t + 5 * 60_000 })).toBe("5m ago");
    expect(sinceLabel(at, { now: t + 3 * 3_600_000 })).toBe("3h ago");
    expect(sinceLabel(at, { now: t + 2 * 86_400_000 })).toBe("2d ago");
  });

  // `Date.parse("2026-10-17")` is UTC midnight, which is the previous local day anywhere
  // west of Greenwich. A day key is a local day, and this is the one place that knows it.
  test("a bare day key is read as a local day, not as UTC midnight", () => {
    const localNoon = new Date(2026, 9, 17, 12, 0).getTime();
    expect(daysSince("2026-10-17", localNoon)).toBe(0);
    expect(dayLabel(daysSince("2026-10-17", localNoon) ?? -1)).toBe("today");
  });

  test("nothing to read is the caller's word", () => {
    expect(sinceLabel(undefined, { now: t })).toBe("");
    expect(sinceLabel("not a date", { now: t })).toBe("");
    expect(sinceLabel("", { now: t, missing: "never" })).toBe("never");
  });
});

describe("an age in days counts calendar days", () => {
  // The bug the three hand-written counters disagreed about: at 01:00, 23:00 last night is
  // two hours ago and yesterday, and `Math.floor(ms / 86_400_000)` called it today.
  test("last night at 23:00 is yesterday at 01:00", () => {
    const days = daysSince(new Date(2026, 9, 16, 23, 0), new Date(2026, 9, 17, 1, 0).getTime());
    expect(days).toBe(1);
    expect(dayLabel(days ?? -1)).toBe("yesterday");
  });

  // Midnight against midnight, in a zone where the clocks move that night: a day is 23 or
  // 25 hours long, and neither is "two days" or "no days".
  test("a short or long night is still one day", () => {
    const spring = daysSince(new Date(2026, 2, 28, 23, 0), new Date(2026, 2, 29, 1, 0).getTime());
    const autumn = daysSince(new Date(2026, 9, 24, 23, 0), new Date(2026, 9, 25, 1, 0).getTime());
    expect([spring, autumn]).toEqual([1, 1]);
  });

  test("a day count is worded in words, and a day that has not happened has no age", () => {
    expect(dayLabel(0)).toBe("today");
    expect(dayLabel(1)).toBe("yesterday");
    expect(dayLabel(2)).toBe("2 days ago");
    expect(dayLabel(45)).toBe("45 days ago");
    expect(dayLabel(-1)).toBe("");
  });

  test("an absent or unreadable value is null rather than a number", () => {
    expect(daysSince(null)).toBeNull();
    expect(daysSince(undefined)).toBeNull();
    expect(daysSince("")).toBeNull();
    expect(daysSince("not a date")).toBeNull();
  });
});

describe("a page does not phrase its own age", () => {
  test("the one module that may hold an age string is still there", () => {
    const present = sources(SRC).map(relative);
    expect([MODULE, present.includes(MODULE)]).toEqual([MODULE, true]);
  });

  test("only the module holds one", () => {
    const offenders = ageStrings().filter((line) => !line.startsWith(`${MODULE}:`));
    expect(offenders).toEqual([]);
  });

  test("the scan still finds age strings, so a passing run means something", () => {
    const inModule = ageStrings().filter((line) => line.startsWith(`${MODULE}:`));
    expect(inModule.length).toBeGreaterThanOrEqual(4);
  });
});
