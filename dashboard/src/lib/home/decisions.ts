/**
 * The Home ladder's contract: what a decision kind is, and how two of them are ordered.
 *
 * This module imports NOTHING at runtime, on purpose. `registry.ts` beside it is the only
 * half that needs Vite (`import.meta.glob`), so this file — and every `kinds/<key>.ts` that
 * implements the interface — stays readable by plain `bun test`. That is the same rule
 * `nav.ts:14-19` follows for `$app/paths`, and the reason `DecisionKind.view` is the NAME
 * of a row component rather than an imported component: a name costs nothing to test, an
 * import drags the Svelte compiler into every unit test that touches a kind.
 *
 * PRD §8.1, "the attention ladder" band table, is the law this file implements. Cited by
 * section and row rather than by line number: the PRD is edited nightly by another writer,
 * and every bare line anchor this file carried had already slipped by eighteen lines a day
 * after it was written. The dashboard owns no tables (dashboard/README.md:7-9), so this
 * interface is the durable contract in a schema's place.
 */

/** The visual spine. Four tones over thirteen bands, because a reader distinguishes four. */
export type BandTone = "alarm" | "now" | "owed" | "offer";

/**
 * The four data classes. c2 and c3 never leave the host and never reach a cloud model.
 *
 * Re-exported from the client rather than redeclared: `api.ts` already owns this union, and
 * a second copy would drift the day comms adds a class. `import type` is erased by both tsc
 * and bun, so this module still imports nothing at runtime.
 */
import type { DataClass } from "../api";
export type { DataClass };

export interface ScoreContext {
  /** Local YYYY-MM-DD. Recomputed at local midnight — a tab left open overnight used to
   *  keep yesterday's `today` and rank every date one day too urgent. */
  todayKey: string;
  /** `todayKey` plus four months, the window the calendar reads. */
  horizonEndKey: string;
  nowMs: number;
  /** Whole days from today. Negative in the past. 365 for an unparseable value. */
  daysUntil(value: string): number;
  /**
   * Another kind's rows, for a kind that declares that kind in `dependsOn`.
   *
   * Returns `[]` when the dependency has not settled or its capability failed, so a kind
   * that reads a peer must still produce a sensible row from nothing. `opportunity` needs
   * this: its gate and both of its rank adjustments read calendar entries and contexts.
   */
  peer<Row>(key: string): readonly Row[];
  /**
   * A dependency's loaded source, before its own gate ran.
   *
   * Additive to `peer`, not a replacement, and needed because `peer` hands back the rows a
   * kind chose to raise. `opportunity` reads calendar entries the calendar kind
   * deliberately does NOT raise — every committed and planned entry, which is what tells
   * an opportunity whether it collides with something already in the diary — and the
   * contexts, which are not entries at all. Returns null until that kind has settled.
   */
  peerSource<Source>(key: string): Source | null;
}

export interface LoadContext extends ScoreContext {
  /** Aborted when Home unmounts or reloads. Clients that accept a signal are handed it;
   *  the rest are simply not awaited into state once this is aborted. */
  signal: AbortSignal;
}

export interface ActOptions<Source> {
  /** False leaves the row on the ladder; the default drops it once the write resolves. */
  dismiss?: boolean;
  /**
   * Rewrites the kind's loaded source, once the write resolves.
   *
   * The ladder is not the only reading of a kind's rows: Locations lists every new
   * opportunity, Sources counts them, and the horizon reads every dated calendar entry.
   * Dropping the row from the ladder alone left all three showing a decision the operator
   * had just made — the page contradicting itself between two tabs. The base page wrote
   * the same patch back by hand for each of its three actions; this is that, once.
   */
  patch?: (source: Source) => Source;
}

export interface DecisionViewProps<Row, Source = unknown> {
  row: Row;
  /** True while this row's own action is in flight. */
  busy: boolean;
  /**
   * Runs a capability write and, only once it RESOLVES, drops the row from the ladder.
   *
   * The order is the point. An optimistic dismissal shows a decision as made that the
   * capability never recorded — the dashboard contradicting the owner of the record. On
   * rejection the row stays and the error surfaces.
   */
  act(run: () => Promise<void>, options?: ActOptions<Source>): void;
}

/**
 * What a row component under `home/rows/` receives.
 *
 * Additive to `DecisionViewProps`: the four extra fields are the shell's, not the row's —
 * where the row sits in the DOM, whether the keyboard cursor is on it, which band tone its
 * spine wears and where its title points. A row renders its own `ListRow` so that it owns
 * its mark, its actions and its meta line; the page owns only the order.
 */
