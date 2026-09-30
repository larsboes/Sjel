import { describe, expect, test } from "bun:test";
import type { StorageReport } from "../src/lib/api";
import { formatBytes, severity, storageView } from "../src/lib/systems/storage";

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

  test("keeps every protected path, largest first, and counts the unmeasured", () => {
    const view = storageView(
      report({
        protected: [
          { path: "~/Library/Application Support/Claude", bytes: 0, reason: "app-owned" },
          { path: "~/Pictures/Photos Library.photoslibrary", bytes: 66_062_876_672, reason: "iCloud governs it" },
        ],
      }),
    );
    expect(view.protected.map((row) => row.path)).toEqual([
      "~/Pictures/Photos Library.photoslibrary",
      "~/Library/Application Support/Claude",
    ]);
    expect(view.protectedUnmeasured).toBe(1);
  });

  test("a zero-size volume cannot divide by zero", () => {
    const view = storageView(
      report({ disk: { used: 0, free: 0, total: 0, target: "/System/Volumes/Data" } }),
    );
    expect(view.usedPct).toBe(0);
  });

  test("carries the tool's used/total into a percentage and labels", () => {
    const view = storageView(report());
    expect(view.usedPct).toBeCloseTo((271_100_000_000 / 460_000_000_000) * 100, 6);
    expect(view.total).toBe("428.4 GB");
    expect(view.free).toBe("152.7 GB");
  });
});
