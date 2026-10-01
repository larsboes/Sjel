// One pass of the Sparpreis price watch, then exit.
//
// The research verdict this implements (travel PRD R4, 2026-08-12): Sparpreis prices
// for a specific train DO fall, but rarely and unpredictably — later-released cheap
// contingents and DB promo windows are the two real events. Watching a booked train is
// dead weight; watching a not-yet-booked trip is one cheap cron.
//
// What it watches, per upcoming plan:
//
// - Every train stage that is not booked or completed, searched by the stage's own place
//   names on its date. Before 2026-09-25 only `option_set` items were watched, so a plan
//   whose route changed kept watching the old route and never the new one: the Berlin
//   plan watched Bonn → Berlin for six weeks after its stages became Bonn → Stuttgart →
//   Berlin.
// - Rail `option_set` items (the solver and the agent surface write those), but only
//   while they still match an unbooked train stage. A plan with no train stage at all
//   keeps the old behaviour and watches every one.
//
// Each watch keeps ONE `option_set` item, `sparpreis-watch:<key>`, whose payload carries
// the whole price history. It used to write one item per day, which put 30 near-identical
// rows in one plan; `consolidate` folds those into the single item and deletes them.
//
// A `note` item is written when today's cheapest fare is below every earlier
// observation. "Below the last check" flagged €67.99 → €47.99 as news when €39.99 had
// already been seen. Durable plan state is the alert surface.
//
// It talks to trips and transit over HTTP and never to their databases — the documented
// composition edge (CONTRIBUTING.md#schemas-and-dependency-direction). Fare context (bc,
// d_ticket, first_class) is replayed from the watched query, so a drop is a drop in the
// price the traveller would actually pay.

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { axonRoot } from "./lib/overlay.ts";
import { authorizedLoopbackRequest, loadInboundCredential } from "./lib/inbound-auth.ts";

function fail(message: string): never {
  console.error(`sparpreis-watch: ${message}`);
  process.exit(1);
}

const SJEL_ROOT = axonRoot();

export { authorizedLoopbackRequest };

function capabilityFetch(url: string, init: RequestInit = {}): Promise<Response> {
  const credential = loadInboundCredential(SJEL_ROOT).authorization;
  if (!credential) throw new Error("deployment inbound credential is not configured");
  const request = authorizedLoopbackRequest(url, init, credential);
  return fetch(request.target, request.init);
}

/**
 * The port a service.toml declares. Throws with the reason when it declares none, or
 * declares something that is not a port.
 *
 * The digits check is what keeps `http://127.0.0.1:${port}` a loopback URL. Measured:
 * `new URL("http://127.0.0.1:1@evil.example/x").host` is `evil.example`, because the
 * last `@` before the path ends the userinfo — so a manifest whose port reads
 * `1@evil.example` moves the host, and every header on that request goes with it.
 * CodeQL alerts 14 and 18-22 were dismissed with "the only file data is the port".
 * That is true, and on its own it was not enough. `tools/feed-sweep.ts` carries the
 * same check for the same reason.
 *
 * Its own exported function so tools/sparpreis-watch.test.ts can watch it refuse;
 * `portOf` below reports through `fail`, which exits the process.
 */
export function portInManifest(body: string): string {
  const line = body.split("\n").find((l) => /^port\s*=/.test(l));
  const port = line?.match(/"([^"]*)"/)?.[1] ?? "";
  if (!port) throw new Error("declares no port");
  if (!/^\d{1,5}$/.test(port) || Number(port) < 1 || Number(port) > 65535) {
    throw new Error(`declares a port that is not a TCP port: ${port}`);
  }
  return port;
}

/** A capability's port, from the one file that declares it. */
function portOf(capability: string): string {
  const manifest = join(SJEL_ROOT, "capabilities", capability, "service.toml");
  if (!existsSync(manifest)) fail(`no ${manifest}`);
  try {
    return portInManifest(readFileSync(manifest, "utf8"));
  } catch (error) {
    fail(`${manifest} ${(error as Error).message}`);
  }
}

export interface RailWatch {
  planId: string;
  from: string;
  to: string;
  time: string;
  bc?: number;
  dTicket?: boolean;
  firstClass?: boolean;
  /** Set when the watch comes from a stage rather than from an option_set. */
  stageId?: string;
}

/** A stable identity for one watched search, so observations land on one item. */
export function watchKey(watch: RailWatch): string {
  const fare = [
    watch.bc ? `bc${watch.bc}` : "",
    watch.dTicket ? "dt" : "",
    watch.firstClass ? "k1" : "",
  ]
    .filter(Boolean)
    .join("-");
  return `${watch.from}:${watch.to}:${watch.time}${fare ? `:${fare}` : ""}`;
}

