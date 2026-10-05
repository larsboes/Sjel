import { link } from "../nav";
/** Entry kind as stored — open text, documented in the calendar capability's
 * README. The UI knows about well-known kinds for color/icon mapping but
 * must render any token gracefully. */
export type EntryKind =
  | "busy"
  | "work_onsite"
  | "work_remote"
  | "away"
  | "event"
  | "nightlife"
  | "deadline"
  | "travel_ok"
  | (string & {});

// The wire shapes live in `$lib/api` and are re-exported here so this module
// stays the one import a calendar component needs. They used to be declared
// twice, which is how `commitment` could exist on one copy and not the other
// — the compiler caught it, but only because the two were passed to each
// other; a field only ever read would have drifted silently.
export type {
  CalendarCommitment,
  CalendarContext,
  CalendarEntry,
  CalendarNewEntry,
  CalendarNewRhythm,
  CalendarRhythm,
  CalendarUpdateEntry,
  CalendarFeasibleWindow,
  CalendarWindows,
} from "$lib/api";
import type { CalendarContext, CalendarEntry, TripPlan } from "$lib/api";

/** A single day in a grid — derived from entries + rhythms. */
export interface CalendarDay {
  date: string; // YYYY-MM-DD
  day: number;  // 1-31
  entries: CalendarEntry[];
  isCurrentMonth: boolean;
  isToday: boolean;
}

/** Which surface the workspace shows. The anchor date is shared across all
 * three, so switching keeps the same date in view. */
export type CalendarView = "month" | "week" | "day";

export const VIEWS: Array<{ value: CalendarView; label: string }> = [
  { value: "month", label: "Month" },
  { value: "week", label: "Week" },
  { value: "day", label: "Day" },
];

export const KINDS: Array<{ value: EntryKind; label: string; color: string }> = [
  { value: "busy", label: "Busy", color: "#e11d48" },
  { value: "work_onsite", label: "Work (on site)", color: "#2563eb" },
  { value: "work_remote", label: "Work (remote)", color: "#0891b2" },
  { value: "away", label: "Away", color: "#d97706" },
  { value: "event", label: "Event", color: "#7c3aed" },
  { value: "nightlife", label: "Nightlife / Open Air", color: "#db2777" },
  { value: "deadline", label: "Deadline", color: "#b45309" },
  { value: "travel_ok", label: "Travel possible", color: "#16a34a" },
];

/** How binding an entry is. Orthogonal to `kind`: a holiday can be an idea or
 * a booked flight, and an event can be a bookmark or a paid ticket. The
 * calendar capability caps feasibility impact by this, so `possible` never
 * blocks a day whatever its kind. */
export type Commitment = "possible" | "planned" | "committed";

export const COMMITMENTS: Array<{
  value: Commitment;
  label: string;
  /** Shown on hover — what raising or lowering it actually costs you. */
  hint: string;
}> = [
  { value: "possible", label: "Possible", hint: "On the radar. Blocks nothing." },
  { value: "planned", label: "Planned", hint: "Decided, nothing booked. Soft block." },
  { value: "committed", label: "Committed", hint: "Booked or registered. Blocks the day." },
];

export function commitmentConfig(commitment: string) {
  return (
    COMMITMENTS.find((c) => c.value === commitment) ?? COMMITMENTS[0]
  );
}

/** Click-to-cycle order, wrapping. Deliberately a plain rotation rather than
 * a menu: raising one is the single most common edit in this workspace. */
export function nextCommitment(commitment: string): Commitment {
  const index = COMMITMENTS.findIndex((c) => c.value === commitment);
  return COMMITMENTS[(index + 1) % COMMITMENTS.length].value;
}

/** Whether an entry is worth surfacing right now.
 *
 * Computed, never stored — that was the design call: a recommendation is a
 * function of the score, the free window, reachability and cost, and all four
 * change whenever the calendar or the scoring does. A stored field would go
 * stale the next time anything moved and would need invalidating.
 *
 * Today it answers a deliberately narrow question: *this is something you have
 * not decided on, on a day the calendar says is genuinely free*. The relevance
 * score is not folded in yet on purpose — with both sides on multilingual-e5
 * every cosine lands in a 2.4-point band, so using it here would dress noise
 * up as a recommendation. */
export function isRecommended(
  entry: { commitment: string; starts_at: string; all_day: boolean },
  freeDays: ReadonlySet<string>,
): boolean {
  if (entry.commitment !== "possible") return false;
  return freeDays.has(entry.starts_at.slice(0, 10));
}

