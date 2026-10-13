import type { StorageAbsent, StorageClass, StorageHistory, StorageProtected, StorageReport, StorageSample } from "../api";

/**
 * How the storage panel reads `sjel storage report --json`.
 *
 * The panel draws; this decides. Everything here is a pure function over the tool's own
 * JSON, so the ordering, the unit switch and the reclaimable sum are testable without a
 * browser or a running sjel-status (dashboard/vite/storage-panel.test.ts).
 */

/** `ok` · `warn` · `critical` — the VOLUME's state, never a class being large. */
export type Severity = "ok" | "warn" | "critical";

/**
 * An unknown state reads as `ok`, deliberately. The tool emits only the three above, and a
 * fourth value arriving (a policy the panel predates) must not paint a healthy disk red —
 * the class rows and the free-space number still say what is true.
 */
export function severity(state: string): Severity {
  return state === "critical" || state === "warn" ? state : "ok";
}

/**
 * Bytes in the units the storage CLI prints: MB below 1 GB, GB above it.
 *
 * This is `fmt_bytes` in tools/storage/src/measure.rs, and it is the same switch on
 * purpose — a reader comparing a class here against `sjel storage report` in a terminal
 * has to see one number, not two that differ by a rounding rule. Sub-megabyte sizes print
 * as `0 MB` for the same reason: that is what the tool's column says.
 */
export function formatBytes(bytes: number): string {
  const GB = 1024 ** 3;
  if (bytes >= GB) return `${(bytes / GB).toFixed(1)} GB`;
  return `${Math.round(bytes / 1024 ** 2)} MB`;
}

/** The report, reduced to what the panel renders and in the order it renders it. */
export interface StorageView {
  usedPct: number;
  used: string;
  total: string;
  free: string;
  state: Severity;
  /** The tool's own word for the volume, shown verbatim so a state this build does not
   *  know still reads as what the policy actually said. */
  stateLabel: string;
  /** Classes with a measured size, largest first — the tool skips empty ones in its own
   *  text report, so a row here means the same thing it means there. */
  classes: StorageClass[];
  /** The total `apply` would actually reclaim: applicable classes only. */
  reclaimable: number;
  reclaimableLabel: string;
  /** Every protected path, largest first. Kept even at 0 MB, because the reason is the
   *  point — these are the paths a reader must NOT clean, whatever they measure today. */
  protected: StorageProtected[];
  /** Protected paths the tool says are not on this machine. The reason beside one of these
   *  was written when the path still existed, which is why it has to be separable from a path
   *  that is merely empty. */
  protectedAbsent: number;
  /** Protected paths measuring 0 bytes without being known-absent: a live but empty
   *  directory, or a path the measuring user cannot read. The tool does not distinguish
   *  those two, so neither does this. */
  protectedUnreadable: number;
  /** Classes the tool could not measure at all: the path is here and `du` returned nothing.
   *  Kept apart from `classes` because the tool keeps them apart — before `measured`, a row
   *  in this state was skipped by the sized list and vanished from the panel too. */
  unmeasured: StorageClass[];
  /** Classes whose paths no longer exist — "Declared but absent" in the report. They carry
   *  no size, so the sized list drops them; the panel names them instead of staying silent. */
  classesAbsent: StorageClass[];
  /** The absent protected paths themselves, not just how many: the panel names each one, and
   *  the disk's own history dates it. */
  protectedAbsentPaths: string[];
  /** Declared services the machine answers `absent` for. `unverifiable` is deliberately not
   *  here — that is the probe refusing to answer, which the tool keeps apart and so does this. */
  servicesAbsent: Array<{ kind: string; name: string }>;
  /** False for a report from a build before the reconciliation fields, so the panel omits
   *  the line rather than printing a `0 MB` that means "not reported". */
  hasReconciliation: boolean;
  /** `used` minus everything a class or a protection accounts for, in the tool's units. */
  named: string;
  unattributed: string;
}

