import { describe, expect, test } from "bun:test";
import type { StorageAbsent, StorageHistory, StorageReport, StorageSample } from "../src/lib/api";
import { classTrends, datedAbsences, dayLabel, formatBytes, severity, signedBytes, storageTrend, storageView } from "../src/lib/systems/storage";

const report = (over: Partial<StorageReport> = {}): StorageReport => ({
  disk: { used: 271_100_000_000, free: 164_000_000_000, total: 460_000_000_000, target: "/System/Volumes/Data" },
  state: "ok",
  classes: [],
  protected: [],
  expected_service: [],
  ...over,
});

describe("severity", () => {
  test("passes through the tool's three states", () => {
    expect(severity("ok")).toBe("ok");
    expect(severity("warn")).toBe("warn");
    expect(severity("critical")).toBe("critical");
  });

  // A fourth value (a policy this build predates) must not paint a healthy disk red.
  test("an unknown state reads as ok", () => {
    expect(severity("kritisch")).toBe("ok");
  });
});

describe("formatBytes — the CLI's own unit switch", () => {
  test("switches to GB at exactly 1 GiB", () => {
    expect(formatBytes(1024 ** 3)).toBe("1.0 GB");
    expect(formatBytes(1024 ** 3 - 1)).toBe("1024 MB");
  });

  test("prints sub-megabyte sizes as 0 MB, as the tool's column does", () => {
    expect(formatBytes(0)).toBe("0 MB");
    expect(formatBytes(4096)).toBe("0 MB");
  });

  test("rounds MB to whole numbers", () => {
    expect(formatBytes(780_000_000)).toBe("744 MB");
  });
});

describe("storageView", () => {
  test("sorts classes largest first and drops the zero-byte rows", () => {
    const view = storageView(
      report({
        classes: [
          { name: "small", bytes: 10, applicable: true, flagged: false },
          { name: "empty", bytes: 0, applicable: true, flagged: false },
          { name: "big", bytes: 9_000_000_000, applicable: false, flagged: true },
        ],
      }),
    );
    expect(view.classes.map((row) => row.name)).toEqual(["big", "small"]);
  });

  // The number a reader decides on: `apply` touches applicable classes only, so the
  // report-only class must not inflate it.
  test("reclaimable counts applicable classes only", () => {
    const view = storageView(
      report({
        classes: [
          { name: "regrows", bytes: 744_000_000, applicable: true, flagged: false },
          { name: "report-only", bytes: 5_000_000_000, applicable: false, flagged: true },
        ],
      }),
    );
    expect(view.reclaimable).toBe(744_000_000);
    expect(view.reclaimableLabel).toBe("710 MB");
  });

  test("keeps every protected path, largest first, and separates absent from unreadable", () => {
    const view = storageView(
      report({
        protected: [
          // The four entries really in this state were paths that no longer existed, and the
          // panel used to explain them to the reader as a permissions problem.
          { path: "~/Documents/Staging", bytes: 0, reason: "13 GB staging area", present: false },
          // Live but empty, or unreadable: the tool does not say which, so neither does this.
          { path: "~/Library/Application Support/Claude", bytes: 0, reason: "app-owned" },
          { path: "~/Pictures/Photos Library.photoslibrary", bytes: 66_062_876_672, reason: "iCloud governs it" },
        ],
      }),
    );
    expect(view.protected.map((row) => row.path)).toEqual([
      "~/Pictures/Photos Library.photoslibrary",
      "~/Documents/Staging",
      "~/Library/Application Support/Claude",
    ]);
    expect(view.protectedAbsent).toBe(1);
    expect(view.protectedUnreadable).toBe(1);
  });

  test("a report from before `present` still counts its zero-byte entries as unreadable", () => {
    // Forward compatibility. The field arrived 2026-10-11, and a dashboard reading a report
    // from an older tool must fall back to the guess rather than quietly report nothing.
    const view = storageView(
      report({ protected: [{ path: "~/gone", bytes: 0, reason: "written when it existed" }] }),
    );
    expect(view.protectedAbsent).toBe(0);
    expect(view.protectedUnreadable).toBe(1);
  });

  test("carries the tool's used/total into a percentage and labels", () => {
    const view = storageView(report());
    expect(view.usedPct).toBeCloseTo((271_100_000_000 / 460_000_000_000) * 100, 6);
    expect(view.total).toBe("428.4 GB");
    expect(view.free).toBe("152.7 GB");
  });

  test("renders the reconciliation numbers the tool reports", () => {
    const view = storageView(report({ named_bytes: 40_698_048_512, unattributed_bytes: 202_032_476_160 }));
    expect(view.hasReconciliation).toBe(true);
    expect(view.named).toBe("37.9 GB");
    expect(view.unattributed).toBe("188.2 GB");
  });

  test("a report from before the reconciliation fields omits the line rather than printing zero", () => {
    // `0 MB unattributed` would be a claim the older report never made.
    const view = storageView(report());
    expect(view.hasReconciliation).toBe(false);
  });

  test("a class that could not be measured is kept apart from the sized rows", () => {
    const view = storageView(
      report({
        classes: [
          // `du` failed on the whole path: no size at all, so the sized list would drop it.
          { name: "unreadable", bytes: 0, applicable: true, flagged: false, measured: false },
          // A partial answer keeps its size and is marked by `measured`.
          { name: "partial", bytes: 4_000_000, applicable: true, flagged: false, measured: false },
          { name: "readable", bytes: 8_000_000, applicable: true, flagged: false, measured: true },
        ],
      }),
    );
    expect(view.classes.map((row) => row.name)).toEqual(["readable", "partial"]);
    expect(view.unmeasured.map((row) => row.name)).toEqual(["unreadable"]);
  });

  test("an absent class is separated from a present but unmeasured one", () => {
    const view = storageView(
      report({
        classes: [
          // "Declared but absent": the path is gone. It measures "completely" — nothing to fail on.
          { name: "gone", bytes: 0, applicable: true, flagged: false, present: false, measured: true },
          // Present, but `du` could not read it.
          { name: "unreadable", bytes: 0, applicable: true, flagged: false, present: true, measured: false },
          // Present, readable, and genuinely empty.
          { name: "empty", bytes: 0, applicable: true, flagged: false, present: true, measured: true },
        ],
      }),
    );
    expect(view.classesAbsent.map((row) => row.name)).toEqual(["gone"]);
    expect(view.unmeasured.map((row) => row.name)).toEqual(["unreadable"]);
    expect(view.classes).toHaveLength(0);
  });

  test("names the absent protected paths and the declared services that are not running", () => {
    const view = storageView(
      report({
        protected: [
          { path: "/gone", bytes: 0, reason: "a reason", present: false },
          { path: "/here", bytes: 10, reason: "a reason", present: true },
        ],
        expected_service: [
          { kind: "launchagent", name: "stopped", note: "", presence: "absent" },
          // The probe refusing to answer, which the tool keeps apart from a death.
          { kind: "launchagent", name: "unreadable", note: "", presence: "unverifiable" },
        ],
      }),
    );
    expect(view.protectedAbsentPaths).toEqual(["/gone"]);
    expect(view.servicesAbsent.map((row) => row.name)).toEqual(["stopped"]);
  });

  test("a zero-size volume cannot divide by zero", () => {
    const view = storageView(
      report({ disk: { used: 0, free: 0, total: 0, target: "/System/Volumes/Data" } }),
    );
    expect(view.usedPct).toBe(0);
  });
});