/** The days inside `windows` whose verdict is a clean `free`. A window that
 * merely avoids `conflicts` still costs a travel day, which is not something
 * to recommend unprompted. */
export function freeDaysOf(
  windows: Array<{ verdict: string; days: string[]; days_needing_travel_day: string[] }>,
): Set<string> {
  const free = new Set<string>();
  for (const window of windows) {
    if (window.verdict !== "free") continue;
    const costly = new Set(window.days_needing_travel_day);
    for (const day of window.days) if (!costly.has(day)) free.add(day);
  }
  return free;
}

export function kindConfig(kind: string) {
  return KINDS.find((k) => k.value === kind) ?? {
    value: kind,
    label: kind.replace(/_/g, " "),
    color: "#6b7280",
  };
}

/** Link to one entry inside the calendar workspace, for anything listing
 * entries elsewhere. The date is part of the link on purpose: it moves the
 * workspace's window before the id is looked up, so the target is inside the
 * range the page loads anyway and no fetch-by-id is needed. */
export function entryLink(entry: CalendarEntry): string {
  return link(`/calendar?date=${entry.starts_at.slice(0, 10)}&entry=${encodeURIComponent(entry.id)}`);
}

/** The entry in the shared content reader — the same surface that renders a
 *  feed article or a mail, via `content-item-v2`. Distinct from `entryLink`,
 *  which opens the editable form in the calendar grid: one is for reading what
 *  this is and where it came from, the other for changing when it happens. */
export function entryReaderLink(entry: CalendarEntry): string {
  return link(`/feed/${encodeURIComponent(entry.id)}?source=calendar`);
}

/** Same for a planning context, which opens in the context panel. */
export function contextLink(context: CalendarContext): string {
  return link(`/calendar?date=${context.valid_from.slice(0, 10)}&context=${encodeURIComponent(context.id)}`);
}

export function dayKey(date: Date | string): string {
  if (typeof date === "string") return date;
  return [
    date.getFullYear(),
    String(date.getMonth() + 1).padStart(2, "0"),
    String(date.getDate()).padStart(2, "0"),
  ].join("-");
}

export function addDays(date: Date, days: number): Date {
  const d = new Date(date);
  d.setDate(d.getDate() + days);
  return d;
}

/** Shifts a `YYYY-MM-DD` key by whole days. Noon, so a DST transition cannot
 *  push the result onto the wrong date. */
export function shiftDayKey(value: string, days: number): string {
  return dayKey(addDays(new Date(`${value}T12:00:00`), days));
}

/**
 * When an entry happens, in the shape a form shows it.
 *
 * `endDate` is **inclusive** here and exclusive in the store. That single
 * mismatch is the whole reason this lives in one place: an all-day entry ending
 * on the 16th covers the 15th, and any surface that forgets the conversion
 * silently moves the entry by a day. The month grid, the form and the reader
 * all have to agree, so none of them owns the rule.
 */
export interface EntryWhen {
  allDay: boolean;
  startDate: string;
  /** Inclusive — the last day the entry covers. */
  endDate: string;
  startTime: string;
  endTime: string;
}

/** Store shape → form shape. Takes only the three fields it reads, so an
 *  unsaved draft (no id yet) converts on the same path as a stored entry.
 *
 *  A contributed draft may omit `all_day`; the timestamp already says which it
 *  is, because a date-only instant is exactly what all-day means in this store.
 *  Reading it is more honest than the blanket default this replaced, which
 *  turned a timed draft into an all-day one. */
export function whenOf(
  entry: { all_day?: boolean; starts_at: string; ends_at: string },
): EntryWhen {
  const allDay = entry.all_day ?? !entry.starts_at.includes("T");
  return {
    allDay,
    startDate: entry.starts_at.slice(0, 10),
    endDate: allDay
      ? shiftDayKey(entry.ends_at.slice(0, 10), -1)
      : entry.ends_at.slice(0, 10),
    startTime: allDay ? "" : entry.starts_at.slice(11, 16),
    endTime: allDay ? "" : entry.ends_at.slice(11, 16),
  };
}