export function storageView(report: StorageReport): StorageView {
  const total = report.disk?.total ?? 0;
  const used = report.disk?.used ?? 0;

  const classes = [...(report.classes ?? [])]
    .filter((row) => row.bytes > 0)
    .sort((a, b) => b.bytes - a.bytes);

  // `present === false` is the tool's own answer that the path is gone. It is checked before
  // `measured`, because an absent class measures "completely" (there is nothing to fail on).
  const classesAbsent = [...(report.classes ?? [])]
    .filter((row) => row.present === false)
    .sort((a, b) => a.name.localeCompare(b.name));

  // `measured === false` is the tool's own answer. A zero-byte row in that state would be
  // dropped by the filter above; a partial row stays in `classes` and carries the flag.
  const unmeasured = [...(report.classes ?? [])]
    .filter((row) => row.present !== false && row.measured === false && row.bytes === 0)
    .sort((a, b) => a.name.localeCompare(b.name));

  const protectedPaths = [...(report.protected ?? [])].sort((a, b) => b.bytes - a.bytes);

  const reclaimable = classes
    .filter((row) => row.applicable)
    .reduce((sum, row) => sum + row.bytes, 0);

  return {
    usedPct: total > 0 ? (used / total) * 100 : 0,
    used: formatBytes(used),
    total: formatBytes(total),
    free: formatBytes(report.disk?.free ?? 0),
    state: severity(report.state),
    stateLabel: report.state,
    classes,
    reclaimable,
    reclaimableLabel: formatBytes(reclaimable),
    protected: protectedPaths,
    // `present === false` is the tool's own answer, added 2026-10-11. Before it, this counted
    // every 0-byte entry and the panel called them unreadable — but all four entries actually
    // in that state were paths that no longer existed, so the panel explained a permissions
    // problem that was not there. A report from an older build omits the field, so the old
    // guess survives as the fallback rather than the rule.
    protectedAbsent: protectedPaths.filter((row) => row.present === false).length,
    protectedUnreadable: protectedPaths.filter(
      (row) => row.bytes === 0 && row.present !== false,
    ).length,
    protectedAbsentPaths: protectedPaths
      .filter((row) => row.present === false)
      .map((row) => row.path),
    servicesAbsent: (report.expected_service ?? [])
      .filter((row) => row.presence === "absent")
      .map((row) => ({ kind: row.kind, name: row.name }))
      .sort((a, b) => a.name.localeCompare(b.name)),
    unmeasured,
    classesAbsent,
    hasReconciliation:
      report.named_bytes !== undefined && report.unattributed_bytes !== undefined,
    named: formatBytes(report.named_bytes ?? 0),
    unattributed: formatBytes(report.unattributed_bytes ?? 0),
  };
}

/** The disk's own history joined onto the names the report says are gone.
 *
 * The report is a point and the history is a line: `report` says a class's path is not there,
 * and only `storage_absent` says since when. The join is by name and kind, and a name the
 * history does not answer for prints bare — one that went away within the hour, or a machine
 * whose sjel-status predates the table. A guessed date would be worse than none, because the
 * sentence's whole job is to say how long. A run that reaches the oldest kept sample reads
 * `or earlier`: the record has not been kept long enough to say when it actually went. */
export function datedAbsences(
  names: string[],
  kind: StorageAbsent["kind"],
  history: StorageHistory | null,
): string[] {
  return names.map((name) => {
    const row = history?.absent?.find((entry) => entry.kind === kind && entry.name === name);
    if (!row) return name;
    return `${name} (absent since ${dayLabel(row.since)}${row.capped ? " or earlier" : ""})`;
  });
}

const MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

/**
 * `2026-10-12 21:00:00.000+00:00` → `12 Oct`.
 *
 * Sliced out of the stamp rather than parsed with `Date`, on purpose: the stamp is canonical
 * UTC and a reader's own zone would render a day earlier or later than the hour the sample is
 * keyed on. The parts are what the sampler wrote, so the day shown is the day it recorded. An
 * unparseable stamp is printed verbatim — the record itself is the evidence, and a label that
 * says `Invalid Date` would hide it.
 */