export interface Stage {
  id: string;
  date?: string | null;
  status?: string;
  transport_modes?: string[];
  origin?: { name?: string };
  destination?: { name?: string };
}

/** The stage statuses still worth re-pricing. `booked` and `completed` are not. */
const WATCHED_STATUSES = new Set(["open", "planning", "option_selected"]);

/** A stage carries a date and no time, so its search starts here. An estimate of when a
 *  traveller leaves, not a measurement; transit returns the next few journeys from it. */
export const STAGE_DEPARTURE = "07:00:00";

export function trainStages(stages: Stage[]): Stage[] {
  return stages.filter((s) => s.transport_modes?.includes("train"));
}

/** One watch per unbooked, dated, upcoming train stage, searched by place name.
 *  Transit resolves a name to a station and answers 400 rather than guess. */
export function stageWatchesOf(planId: string, stages: Stage[], today: string): RailWatch[] {
  const watches: RailWatch[] = [];
  for (const stage of trainStages(stages)) {
    if (!WATCHED_STATUSES.has(stage.status ?? "")) continue;
    if (!stage.date || stage.date < today) continue;
    const from = stage.origin?.name?.trim();
    const to = stage.destination?.name?.trim();
    if (!from || !to) continue;
    watches.push({ planId, from, to, time: `${stage.date}T${STAGE_DEPARTURE}`, stageId: stage.id });
  }
  return watches;
}

/** The rail searches a plan already records: option_set items whose query names two
 *  numeric station ids and a departure time. Accommodation option_sets (coordinate
 *  queries) and anything else fall through the numeric test. */
export function railWatchesOf(planId: string, items: unknown[]): RailWatch[] {
  const watches: RailWatch[] = [];
  for (const raw of items) {
    const item = raw as {
      item_type?: string;
      external_id?: string;
      payload?: {
        query?: {
          from?: unknown;
          to?: unknown;
          time?: unknown;
          bc?: unknown;
          d_ticket?: unknown;
          first_class?: unknown;
        };
      };
    };
    if (item.item_type !== "option_set") continue;
    // This job's own observations are option_sets too; re-watching them would
    // multiply the watch list every run.
    if (item.external_id?.startsWith("sparpreis-watch:")) continue;
    const q = item.payload?.query;
    if (typeof q?.from !== "string" || !/^\d+$/.test(q.from)) continue;
    if (typeof q?.to !== "string" || !/^\d+$/.test(q.to)) continue;
    if (typeof q?.time !== "string" || !q.time.includes("T")) continue;
    watches.push({
      planId,
      from: q.from,
      to: q.to,
      time: q.time,
      bc: typeof q.bc === "number" ? q.bc : undefined,
      dTicket: q.d_ticket === true,
      firstClass: q.first_class === true,
    });
  }
  return watches;
}

/** The identity a station pair gets on one day, for matching option_sets to stages. */
export function legKey(fromEva: string, toEva: string, time: string): string {
  return `${fromEva}:${toEva}:${time.slice(0, 10)}`;
}

/** Whether an option_set watch still describes a leg of the plan.
 *
 *  `stageLegs` holds the station pairs the unbooked stages resolved to in this run.
 *  A plan with no train stage keeps every option_set watch, because there is nothing
 *  to compare it with. A plan whose train stages are all booked watches nothing. */
export function stillPlanned(
  watch: RailWatch,
  hasTrainStages: boolean,
  stageLegs: Set<string>,
): boolean {
  if (!hasTrainStages) return true;
  return stageLegs.has(legKey(watch.from, watch.to, watch.time));
}

export interface Observation {
  day: string;
  prices: number[];
}

const LEGACY_ID = /^sparpreis-watch:(.+):(\d{4}-\d{2}-\d{2})$/;

interface RawItem {
  id?: string;
  item_type?: string;
  external_id?: string;
  payload?: {
    query?: Record<string, unknown>;
    options?: Array<{ total_price?: unknown }>;
    history?: Array<{ day?: unknown; prices?: unknown }>;
  };
}

function pricesOf(options: Array<{ total_price?: unknown }> | undefined): number[] {
  return (options ?? [])
    .map((o) => o.total_price)
    .filter((p): p is number => typeof p === "number");
}

/** The per-day observation items written before 2026-09-25, grouped by watch key. */
export function legacyObservations(items: unknown[]): Map<string, Array<RawItem & Observation>> {
  const groups = new Map<string, Array<RawItem & Observation>>();
  for (const raw of items) {
    const item = raw as RawItem;
    if (item.item_type !== "option_set") continue;
    const match = item.external_id?.match(LEGACY_ID);
    if (!match) continue;
    const [, key, day] = match;
    const list = groups.get(key) ?? [];
    list.push({ ...item, day, prices: pricesOf(item.payload?.options) });
    groups.set(key, list);
  }
  return groups;
}

