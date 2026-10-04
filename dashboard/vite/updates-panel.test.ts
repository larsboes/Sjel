import { describe, expect, test } from "bun:test";
import type { UpdateRow, UpdateSurface, UpdatesReport } from "../src/lib/api";
import {
  applySummary,
  auditLabel,
  auditNeedsAttention,
  OWNER_ORDER,
  sinceLabel,
  statusOf,
  updatesView,
  versionLabel,
} from "../src/lib/systems/updates";

const surface = (over: Partial<UpdateSurface> & { id: string; owner: UpdateSurface["owner"] }): UpdateSurface => ({
  title: over.id,
  ownerDetail: "somewhere",
  actionable: false,
  why: "because",
  ...over,
});

const row = (over: Partial<UpdateRow> & { surface: string }): UpdateRow => ({
  name: "thing",
  owner: "unowned",
  ownerDetail: "",
  status: "current",
  ...over,
});

const report = (over: Partial<UpdatesReport> = {}): UpdatesReport => ({
  generatedAt: "2026-10-01T00:00:00Z",
  offline: false,
  lastApply: null,
  surfaces: [
    surface({ id: "brew", owner: "scheduled", title: "Homebrew", actionable: true }),
    surface({ id: "graphify", owner: "manual", title: "graphify", actionable: true }),
    surface({ id: "cargo", owner: "unowned", title: "cargo", actionable: true }),
    surface({ id: "npm", owner: "unowned", title: "npm", actionable: true }),
    surface({ id: "vendor", owner: "self", title: "vendor", actionable: false }),
  ],
  rows: [],
  ...over,
});

describe("statusOf", () => {
  test("passes the tool's four statuses through", () => {
    for (const s of ["current", "stale", "unknown", "n/a"] as const) expect(statusOf(s)).toBe(s);
  });

  // A fifth status (a tool this build predates) must not render as a clean bill of health.
  test("an unknown status reads as unknown, never as current", () => {
    expect(statusOf("veraltet")).toBe("unknown");
    expect(statusOf("")).toBe("unknown");
  });
});

describe("versionLabel", () => {
  test("shows the move when both versions are known and differ", () => {
    expect(versionLabel(row({ surface: "cargo", installed: "0.7.0", latest: "0.8.2" }))).toBe("0.7.0 → 0.8.2");
  });

  test("shows one version when they agree, so a current row is not noise", () => {
    expect(versionLabel(row({ surface: "cargo", installed: "0.8.2", latest: "0.8.2" }))).toBe("0.8.2");
  });

  test("shows nothing when neither is known", () => {
    expect(versionLabel(row({ surface: "brew" }))).toBe("");
  });
});

describe("sinceLabel", () => {
  const at = "2026-10-01T12:00:00Z";
  const t = Date.parse(at);
  test("switches unit as the gap grows", () => {
    expect(sinceLabel(at, t + 30_000)).toBe("just now");
    expect(sinceLabel(at, t + 5 * 60_000)).toBe("5m ago");
    expect(sinceLabel(at, t + 3 * 3_600_000)).toBe("3h ago");
    expect(sinceLabel(at, t + 2 * 86_400_000)).toBe("2d ago");
  });

  // A clock skew between the host and the browser must not print "−3m ago".
  test("a timestamp in the future prints nothing rather than a negative", () => {
    expect(sinceLabel(at, t - 60_000)).toBe("");
    expect(sinceLabel(undefined, t)).toBe("");
    expect(sinceLabel("not a date", t)).toBe("");
  });
});