export interface DecisionRowProps<Row, Source = unknown>
  extends DecisionViewProps<Row, Source> {
  /** Stable DOM id, so the keyboard cursor can focus this row. */
  id: string;
  current: boolean;
  tone: BandTone;
  href: string;
  /** The attention contract, already evaluated against the live context.
   *  The kind states it, the page evaluates it, the row only shows it — so no row has to
   *  invent a ScoreContext to ask its own kind a question. */
  whyHere: string;
  dataClass: DataClass | null;
  processingRoute: "local" | "cloud" | null;
  candidateStatus: "proposed" | "accepted" | "open";
}

export interface DecisionKind<Source = unknown, Row = unknown> {
  /** Matches the filename under `home/kinds/`, and prefixes every row key. */
  key: string;
  /** PRD §8.1's own number. NOT unique — see the band table below. */
  band: number;
  /** The name used in the "Unavailable: …" line when `load` rejects. */
  label: string;
  /** The capability this kind reads, or null for sjel-status itself. */
  capability: string | null;
  /** Commitments expire whether or not you look at them; reading never does. */
  lane?: "commitment" | "reading";
  /** Kind keys whose rows this kind reads through `ctx.peer()`. Rows are held back until
   *  each has settled; a failed dependency yields `[]` and `whyHere` names it. */
  dependsOn?: readonly string[];
  /** A component filename under `home/rows/`, e.g. `"MailRow"`. A STRING, not an import:
   *  `registry.ts` resolves it, so this module and every kind stay bun-importable. */
  view: string;

  load(ctx: LoadContext): Promise<Source>;
  /** The gate lives here: a source row that earns no decision is simply not returned. */
  rows(source: Source, ctx: ScoreContext): Row[];
  id(row: Row): string;
  /** The row's own name, for the briefing line at the top of the page. Optional because
   *  a kind with one row (system health) has a fixed sentence instead. */
  title?(row: Row): string;
  /** 0..999. `score()` clamps, so a kind may return an unbounded expression. */
  urgency(row: Row, ctx: ScoreContext): number;
  /** `link(...)` for an internal route, or an absolute URL with `external` set. */
  href(row: Row): string;
  external?(row: Row): boolean;

  // --- the attention contract, field names adopted from the private attention policy ---

  /** Why this row is here, in the row's own words. For a model-ranked kind this is the
   *  model's own rationale, never a restatement of the subject (PRD:227). */
  whyHere(row: Row, ctx: ScoreContext): string;
  /** ISO date or datetime, the first declared tie-breaker. null sorts last. */
  startOrDueAt(row: Row): string | null;
  candidateStatus(row: Row): "proposed" | "accepted" | "open";
  /** null wherever the capability publishes no class. NEVER computed here: the dashboard
   *  owns no data, so inventing a class would be a false provenance claim. */
  dataClass(row: Row): DataClass | null;
  /** null on every kind today. The gap is the capabilities', not this layer's. */
  processingRoute(row: Row): "local" | "cloud" | null;
  /**
   * The named terms behind this row's urgency, for a row component that wants to draw them.
   *
   * Declared on the contract and read by no shell: the page hands a row its own `row` and
   * lets the row decide what to render, and the one kind that fills this in today — `feed`
   * — reaches the same numbers through `FeedItemRow`'s `FactorBars`. Kept rather than
   * dropped because `finance`'s kind already fills it, and a row that wants a breakdown
   * should get it from its kind rather than recompute one.
   */
  scoreFactors?(row: Row, ctx: ScoreContext): { label: string; score: number }[];
}

export interface Decision<Row = unknown> {
  /** `${kind.key}:${kind.id(row)}` — stable across polls, which is what `dismissed` keys on. */
  key: string;
  kind: DecisionKind<unknown, Row>;
  row: Row;
  priority: number;
  startOrDueAt: string | null;
}

/**
 * The urgency resolution budget: three significant digits.
 *
 * NOT a guard against crossing the tightest band gap. Clamping is what makes the band
 * decisive — once urgency cannot reach the stride, `(b-1)·S + (S-1) < b·S` holds for every
 * S >= 1, including PRD's tightest 640/630/620 run. The stride only decides how finely
 * urgency can rank rows INSIDE one band.
 */
export const BAND_STRIDE = 1000;
export const MAX_URGENCY = BAND_STRIDE - 1;

export const score = (band: number, urgency: number): number =>
  band * BAND_STRIDE + Math.min(MAX_URGENCY, Math.max(0, Math.round(urgency)));

/** Shape-preserving rescale of a kind's own expression onto 0..999. */
export const RESCALE = (value: number, max: number): number =>
  Math.min(MAX_URGENCY, Math.max(0, Math.round((MAX_URGENCY * value) / max)));

