import type { UpdateApply, UpdateOwner, UpdateRow, UpdateStatus, UpdateSurface, UpdatesReport } from "../api";

/**
 * How the updates panel reads `sjel update --json`.
 *
 * The panel draws; this decides. Everything here is a pure function over the tool's own
 * JSON, so the grouping, the stale count and the apply-button enablement are testable
 * without a browser or a running sjel-status (dashboard/vite/updates-panel.test.ts).
 */

/** The four ownership groups, in the order a reader needs them: what is already handled,
 *  what a person has to run, what nothing runs, and what updates itself. */
export const OWNER_ORDER: UpdateOwner[] = ["scheduled", "manual", "unowned", "self"];

/** The tool's own heading for each group, not a second phrasing of it. */
export const OWNER_HEADING: Record<UpdateOwner, string> = {
  scheduled: "Managed by a scheduled job",
  manual: "Manual — a verb exists, nothing schedules it",
  unowned: "Unowned — nothing moves these",
  self: "Self-managed — the vendor updates these",
};

/** One class with its rows, ready to render. */
export interface UpdateGroup {
  surface: UpdateSurface;
  rows: UpdateRow[];
  /** Rows in this class the tool can act on. Zero means no button, whatever the owner. */
  actionable: number;
}

/** The report, reduced to what the panel renders. */
export interface UpdatesView {
  groups: UpdateGroup[];
  stale: number;
  /** Rows the report could not check — `--offline`, or a registry that did not answer.
   *  Counted separately from `stale` on purpose: "not checked" is not "current". */
  unknown: number;
  /** Classes the apply route accepts, for the button. Derived from the tool's own
   *  `actionable` flag rather than a list here, so a class added to the tool appears
   *  without an edit in this file. */
  applyable: string[];
  lastApply: UpdateApply | null;
  /** True while an apply is running: the panel disables the buttons rather than letting a
   *  second one race the first over the same package manager. */
  busy: boolean;
}

/** A status the panel does not know reads as `unknown`, never as `current`: a tool that
 *  grew a fifth status must not have it rendered as a clean bill of health. */
export function statusOf(value: string): UpdateStatus {
  return value === "current" || value === "stale" || value === "unknown" || value === "n/a"
    ? value
    : "unknown";
}

/** The version cell: `0.7.0 → 0.8.2` when both are known, one version otherwise. */
export function versionLabel(row: UpdateRow): string {
  if (!row.installed) return row.latest ?? "";
  if (row.latest && row.latest !== row.installed) return `${row.installed} → ${row.latest}`;
  return row.installed;
}

/** How long ago an apply started, in the units a person reads at a glance. */
export function sinceLabel(at: string | undefined, now: number): string {
  if (!at) return "";
  const ms = now - Date.parse(at);
  if (!Number.isFinite(ms) || ms < 0) return "";
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

export function updatesView(report: UpdatesReport, now: number = Date.now()): UpdatesView {
  const surfaces = report.surfaces ?? [];
  const rows = (report.rows ?? []).map((row) => ({ ...row, status: statusOf(row.status) }));

  const groups: UpdateGroup[] = [];
  for (const owner of OWNER_ORDER) {
    for (const surface of surfaces.filter((s) => s.owner === owner)) {
      const mine = rows.filter((row) => row.surface === surface.id);
      if (mine.length === 0) continue;
      groups.push({
        surface,
        rows: mine,
        // `row.action` and not just `stale`: a package another global pins is stale and
        // deliberately not actionable, and a button counting it would promise work the tool
        // refuses to do. The count is what the button says, so it has to mean the same thing.
        actionable: surface.actionable ? mine.filter((row) => row.status === "stale" && row.action).length : 0,
      });
    }
  }

  const lastApply = report.lastApply ?? null;
  return {
    groups,
    stale: rows.filter((row) => row.status === "stale").length,
    unknown: rows.filter((row) => row.status === "unknown").length,
    applyable: surfaces.filter((s) => s.actionable).map((s) => s.id),
    lastApply,
    busy: lastApply?.state === "running",
  };
}

/** The line under the apply button: what the last run did, or what it is doing. */
export function applySummary(view: UpdatesView, now: number = Date.now()): string {
  const last = view.lastApply;
  if (!last) return "";
  if (last.state === "running") return `${last.class} — applying, started ${sinceLabel(last.at, now)}`;
  const failed = last.failed ?? 0;
  const steps = last.steps ?? 0;
  const still = last.stillStale ?? 0;
  const when = sinceLabel(last.at, now);
  return (
    `${last.class} — ${failed > 0 ? `${failed} of ${steps} failed` : `${steps} step${steps === 1 ? "" : "s"} applied`}` +
    `${still > 0 ? `, ${still} still stale` : ""}${when ? ` · ${when}` : ""}`
  );
}
