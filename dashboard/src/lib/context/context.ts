import {
  calendar,
  entities,
  finance,
  trips,
  type CalendarEntry,
  type Entity,
  type FinanceTransaction,
  type PlanItem,
  type TripPlan,
} from '../api';
import { eventItem, namedIn, transactionItem, tripItem, tripsToPlaces, withinDays } from '../inspector/connections';
import type { InspectableItem } from '../inspector/inspector.svelte';
import { link } from '../nav';

/**
 * A context view (2026-10-08): a day, a trip or a person, with everything other capabilities
 * hold about it as one list. The open question in the vault note `Projects/Sjel/Connected
 * shell.md` was whether Home becomes a view per context; this is that view, one layer below
 * Home's ranked list and one above a single record (`/record`).
 *
 * Every row is a record some capability owns. It opens in the inspector, so its own
 * connections are one more step away. A source that did not answer is named in `missing`,
 * never shown as an empty list (libs/links/ISA.md D2's rule for connections).
 */

export type ContextKind = 'day' | 'trip' | 'person';
export type RowKind = 'event' | 'plan' | 'spend' | 'trip';

export interface ContextRow {
  key: string;
  /** YYYY-MM-DD, or null for an undated plan item. */
  day: string | null;
  /** HH:MM where the record has a clock time. */
  time: string | null;
  kind: RowKind;
  title: string;
  meta: string;
  /** Signed cents, spend only. */
  amount: number | null;
  currency: string | null;
  inactive: boolean;
  /** What opening the row shows in the inspector. Null: the row links to its page instead. */
  item: InspectableItem | null;
  href: string | null;
}

export interface ContextView {
  kind: ContextKind;
  title: string;
  eyebrow: string;
  properties: { label: string; value: string; href?: string }[];
  rows: ContextRow[];
  /** Capabilities that did not answer, so their rows are absent, not zero. */
  missing: string[];
}

const text = (v: unknown) => (typeof v === 'string' && v ? v : null);
const payloadOf = (item: PlanItem) =>
  (item.payload && typeof item.payload === 'object' ? item.payload : {}) as Record<string, unknown>;

export function entryRow(entry: CalendarEntry): ContextRow {
  return {
    key: `cal:${entry.id}`,
    day: entry.starts_at.slice(0, 10),
    time: entry.all_day ? null : entry.starts_at.slice(11, 16) || null,
    kind: 'event',
    title: entry.title,
    meta: entry.location ?? entry.kind,
    amount: null,
    currency: null,
    inactive: false,
    item: eventItem(entry),
    href: null,
  };
}

export function planItemRow(item: PlanItem, plan: TripPlan): ContextRow {
  const p = payloadOf(item);
  const status = text(p.status);
  return {
    key: `plan:${item.id}`,
    day: item.day,
    time: text(p.time),
    kind: 'plan',
    title: item.title,
    meta: [plan.title, status].filter(Boolean).join(' · '),
    amount: null,
    currency: null,
    inactive: status === 'dropped' || status === 'done',
    // A plan item has no inspector form of its own; its trip page shows it in place.
    item: null,
    href: link(`/travel?plan=${encodeURIComponent(plan.id)}`),
  };
}

export function transactionRow(tx: FinanceTransaction): ContextRow {
  const sign = tx.kind === 'expense' ? -1 : 1;
  return {
    key: `fin:${tx.id}`,
    day: tx.date,
    time: null,
    kind: 'spend',
    title: tx.description,
    meta: tx.category.split(':').slice(1).join(' · ') || tx.category,
    amount: sign * tx.amount_cents,
    currency: tx.currency,
    inactive: tx.kind === 'transfer',
    item: transactionItem(tx),
    href: null,
  };
}

export function tripRow(plan: TripPlan): ContextRow {
  return {
    key: `trip:${plan.id}`,
    day: plan.date_start,
    time: null,
    kind: 'trip',
    title: plan.title,
    meta: `${plan.date_start} – ${plan.date_end}`,
    amount: null,
    currency: null,
    inactive: plan.status === 'archived',
    item: tripItem(plan),
    href: null,
  };
}

/** Day, then clock time; undated days last, untimed (all-day) rows first in their day, as a
 *  calendar shows them. One record never appears twice. */
export function ordered(rows: ContextRow[]): ContextRow[] {
  const seen = new Set<string>();
  return rows
    .filter((r) => !seen.has(r.key) && seen.add(r.key))
    .sort((a, b) => {
      if (a.day !== b.day) return a.day === null ? 1 : b.day === null ? -1 : a.day < b.day ? -1 : 1;
      if (a.time !== b.time) return a.time === null ? -1 : b.time === null ? 1 : a.time < b.time ? -1 : 1;
      return 0;
    });
}

/** Money out per currency, from the spend rows. Only expenses count; a transfer is not spend. */
export function spent(rows: ContextRow[]): Map<string, number> {
  const out = new Map<string, number>();
  for (const r of rows) {
    if (r.kind === 'spend' && !r.inactive && r.amount !== null && r.amount < 0 && r.currency) {
      out.set(r.currency, (out.get(r.currency) ?? 0) - r.amount);
    }
  }
  return out;
}

export function money(cents: number, currency: string): string {
  return new Intl.NumberFormat('de-DE', { style: 'currency', currency }).format(cents / 100);
}

export const dayLabel = (day: string) =>
  new Date(`${day}T12:00:00`).toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long' });