/** Every observation recorded for a watch: the single item's history plus any legacy
 *  per-day items not folded in yet. One entry per day, the later write winning. */
export function historyOf(items: unknown[], key: string): Observation[] {
  const byDay = new Map<string, number[]>();
  for (const obs of legacyObservations(items).get(key) ?? []) byDay.set(obs.day, obs.prices);
  const single = (items as RawItem[]).find(
    (i) => i.item_type === "option_set" && i.external_id === `sparpreis-watch:${key}`,
  );
  for (const entry of single?.payload?.history ?? []) {
    if (typeof entry.day !== "string" || !Array.isArray(entry.prices)) continue;
    byDay.set(
      entry.day,
      entry.prices.filter((p): p is number => typeof p === "number"),
    );
  }
  return [...byDay.entries()]
    .map(([day, prices]) => ({ day, prices }))
    .sort((a, b) => a.day.localeCompare(b.day));
}

/** The lowest fare ever observed for a watch, or null before its first observation. */
export function lowestSeen(history: Observation[]): number | null {
  const all = history.flatMap((o) => o.prices);
  return all.length ? Math.min(...all) : null;
}

/** A new low is a real one, not float noise and not a first observation. */
export function dropped(previousLow: number | null, current: number): boolean {
  return previousLow !== null && current < previousLow - 0.01;
}

/** today's observation replaces an earlier one from the same day. */
export function withObservation(history: Observation[], today: Observation): Observation[] {
  return [...history.filter((o) => o.day !== today.day), today].sort((a, b) =>
    a.day.localeCompare(b.day),
  );
}

async function postItem(trips: string, planId: string, body: unknown): Promise<boolean> {
  const response = await capabilityFetch(`${trips}/api/plans/${encodeURIComponent(planId)}/items`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!response.ok) console.error(`sparpreis-watch: item write HTTP ${response.status}`);
  return response.ok;
}

async function loadItems(trips: string, planId: string): Promise<{ items: unknown[]; stages: Stage[] }> {
  const details = (await (await capabilityFetch(`${trips}/api/plans/${encodeURIComponent(planId)}`)).json()) as {
    items?: unknown[];
    stages?: Stage[];
  };
  return { items: details.items ?? [], stages: details.stages ?? [] };
}

/** Folds the per-day observation items into one item per watch, then deletes them.
 *  The single item is written first, so a failed delete leaves a duplicate rather than
 *  a gap: `historyOf` reads both and counts each day once. */
async function consolidate(trips: string, planId: string, items: unknown[]): Promise<number> {
  let removed = 0;
  for (const [key, legacy] of legacyObservations(items)) {
    const latest = [...legacy].sort((a, b) => a.day.localeCompare(b.day)).at(-1)!;
    const history = historyOf(items, key);
    const wrote = await postItem(trips, planId, {
      item_type: "option_set",
      external_id: `sparpreis-watch:${key}`,
      title: `Sparpreis watch ${key.split(":").slice(0, 2).join(" → ")}`,
      payload: {
        query: latest.payload?.query ?? {},
        options: latest.payload?.options ?? [],
        observed_at: `${latest.day}T00:00:00Z`,
        history,
        lowest: lowestSeen(history),
      },
    });
    if (!wrote) continue;
    for (const item of legacy) {
      if (!item.id) continue;
      const response = await capabilityFetch(
        `${trips}/api/plans/${encodeURIComponent(planId)}/items/${encodeURIComponent(item.id)}`,
        { method: "DELETE" },
      );
      if (response.ok) removed += 1;
      else console.error(`sparpreis-watch: delete ${item.external_id} HTTP ${response.status}`);
    }
  }
  return removed;
}

interface Journey {
  total_price?: number | null;
  start_station?: { id?: string; name?: string };
  end_station?: { id?: string; name?: string };
}