const sample = (hour: string, used: number, unattributed = 0): StorageSample => ({
  hour,
  at: hour,
  used,
  free: 0,
  total: 1,
  named: 0,
  unattributed,
  reclaimable: 0,
  state: "ok",
});

describe("storageTrend", () => {
  test("no history and an empty history both read as no trend", () => {
    expect(storageTrend(null)).toBeNull();
    expect(storageTrend({ retention_days: 90, samples: [] })).toBeNull();
  });

  test("one sample is not yet a line", () => {
    const trend = storageTrend({
      retention_days: 90,
      samples: [sample("2026-10-12 21:00:00.000+00:00", 100)],
    });
    expect(trend?.enough).toBe(false);
    expect(trend?.line).toBe("");
  });

  test("two samples give a slope and a real span", () => {
    const trend = storageTrend({
      retention_days: 90,
      samples: [
        sample("2026-10-12 20:00:00.000+00:00", 100_000_000_000),
        sample("2026-10-12 21:00:00.000+00:00", 101_000_000_000),
      ],
    });
    expect(trend?.enough).toBe(true);
    expect(trend?.delta).toBe(1_000_000_000);
    expect(trend?.deltaLabel).toBe("+954 MB");
    expect(trend?.spanLabel).toBe("1h");
    expect(trend?.line.split(" ")).toHaveLength(2);
  });

  test("a window reads only its last N samples", () => {
    const samples = Array.from({ length: 5 }, (_, i) =>
      sample(`2026-10-12 2${i}:00:00.000+00:00`, i),
    );
    const trend = storageTrend({ retention_days: 90, samples }, 2);
    expect(trend?.samples).toHaveLength(2);
    expect(trend?.delta).toBe(1);
  });

  test("the trend carries the unattributed delta beside the volume's", () => {
    const trend = storageTrend({
      retention_days: 90,
      samples: [
        sample("2026-10-12 20:00:00.000+00:00", 100, 40_000_000_000),
        sample("2026-10-12 21:00:00.000+00:00", 101, 45_000_000_000),
      ],
    });
    expect(trend?.unattributedDelta).toBe(5_000_000_000);
    expect(trend?.unattributedDeltaLabel).toBe("+4.7 GB");
  });

  test("the span is measured from the stamps, not the sample count", () => {
    // A gap where the machine was off: two samples, 24 hours apart. A count would say 1h.
    const trend = storageTrend({
      retention_days: 90,
      samples: [
        sample("2026-10-11 21:00:00.000+00:00", 1),
        sample("2026-10-12 21:00:00.000+00:00", 2),
      ],
    });
    expect(trend?.spanLabel).toBe("1d");
  });
});

