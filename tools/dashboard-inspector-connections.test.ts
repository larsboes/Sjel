// The inspector's links point at the item, not at the page it lives on.
//
// Before 2026-10-06 every "Open in …" chip went to a section root (`/finance`, `/travel`),
// so following a connection dropped the reader on an overview and the connection was lost.
// `deepLink` must carry the id each route already reads, and the date join must be
// inclusive, because a trip's last day is still the trip.

import { describe, expect, test } from "bun:test";
import {
  answering,
  deepLink,
  kindOf,
  merge,
  namedIn,
  planOf,
  tripsToPlaces,
  transactionItem,
  tripItem,
  withinDays,
  type ConnectionGroup,
} from "../dashboard/src/lib/inspector/connections.ts";
import type { CalendarEntry, CapabilityView, FinanceTransaction, TripPlan } from "../dashboard/src/lib/api.ts";

// The shell asks only the capabilities that declare an id's kind (libs/links/ISA.md D1), and a
// row it already lists as a reference is never repeated as an inference (D2).
describe("discovery", () => {
  test("the kind is every segment but the last, and an untyped id has none", () => {
    expect(kindOf("trip:plan:18c7")).toBe("trip:plan");
    expect(kindOf("ent:18d8")).toBe("ent");
    for (const bad of ["place_0bd5", "transaction_16_1_eur", ":x", "a::b", "trip:", undefined]) {
      expect(kindOf(bad)).toBeNull();
    }
  });

  test("only declaring capabilities are asked, and a registry without links_to asks none", () => {
    const registry = [
      { name: "finance", links_to: ["trip:plan"] },
      { name: "calendar", links_to: ["trip:plan"] },
      { name: "inventory", links_to: ["ent"] },
      { name: "old-status-build" },
    ] as unknown as CapabilityView[];
    expect(answering(registry, "trip:plan:1").map((c) => c.name)).toEqual(["finance", "calendar"]);
    expect(answering(registry, "place_1")).toEqual([]);
  });

  test("a referenced row is not repeated under an inference, and references come first", () => {
    const row = (key: string) => ({ key, title: key, meta: "", item: { type: "link", id: key, kind: "cal:entry", title: key, via: "x" } as const });
    const groups: ConnectionGroup[] = [
      { capability: "calendar", basis: "coincidence", label: "Same days", icon: "calendar", items: [row("cal:entry:1"), row("cal:entry:2")] },
      { capability: "calendar", basis: "reference", label: "Calendar", icon: "calendar", items: [row("cal:entry:1")] },
    ];
    const merged = merge(groups);
    expect(merged.map((g) => g.basis)).toEqual(["reference", "coincidence"]);
    expect(merged[1].items.map((r) => r.key)).toEqual(["cal:entry:2"]);
  });
});

// A person's text and place matches are inferences, so they must not over-match: a first name
// inside another word is not a mention, and a short first name is not used at all.
describe("person inferences", () => {
  const entry = (title: string, notes: string | null = null) => ({ id: title, title, notes }) as CalendarEntry;

  test("a full name or a whole first word is a mention, a substring is not", () => {
    const entries = [entry("Dinner with Lucia García"), entry("Call lucia"), entry("Luciano visits"), entry("x", "bring Lucia's book")];
    expect(namedIn(entries, "Lucia García").map((e) => e.title)).toEqual(["Dinner with Lucia García", "Call lucia", "x"]);
  });

  test("a first name of two letters is never matched alone", () => {
    expect(namedIn([entry("Jo and Tom")], "Jo Weber")).toEqual([]);
  });

  test("a trip to a place the person lives is found either way round", () => {
    const plan = (name: string) => ({ id: name, destinations: [{ name }] }) as unknown as TripPlan;
    const found = tripsToPlaces([plan("Berlin, Germany"), plan("Rome")], ["berlin"]);
    expect(found.map((t) => t.id)).toEqual(["Berlin, Germany"]);
    expect(tripsToPlaces([plan("Rome")], [])).toEqual([]);
  });
});

describe("planOf", () => {
  test("reads payload.plan_id only on entries trips wrote", () => {
    const payload = { plan_id: "trip:plan:1" };
    expect(planOf({ source: "trips", payload })).toBe("trip:plan:1");
    expect(planOf({ source: "manual", payload })).toBeUndefined();
    expect(planOf({ source: "trips", payload: null })).toBeUndefined();
    expect(planOf({ source: "trips", payload: { plan_id: 7 } })).toBeUndefined();
  });
});

describe("deepLink", () => {
  test("a trip opens its own plan", () => {
    expect(deepLink({ type: "trip", id: "p 1", title: "", destination: "", dates: "" })).toBe("/travel?plan=p%201");
  });

  test("an event opens its day with the entry selected", () => {
    expect(deepLink({ type: "event", id: "e1", title: "", startsAt: "2026-10-06T09:00:00Z" })).toBe(
      "/calendar?date=2026-10-06&entry=e1",
    );
  });

  test("a place link opens the map on that place", () => {
    expect(deepLink({ type: "link", id: "place:5f49", kind: "place", title: "Phantasialand", via: "x" })).toBe(
      "/map?q=Phantasialand",
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
    expect(transactionItem(row)).toMatchObject({ type: "transaction", id: "t1", trip: "p1", amount: "−49.90 EUR", category: "travel · rail" });
    // With a source_id the id is the linkable one (libs/links D3), not the journal position.
    expect(transactionItem({ ...row, source_id: "abc" })).toMatchObject({ id: "fin:tx:abc" });
  });

  test("a trip without a budget states none rather than zero", () => {
    const plan = {
      id: "p1", title: "Berlin", destinations: [{ name: "Berlin" }], date_start: "2026-10-01",
      date_end: "2026-10-03", travelers: [], budget_cents: null, currency: null,
    } as unknown as TripPlan;
    expect(tripItem(plan)).toMatchObject({ dates: "2026-10-01 – 2026-10-03", budget: undefined, companions: undefined });
  });
});