async function main(): Promise<void> {
  const trips = `http://127.0.0.1:${portOf("trips")}`;
  const transit = `http://127.0.0.1:${portOf("transit")}`;
  const today = new Date().toISOString().slice(0, 10);

  const plans = (await (await capabilityFetch(`${trips}/api/plans`)).json()) as Array<{
    id: string;
    date_start: string;
    title: string;
  }>;
  const upcoming = plans.filter((p) => p.date_start >= today);
  console.log(`sparpreis-watch: ${upcoming.length}/${plans.length} plans upcoming`);

  let watched = 0;
  let dropCount = 0;
  const CAP = 10;

  // One search, recorded on the watch's single item. Returns the journeys, or null
  // when the search failed or priced nothing.
  async function observe(planId: string, planTitle: string, items: unknown[], watch: RailWatch) {
    watched += 1;
    // The endpoint under this is bahn.de via transit; transit paces itself,
    // and this pause keeps a multi-watch run from bursting anyway.
    await new Promise((resolve) => setTimeout(resolve, 1000));
    const params = new URLSearchParams({ from: watch.from, to: watch.to, time: watch.time });
    if (watch.bc) params.set("bc", String(watch.bc));
    if (watch.dTicket) params.set("d_ticket", "true");
    if (watch.firstClass) params.set("first_class", "true");
    const search = await capabilityFetch(`${transit}/api/search?${params}`);
    if (!search.ok) {
      console.error(
        `sparpreis-watch: search ${watch.from}->${watch.to} HTTP ${search.status}: ${(await search.text()).slice(0, 200)}`,
      );
      return null;
    }
    const journeys = (await search.json()) as Journey[];
    const prices = journeys
      .map((j) => j.total_price)
      .filter((p): p is number => typeof p === "number");
    if (!prices.length) {
      console.log(`sparpreis-watch: ${watch.from}->${watch.to} returned no priced journey`);
      return journeys;
    }
    const cheapest = Math.min(...prices);
    const key = watchKey(watch);
    const earlier = historyOf(items, key);
    const low = lowestSeen(earlier);
    const history = withObservation(earlier, { day: today, prices });
    const fromName = journeys[0]?.start_station?.name ?? watch.from;
    const toName = journeys[0]?.end_station?.name ?? watch.to;

    const wrote = await postItem(trips, planId, {
      item_type: "option_set",
      external_id: `sparpreis-watch:${key}`,
      title: `Sparpreis watch ${fromName} → ${toName}, ${watch.time.slice(0, 10)}`,
      payload: {
        query: {
          from: watch.from,
          to: watch.to,
          time: watch.time,
          bc: watch.bc ?? null,
          stage_id: watch.stageId ?? null,
        },
        observed_at: new Date().toISOString(),
        options: journeys.slice(0, 5).map((j) => ({ total_price: j.total_price ?? null })),
        history,
        lowest: lowestSeen(history),
      },
    });
    if (!wrote) return journeys;
    if (dropped(low, cheapest)) {
      dropCount += 1;
      await postItem(trips, planId, {
        item_type: "note",
        external_id: `sparpreis-low:${key}:${today}`,
        title: `Sparpreis new low ${fromName} → ${toName}, ${watch.time.slice(0, 10)}: €${cheapest} (lowest before: €${low})`,
        payload: { previous_low: low, current: cheapest, watched_time: watch.time, stage_id: watch.stageId ?? null },
      });
      console.log(`sparpreis-watch: NEW LOW ${fromName}->${toName} €${low} -> €${cheapest} (${planTitle})`);
    } else {
      console.log(
        `sparpreis-watch: ${fromName}->${toName} cheapest €${cheapest}` +
          (low !== null ? ` (lowest seen €${low})` : " (first observation)"),
      );
    }
    return journeys;
  }

  for (const plan of upcoming) {
    let { items, stages } = await loadItems(trips, plan.id);
    const folded = await consolidate(trips, plan.id, items);
    if (folded) {
      console.log(`sparpreis-watch: folded ${folded} per-day observations into single items (${plan.title})`);
      ({ items, stages } = await loadItems(trips, plan.id));
    }

    // Stages first: their searches resolve the station pairs the option_sets are
    // matched against.
    const stageLegs = new Set<string>();
    for (const watch of stageWatchesOf(plan.id, stages, today)) {
      if (watched >= CAP) break;
      const journeys = await observe(plan.id, plan.title, items, watch);
      for (const j of journeys ?? []) {
        if (j.start_station?.id && j.end_station?.id) {
          stageLegs.add(legKey(j.start_station.id, j.end_station.id, watch.time));
        }
      }
    }

    const hasTrainStages = trainStages(stages).length > 0;
    for (const watch of railWatchesOf(plan.id, items)) {
      if (!stillPlanned(watch, hasTrainStages, stageLegs)) {
        console.log(
          `sparpreis-watch: ${watch.from}->${watch.to} ${watch.time} matches no unbooked stage, not watched (${plan.title})`,
        );
        continue;
      }
      if (watched >= CAP) break;
      await observe(plan.id, plan.title, items, watch);
    }
    if (watched >= CAP) {
      console.log(`sparpreis-watch: cap of ${CAP} reached, remaining watches skipped this run`);
      break;
    }
  }
  console.log(`sparpreis-watch: ${watched} watched, ${dropCount} new lows`);
}

if (import.meta.main) {
  await main();
}
