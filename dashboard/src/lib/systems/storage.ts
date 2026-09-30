import type { StorageClass, StorageProtected, StorageReport } from "../api";

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
  /** Protected paths the tool could not measure, so the panel can say why they show 0 MB
   *  rather than leaving a reader to wonder if the path is gone. */
  protectedUnmeasured: number;
}

export function storageView(report: StorageReport): StorageView {
  const total = report.disk?.total ?? 0;
  const used = report.disk?.used ?? 0;

  const classes = [...(report.classes ?? [])]
    .filter((row) => row.bytes > 0)
    .sort((a, b) => b.bytes - a.bytes);

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
    protectedUnmeasured: protectedPaths.filter((row) => row.bytes === 0).length,
  };
}