/** Form shape → the three fields the store takes. */
export function whenPatch(when: EntryWhen): {
  all_day: boolean;
  starts_at: string;
  ends_at: string;
} {
  if (when.allDay) {
    return {
      all_day: true,
      starts_at: when.startDate,
      ends_at: shiftDayKey(when.endDate, 1),
    };
  }
  return {
    all_day: false,
    starts_at: `${when.startDate}T${when.startTime || "09:00"}:00`,
    ends_at: `${when.endDate}T${when.endTime || "10:00"}:00`,
  };
}

/**
 * Why the store would reject this, before asking it.
 *
 * The store enforces the same rules and returns a 400; checking here is so the
 * operator sees the reason next to the field they just changed rather than as a
 * banner after a round trip. `null` means it would be accepted.
 */
export function whenError(when: EntryWhen): string | null {
  if (!when.startDate || !when.endDate) return "A start and end date are required";
  if (when.endDate < when.startDate) return "The end must be on or after the start";
  if (when.allDay) return null;
  if (!when.startTime || !when.endTime) return "A timed entry needs a start and end time";
  // Ends are exclusive, so same instant is an empty range the store refuses.
  if (`${when.endDate}T${when.endTime}` <= `${when.startDate}T${when.startTime}`) {
    return "The end must be after the start";
  }
  return null;
}

/** Monday-based weekday index (0=Mon..6=Sun), matching calendar-capability's
 * date::weekday. */
export function mondayWeekday(date: Date): number {
  return ((date.getDay() + 6) % 7);
}

/** A day key back to a Date. Noon, so a DST shift can never move the day. */
export function parseDayKey(date: string): Date {
  return new Date(`${date}T12:00:00`);
}

/** Monday to Sunday around `date`. */
export function weekDates(date: Date): string[] {
  const monday = addDays(date, -mondayWeekday(date));
  return Array.from({ length: 7 }, (_, index) => dayKey(addDays(monday, index)));
}

/** The whole-weeks grid a month is drawn on: Monday-based, padded with the
 * neighbouring months' days so the first and last rows are complete. */
export function monthDates(year: number, month: number): string[] {
  const last = new Date(year, month + 1, 0);
  const start = addDays(new Date(year, month, 1), -mondayWeekday(new Date(year, month, 1)));
  const end = addDays(last, 6 - mondayWeekday(last));
  const dates: string[] = [];
  for (let cursor = start; cursor <= end; cursor = addDays(cursor, 1)) {
    dates.push(dayKey(cursor));
  }
  return dates;
}

export const MINUTES_PER_DAY = 24 * 60;

/** A half-open [start, end) interval — minutes of a day for timed entries,
 * column indices for all-day ones. */
export interface Span {
  start: number;
  end: number;
}

/** Where a span sits among the neighbours it overlaps. */
export interface Lane {
  lane: number;
  lanes: number;
}

function minuteOfDay(stamp: string): number {
  const hours = Number(stamp.slice(11, 13));
  const minutes = Number(stamp.slice(14, 16));
  return Number.isFinite(hours) && Number.isFinite(minutes) ? hours * 60 + minutes : 0;
}

/** Minutes of `date` a timed entry occupies, clamped to that day. Ends stay
 * exclusive: 09:00–10:00 gives [540, 600), which is the 9 o'clock slot alone.
 * A block running past midnight yields a span on both days. Null for all-day
 * entries and for days the entry does not reach. */
export function timedSpan(entry: CalendarEntry, date: string): Span | null {
  if (entry.all_day) return null;
  const startDay = entry.starts_at.slice(0, 10);
  const endDay = entry.ends_at.slice(0, 10);
  if (date < startDay || date > endDay) return null;
  const start = startDay === date ? minuteOfDay(entry.starts_at) : 0;
  const end = endDay === date ? minuteOfDay(entry.ends_at) : MINUTES_PER_DAY;
  return end > start ? { start, end } : null;
}

/** Column range an all-day entry covers inside `dates`, as [start, end)
 * indices. The stored end is already exclusive; an entry reaching past either
 * edge of the window is clamped to it. */
export function allDaySpan(entry: CalendarEntry, dates: string[]): Span | null {
  if (!entry.all_day) return null;
  return rangeSpan(entry.starts_at.slice(0, 10), entry.ends_at.slice(0, 10), dates);
}

/** Columns the day range [first, endExclusive) covers inside `dates`, clamped to it. */
function rangeSpan(first: string, endExclusive: string, dates: string[]): Span | null {
  if (dates.length === 0) return null;
  if (endExclusive <= dates[0] || first > dates[dates.length - 1]) return null;
  const start = Math.max(dates.indexOf(first), 0);
  const found = dates.indexOf(endExclusive);
  const end = found < 0 ? dates.length : found;
  return end > start ? { start, end } : null;
}

