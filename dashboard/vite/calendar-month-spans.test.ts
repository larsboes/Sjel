import { describe, expect, test } from "bun:test";

import { isMultiDay, weekSpans } from "../src/lib/calendar/types";
import type { CalendarEntry, TripPlan } from "../src/lib/api";

// Mon 5 Oct 2026 to Sun 11 Oct 2026.
const WEEK = ["05", "06", "07", "08", "09", "10", "11"].map((d) => `2026-10-${d}`);

function entry(overrides: Partial<CalendarEntry>): CalendarEntry {
  return {
    id: "cal:entry:x",
    kind: "event",
    commitment: "planned",
    title: "Something",
    starts_at: "2026-10-07",
    ends_at: "2026-10-08",
    all_day: true,
    location: null,
    notes: null,
    source: "manual",
    external_id: null,
    rhythm_id: null,
    payload: null,
    created_at: "2026-10-01T00:00:00",
    updated_at: "2026-10-01T00:00:00",
    ...overrides,
  } as CalendarEntry;
}

function trip(overrides: Partial<TripPlan>): TripPlan {
  return {
    id: "trip:x",
    title: "Bonn to Stuttgart",
    destinations: [{ id: "p", name: "Stuttgart" }],
    date_start: "2026-10-07",
    date_end: "2026-10-13",
    ...overrides,
  } as TripPlan;
}

describe("a range is one bar, not a chip per day", () => {
  test("one all-day day is a chip, two or more are a bar", () => {
    expect(isMultiDay(entry({}))).toBe(false);
    expect(isMultiDay(entry({ ends_at: "2026-10-09" }))).toBe(true);
    expect(isMultiDay(entry({ all_day: false, starts_at: "2026-10-07T22:00", ends_at: "2026-10-09T02:00" }))).toBe(false);
  });

  test("an entry inside the week spans its own columns, end exclusive", () => {
    const { spans, lanes } = weekSpans(WEEK, [entry({ ends_at: "2026-10-10" })], []);
    expect(spans.map(({ start, end, continuesBefore, continuesAfter }) => ({ start, end, continuesBefore, continuesAfter })))
      .toEqual([{ start: 2, end: 5, continuesBefore: false, continuesAfter: false }]);
    expect(lanes).toBe(1);
  });

  test("a trip's end date is inclusive and runs on past the row", () => {
    const [bar] = weekSpans(WEEK, [], [trip({})]).spans;
    expect([bar.start, bar.end, bar.continuesAfter, bar.label]).toEqual([2, 7, true, "Stuttgart"]);
  });

  test("a trip that ends on the row's last day does not run on", () => {
    const [bar] = weekSpans(WEEK, [], [trip({ date_end: "2026-10-11" })]).spans;
    expect([bar.end, bar.continuesAfter]).toEqual([7, false]);
  });

  test("a range from last week is clamped and marked as continuing", () => {
    const [bar] = weekSpans(WEEK, [entry({ starts_at: "2026-10-01", ends_at: "2026-10-07" })], []).spans;
    expect([bar.start, bar.end, bar.continuesBefore]).toEqual([0, 2, true]);
  });

  test("overlapping bars stack into lanes; disjoint ones share a lane", () => {
    const { spans, lanes } = weekSpans(
      WEEK,
      [
        entry({ id: "a", starts_at: "2026-10-05", ends_at: "2026-10-08" }),
        entry({ id: "b", starts_at: "2026-10-06", ends_at: "2026-10-09" }),
        entry({ id: "c", starts_at: "2026-10-09", ends_at: "2026-10-11" }),
      ],
      [],
    );
    const lane = Object.fromEntries(spans.map((s) => [s.key, s.lane]));
    expect(lane).toEqual({ a: 0, b: 1, c: 0 });
    expect(lanes).toBe(2);
  });

  test("a trip without dates, or outside the week, draws nothing", () => {
    expect(weekSpans(WEEK, [], [trip({ date_start: "" }), trip({ date_start: "2026-11-01", date_end: "2026-11-03" })]).spans)
      .toEqual([]);
  });
});