describe("datedAbsences — the report joined onto the disk's own history", () => {
  const history = (absent: StorageAbsent[]): StorageHistory => ({ retention_days: 90, samples: [], absent });
  const run = (kind: StorageAbsent["kind"], name: string, since: string, capped = false): StorageAbsent => ({
    kind,
    name,
    since,
    capped,
  });

  test("dates a name the history has an open run for", () => {
    const dated = datedAbsences(
      ["com.apple.dt.Xcode"],
      "class",
      history([run("class", "com.apple.dt.Xcode", "2026-10-12 09:00:00.000+00:00")]),
    );
    expect(dated).toEqual(["com.apple.dt.Xcode (absent since 12 Oct)"]);
  });

  // A run reaching the oldest kept sample cannot say when the name went, only that it was
  // already gone then. Printing the bare date would read as the day it died.
  test("says `or earlier` when the run reaches the oldest kept sample", () => {
    const dated = datedAbsences(
      ["gone"],
      "class",
      history([run("class", "gone", "2026-07-14 09:00:00.000+00:00", true)]),
    );
    expect(dated).toEqual(["gone (absent since 14 Jul or earlier)"]);
  });

  // The case that matters most: a date invented here would be indistinguishable from a real
  // one, and the sentence exists to say how long.
  test("a name the history does not answer for prints bare", () => {
    expect(datedAbsences(["gone"], "class", history([]))).toEqual(["gone"]);
    expect(datedAbsences(["gone"], "class", null)).toEqual(["gone"]);
  });

  test("a response from a build before the absence table prints names without dates", () => {
    expect(datedAbsences(["gone"], "class", { retention_days: 90, samples: [] })).toEqual(["gone"]);
  });

  test("a class and a protected path of the same name are not each other's date", () => {
    const h = history([run("protected", "gone", "2026-10-05 09:00:00.000+00:00")]);
    expect(datedAbsences(["gone"], "class", h)).toEqual(["gone"]);
    expect(datedAbsences(["gone"], "protected", h)).toEqual(["gone (absent since 5 Oct)"]);
  });
});

describe("dayLabel", () => {
  test("reads the day out of the canonical stamp, with no timezone of its own", () => {
    expect(dayLabel("2026-10-12 23:00:00.000+00:00")).toBe("12 Oct");
    expect(dayLabel("2026-01-01 00:00:00.000+00:00")).toBe("1 Jan");
  });

  test("prints an unparseable stamp verbatim rather than Invalid Date", () => {
    expect(dayLabel("whenever")).toBe("whenever");
    expect(dayLabel("2026-13-99 00:00:00.000+00:00")).toBe("2026-13-99 00:00:00.000+00:00");
  });
});

describe("signedBytes", () => {
  test("signs growth, shrink, and a change too small to show", () => {
    expect(signedBytes(1_000_000_000)).toBe("+954 MB");
    expect(signedBytes(-1_000_000_000)).toBe("-954 MB");
    expect(signedBytes(0)).toBe("±0 MB");
    expect(signedBytes(1024)).toBe("±0 MB");
  });
});

describe("classTrends", () => {
  test("computes growth per class across samples", () => {
    const h: StorageHistory = {
      retention_days: 90,
      samples: [],
      classes: [
        { hour: "2026-10-12 10:00:00.000+00:00", name: "rust-debug-cache", bytes: 10_000_000_000 },
        { hour: "2026-10-12 11:00:00.000+00:00", name: "rust-debug-cache", bytes: 12_000_000_000 },
        { hour: "2026-10-12 11:00:00.000+00:00", name: "ollama-models", bytes: 18_000_000_000 },
      ],
    };
    const trends = classTrends(h);
    expect(trends.get("rust-debug-cache")?.deltaLabel).toBe("+1.9 GB");
    expect(trends.has("ollama-models")).toBe(false);
  });
});