/** An all-day entry that covers more than one day. The month grid draws it as one bar
 *  across the week rather than a chip repeated in every cell it touches. */
export function isMultiDay(entry: CalendarEntry): boolean {
  return entry.all_day && shiftDayKey(entry.starts_at.slice(0, 10), 1) < entry.ends_at.slice(0, 10);
}

/** One bar in a month row, clamped to the week. `continuesBefore` and `continuesAfter`
 *  say the range runs past the row's edge, so the bar's end is drawn open, not rounded. */
export type WeekSpan = Span &
  Lane & {
    key: string;
    label: string;
    continuesBefore: boolean;
    continuesAfter: boolean;
  } & ({ entry: CalendarEntry; trip?: never } | { trip: TripPlan; entry?: never });

/** The bars of one week row and how many lanes they stack into. Trips and multi-day
 *  entries were a chip in every cell until 2026-10-05: "Berlin-Urlaub mit Luc…" seven
 *  times in a row, cut off each time, for one range. A range is one bar now. */
export function weekSpans(
  dates: string[],
  entries: CalendarEntry[],
  trips: TripPlan[],
): { spans: WeekSpan[]; lanes: number } {
  const last = dates[dates.length - 1];
  const after = last ? shiftDayKey(last, 1) : "";
  const items: Array<Omit<WeekSpan, "lane" | "lanes">> = [];

  for (const trip of trips) {
    if (!trip.date_start || !trip.date_end) continue;
    // A trip's end date is inclusive; a span's end is not.
    const end = shiftDayKey(trip.date_end, 1);
    const span = rangeSpan(trip.date_start, end, dates);
    if (!span) continue;
    items.push({
      ...span,
      key: `trip:${trip.id}`,
      trip,
      label: trip.destinations?.[0]?.name ?? trip.title,
      continuesBefore: trip.date_start < dates[0],
      continuesAfter: end > after,
    });
  }

  for (const entry of entries) {
    if (!isMultiDay(entry)) continue;
    const span = allDaySpan(entry, dates);
    if (!span) continue;
    items.push({
      ...span,
      key: entry.id,
      entry,
      label: entry.title,
      continuesBefore: entry.starts_at.slice(0, 10) < dates[0],
      continuesAfter: entry.ends_at.slice(0, 10) > after,
    });
  }

  // ponytail: no lane cap. A week with many overlapping ranges grows taller; fold the
  // overflow into "+N" if a real month ever needs it.
  const spans = packLanes(items) as WeekSpan[];
  return { spans, lanes: spans.reduce((most, span) => Math.max(most, span.lane + 1), 0) };
}

/** Does the entry touch this day at all? All-day blocks cover their exclusive
 * range; timed blocks cover every day their clock time falls in. */
export function coversDay(entry: CalendarEntry, date: string): boolean {
  if (entry.all_day) {
    return entry.starts_at.slice(0, 10) <= date && date < entry.ends_at.slice(0, 10);
  }
  return timedSpan(entry, date) !== null;
}

/** Greedy lane assignment for overlapping spans. Every span in one cluster of
 * mutual overlaps reports the same `lanes` count, so its members divide the
 * available room evenly instead of each guessing its own width. */
export function packLanes<T extends Span>(items: T[]): Array<T & Lane> {
  const sorted = [...items].sort((a, b) => a.start - b.start || b.end - a.end);
  const placed: Array<T & Lane> = [];
  let cluster: Array<T & Lane> = [];
  let laneEnds: number[] = [];
  let clusterEnd = -Infinity;

  // Widths are only known once the cluster is complete, so they are written
  // back onto the members — the same objects that already went into `placed`.
  function closeCluster() {
    for (const member of cluster) member.lanes = laneEnds.length;
    cluster = [];
    laneEnds = [];
    clusterEnd = -Infinity;
  }

  for (const item of sorted) {
    if (item.start >= clusterEnd) closeCluster();
    let lane = laneEnds.findIndex((end) => end <= item.start);
    if (lane < 0) lane = laneEnds.push(0) - 1;
    laneEnds[lane] = item.end;
    const positioned = { ...item, lane, lanes: 1 };
    cluster.push(positioned);
    placed.push(positioned);
    clusterEnd = Math.max(clusterEnd, item.end);
  }
  closeCluster();

  return placed;
}