/** Earlier first; a row with no date sorts last. */
export const compareStartOrDue = (a: string | null, b: string | null): number => {
  if (a === b) return 0;
  if (a === null) return 1;
  if (b === null) return -1;
  return a < b ? -1 : 1;
};

/** Later first; a row with no date still sorts last. */
export const compareNewestFirst = (a: string | null, b: string | null): number => {
  if (a === b) return 0;
  if (a === null) return 1;
  if (b === null) return -1;
  return a > b ? -1 : 1;
};

/**
 * Highest priority first, then the two declared tie-breakers in order.
 *
 * Ties are routine rather than rare once urgency is clamped: every undated task lands on
 * the same number and so does every unscored mail. Without a declared tie-break, intra-band
 * order was whatever the capability happened to return that poll, and the ladder visibly
 * reshuffled between fifteen-second refreshes.
 *
 * The direction of the date tie-break follows the LANE, because the lane is already the
 * statement of what the date means. A commitment's `startOrDueAt` is a deadline and the
 * soonest one is the most urgent — the declared "earliest_start_or_due_at". The reading
 * lane's is an arrival time (`created_at`), and "earliest first" there put the six OLDEST
 * unread articles in the collapsed preview of a list comms itself serves newest-first.
 * Rows in the two lanes are never compared for a result the reader sees: the ladder is
 * partitioned by lane after this sort, and a partition keeps relative order.
 */
const newestFirst = (decision: Decision): boolean => decision.kind.lane === "reading";

export const compareDecisions = (a: Decision, b: Decision): number =>
  b.priority - a.priority ||
  (newestFirst(a) && newestFirst(b)
    ? compareNewestFirst(a.startOrDueAt, b.startOrDueAt)
    : compareStartOrDue(a.startOrDueAt, b.startOrDueAt)) ||
  (a.key < b.key ? -1 : a.key > b.key ? 1 : 0);

/**
 * The band table. PRD §8.1's own numbers, plus 610 for the trip retrospective (§8.2).
 *
 * Every row cites a PRD SECTION and the row's own wording, never a line number. The PRD is
 * edited by another writer most nights; the seven bare line anchors this table carried had
 * slipped by eighteen lines within a day of being written, so each one resolved to
 * unrelated text. A section and a row title stay findable after an edit.
 *
 * band  | key                | owner stream       | PRD row
 * ------|--------------------|--------------------|--------------------------------------
 * 10000 | system             | dashboard-refresh  | §8.1 System health
 *   900 | host               | dashboard-refresh  | §9 resource rule broken
 *   800 | trip               | dashboard-refresh  | §8.1 Trip needing planning
 *   700 | calendar           | dashboard-refresh  | §8.1 Calendar `possible`
 *   640 | finance            | finance-invest     | §8.1 Purchase decision     (SHARED)
 *   640 | (open)             | —                  | §8.1 purchase renewal / wishlist
 *   640 | (open)             | —                  | §13.1 budget overrun
 *   630 | (open)             | —                  | §8.1 stalled project
 *   620 | task               | dashboard-refresh  | §8.1 Task
 *   610 | trip-retrospective | travel-season-cost | ADDITION, §8.2
 *   600 | opportunity        | dashboard-refresh  | §8.1 Opportunity
 *   550 | mail               | dashboard-refresh  | §8.1 Mail
 *   540 | people             | people-entities    | §8.1 person contact frequency
 *   500 | feed               | dashboard-refresh  | §8.1 Feed item
 *   490 | (open)             | —                  | §8.1 note due for review
 *
 * Bands are NOT unique. §8.1 gives 640 to the purchase decision and §13.1 routes a doubled
 * month of metered spend to the same band, so a band is a rank and not a slot. A kind that
 * joins an occupied band is correct; a kind that invents a band this table does not name
 * declares the row it extends in its own file, on a line of the exact form
 *
 *     BAND <band> EXTENDS PRD <section and row>
 *
 * with the same number the kind sets. `tools/dashboard-home-registry.test.ts` asks for
 * exactly that form, because the substring "PRD" alone appears in every kind file already
 * and exempted anything that copied one.
 */
export const PRD_BANDS: readonly number[] = [
  10_000, 900, 800, 700, 640, 630, 620, 610, 600, 550, 540, 500, 490,
];

export const bandTone = (band: number): BandTone =>
  band >= 900 ? "alarm" : band >= 700 ? "now" : band >= 610 ? "owed" : "offer";

export const bandLabel = (band: number): string =>
  band >= 900 ? "Needs attention now" : band >= 700 ? "Dated commitments" : band >= 610 ? "Owed" : "Offered";
