// A trip read as its stages (2026-10-07): which items belong to which leg, in what order, and
// what each one's state means. The page renders this and decides nothing. Kept free of Svelte
// so `bun test` drives it (tools/dashboard-trip-stages.test.ts).
import type { PlanItem, TripPlan, TripStage } from "../api";
import type { Tone } from "../ui/Chip.svelte";

export interface StageBand {
  stage: TripStage;
  /** Days this stage owns, ascending. At least the stage's own date. */
  days: string[];
  items: PlanItem[];
}

export interface TripBands {
  bands: StageBand[];
  /** Dated before the first stage: usually a leftover from an older version of the plan. */
  outside: PlanItem[];
  /** No day at all. */
  undated: PlanItem[];
}

const BLOCK_ORDER: Record<string, number> = { morning: 0, day: 1, afternoon: 2, evening: 3 };

type Payload = Record<string, unknown>;
const payloadOf = (item: PlanItem): Payload =>
  item.payload && typeof item.payload === "object" ? (item.payload as Payload) : {};
const str = (v: unknown): string | null => (typeof v === "string" && v ? v : null);

function addDays(day: string, n: number): string {
  const d = new Date(`${day}T12:00:00Z`);
  d.setUTCDate(d.getUTCDate() + n);
  return d.toISOString().slice(0, 10);
}

/** "HH:MM" when the item names one: its own `time`, a booking's departure, a stay's check-in. */
export function itemTime(item: PlanItem): string | null {
  const p = payloadOf(item);
  const own = str(p.time);
  if (own && /^\d{2}:\d{2}/.test(own)) return own.slice(0, 5);
  const departure = str(p.departure);
  if (departure && departure.length >= 16) return departure.slice(11, 16);
  return null;
}

export function itemBlock(item: PlanItem): string | null {
  return str(payloadOf(item).block);
}

/** Booked things are settled; everything else an agent wrote is a proposal until it is not. */
export function itemStatus(item: PlanItem): { label: string; tone: Tone } {
  const p = payloadOf(item);
  if (item.item_type === "booking" || p.booked === true) return { label: "Booked", tone: "success" };
  switch (str(p.status)) {
    case "booked":
      return { label: "Booked", tone: "success" };
    case "planned":
      return { label: "Planned", tone: "accent" };
    case "done":
      return { label: "Done", tone: "muted" };
    case "dropped":
      return { label: "Dropped", tone: "muted" };
    case "proposed":
      return { label: "Proposed", tone: "neutral" };
    default:
      return item.item_type === "note" ? { label: "Note", tone: "muted" } : { label: "Open", tone: "warning" };
  }
}

export function itemInactive(item: PlanItem): boolean {
  const s = str(payloadOf(item).status);
  return s === "done" || s === "dropped";
}

/** Whole euros or cents, in the item's currency; null when it names no amount. */
export function itemCost(item: PlanItem): string | null {
  const p = payloadOf(item);
  if (typeof p.amount_cents !== "number") return null;
  const currency = str(p.currency) ?? "EUR";
  return new Intl.NumberFormat("de-DE", { style: "currency", currency }).format(p.amount_cents / 100);
}

export function stageTone(status: TripStage["status"]): Tone {
  return { booked: "success", completed: "success", option_selected: "accent", planning: "warning", open: "muted" }[
    status
  ] as Tone;
}

function byTimeOfDay(a: PlanItem, b: PlanItem): number {
  const rank = (item: PlanItem) => {
    const time = itemTime(item);
    if (time) return Number(time.slice(0, 2)) * 60 + Number(time.slice(3, 5));
    // A block without a clock time sorts at its block's start: morning 08:00 … evening 18:00.
    return [480, 600, 780, 1080][BLOCK_ORDER[itemBlock(item) ?? ""] ?? 1];
  };
  return (a.day ?? "").localeCompare(b.day ?? "") || rank(a) - rank(b);
}

/**
 * A stage owns the days from its date up to the next stage's date. Two stages on one day
 * (Nürnberg and Berlin both on Fri) both claim that day; an item dated there goes to the later
 * one unless its payload names a `stage_id`, because the later stage is where the night is.
 */
export function bandsFor(plan: Pick<TripPlan, "date_start" | "date_end" | "stages">, items: PlanItem[]): TripBands {
  const stages = [...plan.stages].sort((a, b) => a.sequence - b.sequence);
  const bands: StageBand[] = stages.map((stage, index) => {
    const start = stage.date ?? plan.date_start;
    const next = stages[index + 1]?.date ?? addDays(plan.date_end, 1);
    const days = [start];
    for (let d = addDays(start, 1); d < next; d = addDays(d, 1)) days.push(d);
    return { stage, days, items: [] };
  });
  const outside: PlanItem[] = [];
  const undated: PlanItem[] = [];
  for (const item of items) {
    const named = str(payloadOf(item).stage_id);
    const byId = named ? bands.find((b) => b.stage.id === named) : undefined;
    if (byId) {
      byId.items.push(item);
      continue;
    }
    if (!item.day) {
      undated.push(item);
      continue;
    }
    const owner = [...bands].reverse().find((b) => b.days[0] <= item.day!);
    if (owner) owner.items.push(item);
    else outside.push(item);
  }
  for (const band of bands) band.items.sort(byTimeOfDay);
  outside.sort(byTimeOfDay);
  return { bands, outside, undated };
}
