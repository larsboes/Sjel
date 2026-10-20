/**
 * The Home ladder's "trip closed, retrospective open" decision kind.
 *
 * Band 610, an ADDITION to PRD §8.1's table, ruled by §8.2: "when a trip
 * closes, the dashboard raises one decision with three fields". It sits between
 * `task` (620) and `opportunity` (600) — a closed trip is a small, bounded thing
 * to answer, and it is never allowed to outrank a task that is actually due.
 *
 * The 45-day expiry is NOT here. It lives in
 * `capabilities/trips/src/store.rs::pending_retrospectives`, which is the route
 * this loader calls, so the rule has exactly one home: a retrospective filled
 * from a memory that is gone is a fabrication, and that judgement belongs to the
 * capability that owns the record.
 *
 * Relative imports throughout, so this module resolves under plain `bun test` —
 * the reason `tools/dashboard-nav-links.test.ts` records for `nav.ts`.
 */
import { link } from "../../nav";
import {
  pendingRetrospectives,
  type PendingRetrospective,
  type PendingRetrospectives,
} from "../../travel/api";

import type { DecisionKind, LoadContext, ScoreContext, DataClass } from "../decisions";
import { dayLabel } from "../../ui/age";

/** Used only when the response omits the window, which the route does not do.
 *  The number that matters is `source.window_days`, read below and carried onto
 *  each row — `urgency` is handed a row and no source, so the value has to
 *  travel with the row or it is restated here and drifts. */
const FALLBACK_WINDOW_DAYS = 45;

/** A pending row plus the window it was measured against. */
type PendingRow = PendingRetrospective & { window_days: number };

const kind: DecisionKind<PendingRetrospectives, PendingRow> = {
  key: "trip-retrospective",
  band: 610,
  label: "Trip retrospectives",
  capability: "trips",
  lane: "commitment",
  view: "TripRetrospectiveRow",

  // The route takes no parameters at all: what the ladder should raise today is
  // a judgement the capability owns, not one the caller narrows.
  load: () => pendingRetrospectives(),

  rows: (source: PendingRetrospectives) =>
    source.pending.map((row) => ({
      ...row,
      // From the response, not from a constant here: the 45 days live in
      // `capabilities/trips/src/store.rs::pending_retrospectives` and changing
      // them there must move this ramp with them.
      window_days: source.window_days > 0 ? source.window_days : FALLBACK_WINDOW_DAYS,
    })),

  id: (row: PendingRow) => row.plan_id,

  /**
   * Rises as the window runs out, saturating at its end.
   *
   * The shape is deliberate: a trip that closed yesterday is worth asking about
   * and is not urgent, while one at day 44 is the last chance before the prompt
   * expires and the answer stops being a memory.
   */
  urgency: (row: PendingRow) => {
    const window = row.window_days;
    const elapsed = Math.max(0, Math.min(row.days_since_close, window));
    return Math.round((999 * elapsed) / window);
  },

  href: (row: PendingRow) => link(`/travel?plan=${encodeURIComponent(row.plan_id)}`),

  whyHere: (row: PendingRow) =>
    row.destinations.length > 0
      ? `Back from ${row.destinations.join(", ")} ${dayLabel(row.days_since_close)}; no retrospective yet`
      : `Closed ${dayLabel(row.days_since_close)}; no retrospective yet`,

  /** The trip's end date: the moment that made this a decision. */
  startOrDueAt: (row: PendingRow) => row.date_end,

  candidateStatus: () => "open",

  dataClass: (row: PendingRow) => (row.data_class as DataClass | undefined) ?? null,

  processingRoute: () => null,
};

export default kind;
