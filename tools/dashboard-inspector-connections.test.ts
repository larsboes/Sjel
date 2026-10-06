// The inspector's links point at the item, not at the page it lives on.
//
// Before 2026-10-06 every "Open in …" chip went to a section root (`/finance`, `/travel`),
// so following a connection dropped the reader on an overview and the connection was lost.
// `deepLink` must carry the id each route already reads, and the date join must be
// inclusive, because a trip's last day is still the trip.

import { describe, expect, test } from "bun:test";
import {
  deepLink,
  transactionItem,
  tripItem,
  withinDays,
} from "../dashboard/src/lib/inspector/connections.ts";
import type { FinanceTransaction, TripPlan } from "../dashboard/src/lib/api.ts";

describe("deepLink", () => {
  test("a trip opens its own plan", () => {
    expect(deepLink({ type: "trip", id: "p 1", title: "", destination: "", dates: "" })).toBe("/travel?plan=p%201");
  });

  test("an event opens its day with the entry selected", () => {
    expect(deepLink({ type: "event", id: "e1", title: "", startsAt: "2026-10-06T09:00:00Z" })).toBe(
      "/calendar?date=2026-10-06&entry=e1",
    );
  });

  test("an event without an id still opens its day", () => {
    expect(deepLink({ type: "event", title: "", startsAt: "2026-10-06" })).toBe("/calendar?date=2026-10-06");
  });
});

describe("withinDays", () => {
  test("both ends are inside", () => {
    expect(withinDays("2026-10-01", "2026-10-01", "2026-10-03")).toBe(true);
    expect(withinDays("2026-10-03", "2026-10-01", "2026-10-03")).toBe(true);
    expect(withinDays("2026-10-04", "2026-10-01", "2026-10-03")).toBe(false);
  });
});

describe("mapping", () => {
  test("a transaction keeps its trip id so the inspector can follow it", () => {
    const row = {
      id: "t1", date: "2026-10-02", description: "DB Fernverkehr", kind: "expense",
      account: "assets:bank", category: "expenses:travel:rail", amount_cents: 4990, currency: "EUR",
      trip_id: "p1",
    } as FinanceTransaction;
    const item = transactionItem(row);
    expect(item).toMatchObject({ type: "transaction", trip: "p1", amount: "−49.90 EUR", category: "travel · rail" });
  });

  test("a trip without a budget states none rather than zero", () => {
    const plan = {
      id: "p1", title: "Berlin", destinations: [{ name: "Berlin" }], date_start: "2026-10-01",
      date_end: "2026-10-03", travelers: [], budget_cents: null, currency: null,
    } as unknown as TripPlan;
    expect(tripItem(plan)).toMatchObject({ dates: "2026-10-01 – 2026-10-03", budget: undefined, companions: undefined });
  });
});