export function shiftDay(day: string, by: number): string {
  const d = new Date(`${day}T12:00:00Z`);
  d.setUTCDate(d.getUTCDate() + by);
  return d.toISOString().slice(0, 10);
}

/** The context page for a record, where it has one. */
export function contextHref(item: InspectableItem): string | null {
  switch (item.type) {
    case 'trip':
      return link(`/context?trip=${encodeURIComponent(item.id)}`);
    case 'person':
      return link(`/context?person=${encodeURIComponent(item.id)}`);
    case 'event':
      return link(`/context?day=${item.startsAt.slice(0, 10)}`);
    case 'transaction':
      return link(`/context?day=${item.date}`);
    default:
      return null;
  }
}

/** Runs each source; a failed one is named in `missing` and contributes no rows. */
async function gather(sources: [string, () => Promise<ContextRow[]>][]) {
  const results = await Promise.allSettled(sources.map(([, read]) => read()));
  const missing: string[] = [];
  const rows: ContextRow[] = [];
  results.forEach((r, i) => {
    if (r.status === 'fulfilled') rows.push(...r.value);
    else missing.push(sources[i][0]);
  });
  return { rows: ordered(rows), missing: [...new Set(missing)] };
}

const spendIn = async (start: string, end: string) =>
  (await finance.dashboard({ start, end })).transactions.map(transactionRow);

export async function loadDay(day: string): Promise<ContextView> {
  let spanning: TripPlan[] = [];
  const { rows, missing } = await gather([
    ['calendar', async () => (await calendar.entries.list(day, day)).map(entryRow)],
    [
      'trips',
      async () => {
        spanning = (await trips.list()).filter((p) => withinDays(day, p.date_start, p.date_end));
        const details = await Promise.all(spanning.map((p) => trips.get(p.id)));
        return details.flatMap((d) => [tripRow(d), ...d.items.filter((i) => i.day === day).map((i) => planItemRow(i, d))]);
      },
    ],
    ['finance', () => spendIn(day, day)],
  ]);
  const total = spent(rows);
  return {
    kind: 'day',
    title: dayLabel(day),
    eyebrow: 'Day',
    properties: [
      ...spanning.map((p) => ({ label: 'Trip', value: p.title, href: link(`/context?trip=${encodeURIComponent(p.id)}`) })),
      ...[...total].map(([currency, cents]) => ({ label: 'Spent', value: money(cents, currency) })),
    ],
    rows,
    missing,
  };
}

export async function loadTrip(id: string): Promise<ContextView> {
  const plan = await trips.get(id);
  const { rows, missing } = await gather([
    ['trips', async () => plan.items.map((i) => planItemRow(i, plan))],
    ['calendar', async () => (await calendar.entries.list(plan.date_start, plan.date_end)).map(entryRow)],
    ['finance', () => spendIn(plan.date_start, plan.date_end)],
  ]);
  const total = spent(rows);
  const budget = plan.budget_cents != null ? money(plan.budget_cents, plan.currency ?? 'EUR') : null;
  return {
    kind: 'trip',
    title: plan.title,
    eyebrow: 'Trip',
    properties: [
      { label: 'Dates', value: `${plan.date_start} – ${plan.date_end}` },
      { label: 'Where', value: plan.destinations.map((d) => d.name).join(' → ') || plan.title },
      ...plan.travelers.map((t) => ({ label: 'With', value: t, href: link(`/context?person=${encodeURIComponent(t)}`) })),
      ...(budget ? [{ label: 'Budget', value: budget }] : []),
      ...[...total].map(([currency, cents]) => ({ label: 'Spent in these days', value: money(cents, currency) })),
      { label: 'Plan', value: 'Open the itinerary', href: link(`/travel?plan=${encodeURIComponent(plan.id)}`) },
    ],
    rows,
    missing,
  };
}

/** A person by entity id, or by bare name where the caller only had a name (a traveller). */
export async function loadPerson(idOrName: string): Promise<ContextView> {
  let person: Entity | null = null;
  try {
    person = await entities.get(idOrName);
  } catch {
    person = (await entities.list('person', idOrName).catch(() => [])).find(
      (e) => e.name.toLowerCase() === idOrName.toLowerCase(),
    ) ?? null;
  }
  const name = person?.name ?? idOrName;
  const places = [...new Set((person?.facts ?? []).map((f) => f.place).filter((p): p is string => !!p))];
  const now = Date.now();
  const iso = (ms: number) => new Date(ms).toISOString().slice(0, 10);
  const { rows, missing } = await gather([
    ['calendar', async () => namedIn(await calendar.entries.list(iso(now - 30 * 864e5), iso(now + 120 * 864e5)), name).map(entryRow)],
    [
      'trips',
      async () => {
        const plans = await trips.list();
        const lower = name.toLowerCase();
        const along = plans.filter((p) => p.travelers.some((t) => t.toLowerCase() === lower));
        return [...along, ...tripsToPlaces(plans, places)].map(tripRow);
      },
    ],
  ]);
  const relationship = person?.values?.relationship?.value;
  return {
    kind: 'person',
    title: name,
    eyebrow: 'Person',
    properties: [
      ...(typeof relationship === 'string' ? [{ label: 'Relationship', value: relationship }] : []),
      ...places.map((p) => ({ label: 'Place', value: p, href: link(`/map?q=${encodeURIComponent(p)}`) })),
      person
        ? { label: 'Profile', value: 'Open in People', href: link(`/people?id=${encodeURIComponent(person.id)}`) }
        : { label: 'Profile', value: 'No People entry has this name' },
    ],
    rows,
    missing,
  };
}