describe("updatesView", () => {
  test("groups by owner in the order a reader needs, and counts the stale rows", () => {
    const view = updatesView(
      report({
        rows: [
          row({ surface: "brew", name: "nettle", status: "stale", installed: "3.10", latest: "3.10.1" }),
          row({ surface: "cargo", name: "macmon", status: "stale", installed: "0.7.0", latest: "0.8.2" }),
          row({ surface: "vendor", name: "pi", status: "n/a" }),
        ],
      }),
    );
    expect(view.groups.map((g) => g.surface.owner)).toEqual(["scheduled", "unowned", "self"]);
    expect(view.stale).toBe(2);
  });

  test("a surface with no rows is not rendered at all", () => {
    const view = updatesView(report({ rows: [row({ surface: "cargo", status: "stale" })] }));
    expect(view.groups.map((g) => g.surface.id)).toEqual(["cargo"]);
  });

  // The button is offered per class and only where the tool says it can act. A `self` class
  // is never actionable: that would be a second updater for a binary that has one.
  test("only actionable classes with stale rows get a button", () => {
    const view = updatesView(
      report({
        rows: [
          row({ surface: "brew", status: "current" }),
          row({ surface: "cargo", status: "stale", action: "cargo install thing --locked --force" }),
          row({ surface: "vendor", status: "stale", action: "never" }),
        ],
      }),
    );
    const byId = new Map(view.groups.map((g) => [g.surface.id, g]));
    expect(byId.get("cargo")!.actionable).toBe(1);
    expect(byId.get("brew")!.actionable).toBe(0);
    expect(byId.get("vendor")!.actionable).toBe(0);
    expect(view.applyable).not.toContain("vendor");
  });

  test("a stale row the tool will not act on is not counted by the button", () => {
    const view = updatesView(
      report({
        rows: [
          row({ surface: "npm", name: "pinned", status: "stale", note: "pinned by other — upgrade those instead" }),
          row({ surface: "npm", name: "free", status: "stale", action: "npm install -g free@latest" }),
        ],
      }),
    );
    // Two stale, one button. The count is what the button promises, so it must match.
    expect(view.stale).toBe(2);
    expect(view.groups.find((g) => g.surface.id === "npm")!.actionable).toBe(1);
  });

  test("unknown is counted apart from stale, because not-checked is not current", () => {
    const view = updatesView(
      report({
        offline: true,
        rows: [
          row({ surface: "cargo", status: "unknown" }),
          row({ surface: "cargo", name: "other", status: "unknown" }),
          row({ surface: "brew", status: "current" }),
        ],
      }),
    );
    expect(view.unknown).toBe(2);
    expect(view.stale).toBe(0);
  });

  test("a running apply disables the buttons, so two cannot race one package manager", () => {
    const view = updatesView(
      report({
        lastApply: { at: "2026-10-01T12:00:00Z", class: "cargo", steps: 3, state: "running" },
        rows: [row({ surface: "cargo", status: "stale" })],
      }),
    );
    expect(view.busy).toBe(true);
    expect(updatesView(report()).busy).toBe(false);
  });

  test("an empty payload renders nothing rather than throwing", () => {
    const view = updatesView({ generatedAt: "", offline: false, lastApply: null, surfaces: [], rows: [] });
    expect(view.groups).toEqual([]);
    expect(view.stale).toBe(0);
  });

  test("every owner in the order has a heading, so no group renders headless", () => {
    expect(OWNER_ORDER).toHaveLength(4);
  });
});

describe("applySummary", () => {
  const now = Date.parse("2026-10-01T12:05:00Z");
  test("says what a finished run did, including what it did not fix", () => {
    const view = updatesView(
      report({
        lastApply: { at: "2026-10-01T12:00:00Z", class: "npm", steps: 11, failed: 1, stillStale: 2, state: "failed" },
      }),
    );
    const line = applySummary(view, now);
    expect(line).toContain("npm");
    expect(line).toContain("1 of 11 failed");
    expect(line).toContain("2 still stale");
    expect(line).toContain("5m ago");
  });

  test("says it is still applying rather than showing a stale result", () => {
    const view = updatesView(
      report({ lastApply: { at: "2026-10-01T12:00:00Z", class: "cargo", steps: 3, state: "running" } }),
    );
    expect(applySummary(view, now)).toContain("applying");
  });

  test("no receipt is no line", () => {
    expect(applySummary(updatesView(report()), now)).toBe("");
  });
});

describe("the audit verdict on the summary line", () => {
  const now = Date.parse("2026-10-01T12:05:00Z");
  const applied = (audit?: string) =>
    updatesView(
      report({ lastApply: { at: "2026-10-01T12:00:00Z", class: "cargo", steps: 1, state: "done", audit } }),
    );

  // The point of printing `clean` rather than staying silent: a missing audit and a passing one
  // must not look the same, because the seam this closes is `apply --only cargo` installing
  // software that nothing used to check.
  test("a clean verdict is shown, not swallowed", () => {
    expect(applySummary(applied("clean"), now)).toContain("audit clean");
  });

  test("a verdict that needs attention says what to run", () => {
    for (const verdict of ["finding(s)", "scanner-missing", "could not run"]) {
      const line = applySummary(applied(verdict), now);
      expect(line).toContain(`audit ${verdict}`);
      expect(line).toContain("run tools/audit");
    }
  });

  test("a verdict this file has never heard of reads as something to look at", () => {
    expect(auditLabel("something-new")).toBe(" · audit something-new — run tools/audit");
    expect(applySummary(applied("something-new"), now)).toContain("audit something-new");
  });

  test("a receipt written before the field existed says nothing about an audit", () => {
    expect(applySummary(applied(undefined), now)).not.toContain("audit");
    expect(auditLabel(undefined)).toBe("");
  });

  // Only `clean` is good news. This drives the summary line's warning colour, so an unknown
  // verdict must colour it rather than pass as a clean bill of health.
  test("only clean is treated as fine, and an absent field is not a warning", () => {
    expect(auditNeedsAttention("clean")).toBe(false);
    expect(auditNeedsAttention(undefined)).toBe(false);
    expect(auditNeedsAttention("finding(s)")).toBe(true);
    expect(auditNeedsAttention("scanner-missing")).toBe(true);
    expect(auditNeedsAttention("something-new")).toBe(true);
  });
});