export function dayLabel(stamp: string): string {
  const found = /^(\d{4})-(\d{2})-(\d{2})/.exec(stamp);
  const month = found?.[2];
  const date = found?.[3];
  const name = month === undefined ? undefined : MONTHS[Number(month) - 1];
  if (date === undefined || name === undefined) return stamp;
  return `${Number(date)} ${name}`;
}

/** How many hourly samples a trend reads by default: one week. */
export const TREND_WINDOW_HOURS = 168;

export interface StorageTrend {
  /** The samples in the window, oldest first. */
  samples: StorageSample[];
  /** True once the window holds two readings, so a line and a slope mean anything. */
  enough: boolean;
  /** Change in `used` across the window. Meaningless when `enough` is false. */
  delta: number;
  /** `+12.3 GB`, `-1.0 GB` or `±0 MB`. */
  deltaLabel: string;
  /** Change in the space no policy path names across the same window. */
  unattributedDelta: number;
  unattributedDeltaLabel: string;
  /** How much time the window actually covers, e.g. `7d` or `3h`. */
  spanLabel: string;
  /** An SVG `points` string in a 100×24 viewBox, used bytes as the line. */
  line: string;
}

/** Reduce the hourly history to what the panel draws. Pure, so the shape is testable
 *  without a browser — the same reason `storageView` is. */
export function storageTrend(
  history: StorageHistory | null,
  windowHours = TREND_WINDOW_HOURS,
): StorageTrend | null {
  const all = history?.samples ?? [];
  if (all.length === 0) return null;
  const samples = all.slice(-windowHours);
  const first = samples[0];
  const last = samples[samples.length - 1];
  const enough = samples.length >= 2;
  return {
    samples,
    enough,
    delta: last.used - first.used,
    deltaLabel: signedBytes(last.used - first.used),
    unattributedDelta: last.unattributed - first.unattributed,
    unattributedDeltaLabel: signedBytes(last.unattributed - first.unattributed),
    // The window's real span, not its sample count: an hour the machine was off leaves a
    // gap, and a count would quietly claim the line covers less time than it does.
    spanLabel: spanLabel(hoursBetween(first.hour, last.hour)),
    line: enough ? sparkline(samples.map((s) => s.used)) : "",
  };
}

/** `+12.3 GB`, `-1.0 GB`, `±0 MB`. ASCII signs, because the label sits in a monospace
 *  column beside a byte count and a typographic minus would change the column's width. */
export function signedBytes(bytes: number): string {
  const size = formatBytes(Math.abs(bytes));
  if (size === "0 MB") return "±0 MB";
  return `${bytes > 0 ? "+" : "-"}${size}`;
}

/** The canonical stamps are `2026-10-12 21:00:00.000+00:00`; `Date.parse` wants a `T`. */
function hoursBetween(a: string, b: string): number {
  const at = (s: string) => Date.parse(s.replace(" ", "T"));
  const hours = (at(b) - at(a)) / 3_600_000;
  return Number.isFinite(hours) && hours > 0 ? Math.round(hours) : 0;
}

function spanLabel(hours: number): string {
  if (hours < 24) return `${hours}h`;
  return `${Math.round(hours / 24)}d`;
}

/** Values into an SVG `points` string for a 100×24 viewBox, higher value higher on screen. */
function sparkline(values: number[], width = 100, height = 24): string {
  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;
  const pad = 3;
  const w = width - pad * 2;
  const h = height - pad * 2;
  const step = w / (values.length - 1);
  return values
    .map(
      (v, i) =>
        `${(pad + i * step).toFixed(1)},${(pad + h - ((v - min) / range) * h).toFixed(1)}`,
    )
    .join(" ");
}
