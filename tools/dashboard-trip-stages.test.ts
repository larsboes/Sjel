// dashboard/src/lib/travel/stages.ts: which stage owns which item, and in what order.
import { describe, expect, test } from "bun:test";
import type { PlanItem, TripStage } from "../dashboard/src/lib/api";
import { bandsFor, itemTime } from "../dashboard/src/lib/travel/stages";

const place = (id: string) => ({ id, name: id, kind: "city" as const, address: null, latitude: null, longitude: null });
const stage = (id: string, sequence: number, date: string): TripStage => ({
  id, sequence, origin: place("a"), destination: place(id), date,
  transport_modes: ["train"], travelers: [], status: "planning", selected_option_id: null,
});
const item = (title: string, day: string | null, payload: Record<string, unknown> = {}): PlanItem => ({
  id: title, plan_id: "p", item_type: "activity", day, external_id: title, title, payload, created_at: "",
});

describe("bandsFor", () => {
  const plan = {
    date_start: "2026-10-08",
    date_end: "2026-10-13",
    stages: [stage("stuttgart", 0, "2026-10-08"), stage("nuernberg", 1, "2026-10-09"), stage("berlin", 2, "2026-10-09"), stage("home", 3, "2026-10-13")],
  };

  test("a shared day goes to the later stage unless the item names its stage", () => {
    const { bands } = bandsFor(plan, [item("gallery", "2026-10-09"), item("castle", "2026-10-09", { stage_id: "nuernberg" })]);
    expect(bands[1].items.map((i) => i.title)).toEqual(["castle"]);
    expect(bands[2].items.map((i) => i.title)).toEqual(["gallery"]);
    expect(bands[2].days).toEqual(["2026-10-09", "2026-10-10", "2026-10-11", "2026-10-12"]);
    expect(bands[3].days).toEqual(["2026-10-13"]);
  });

  test("items before the first stage and without a day are not hidden", () => {
    const r = bandsFor(plan, [item("old", "2026-10-07"), item("someday", null)]);
    expect(r.outside.map((i) => i.title)).toEqual(["old"]);
    expect(r.undated.map((i) => i.title)).toEqual(["someday"]);
  });

  test("a clock time beats a block, and a block sorts at its start", () => {
    const { bands } = bandsFor(plan, [
      item("dinner", "2026-10-08", { block: "evening" }),
      item("pitch", "2026-10-08", { time: "17:00", block: "afternoon" }),
      item("arrive", "2026-10-08", { block: "morning" }),
      item("train", "2026-10-08", { departure: "2026-10-08T07:14" }),
    ]);
    expect(bands[0].items.map((i) => i.title)).toEqual(["train", "arrive", "pitch", "dinner"]);
    expect(itemTime(bands[0].items[0])).toBe("07:14");
  });
});
