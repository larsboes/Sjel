import {
  calendar,
  finance,
  links,
  trips,
  type CalendarEntry,
  type CapabilityView,
  type FinanceTransaction,
  type Link,
  type TripPlan,
} from '../api';
import { link } from '../nav';
import { money } from '../travel/plan-search';
import type { InspectableEvent, InspectableItem, InspectableTransaction, InspectableTrip } from './inspector.svelte';

/**
 * What the inspected item touches in other capabilities, read live when the inspector opens.
 *
 * Two kinds of connection, kept apart on purpose (libs/links/ISA.md D2):
 *
 * - **reference**: a row holds this item's id, or the item holds the row's. Back-references come
 *   from `GET /<capability>/api/links?to=<id>`, asked only of the capabilities whose `links_to`
 *   declares the id's kind, so no pair of capabilities is named here (D1).
 * - **coincidence**: a row shares the item's days. An inference, labelled as one, and never a
 *   row the references already list.
 *
 * A group whose capability did not answer carries `error` and names the capability; it is
 * shown, not dropped, so an empty list never stands in for an unanswered one.
 */

export type Basis = 'reference' | 'coincidence';

export interface Related {
  key: string;
  title: string;
  meta: string;
  item: InspectableItem;
}

export interface ConnectionGroup {
  capability: string;
  basis: Basis;
  label: string;
  icon: 'calendar' | 'wallet' | 'train' | 'boxes';
  items: Related[];
  /** Rows that reference the item but have no linkable id of their own. */
  unlinkable?: number;
  error?: string;
}

/** Where the item lives on its own page. Each route reads these params already. */
export function deepLink(item: InspectableItem): string | null {
  switch (item.type) {
    case 'event':
      return link(`/calendar?date=${item.startsAt.slice(0, 10)}${item.id ? `&entry=${encodeURIComponent(item.id)}` : ''}`);
    case 'trip':
      return link(`/travel?plan=${encodeURIComponent(item.id)}`);
    case 'transaction':
      // ponytail: finance has no per-transaction route; the tab is the closest it gets.
      return link('/finance?view=transactions');
    case 'person':
      return link(`/people?id=${encodeURIComponent(item.id)}`);
    case 'layout':
      return link('/interior');
    case 'link':
      return item.kind === 'fin:tx' ? link('/finance?view=transactions') : null;
  }
}

/** The kind of a typed id: every segment but the last. `trip:plan:18c7` → `trip:plan`.
 *  Null for an untyped id, which nothing can reference (libs/links, `TypedId::parse`). */
export function kindOf(id: string | undefined): string | null {
  if (!id) return null;
  const cut = id.lastIndexOf(':');
  if (cut <= 0 || cut === id.length - 1) return null;
  const kind = id.slice(0, cut);
  return kind.split(':').every(Boolean) ? kind : null;
}

/** The capabilities to ask about `id`: those whose `links_to` declares its kind. */
export function answering(registry: CapabilityView[], id: string | undefined): CapabilityView[] {
  const kind = kindOf(id);
  return kind ? registry.filter((c) => c.links_to?.includes(kind)) : [];
}

/** Inclusive on both ends; all three are YYYY-MM-DD, so string order is date order. */
export function withinDays(day: string, start: string, end: string): boolean {
  return day >= start && day <= end;
}

export function tripItem(plan: TripPlan): InspectableTrip {
  return {
    type: 'trip',
    id: plan.id,
    title: plan.title,
    // All stops, in order: the first one is often a waypoint, not where the trip goes.
    destination: plan.destinations.map((d) => d.name).join(' → ') || plan.title,
    dates: `${plan.date_start} – ${plan.date_end}`,
    companions: plan.travelers.length ? plan.travelers : undefined,
    budget: plan.budget_cents != null ? money(plan.budget_cents, plan.currency ?? 'EUR') : undefined,
  };
}

export function transactionItem(row: FinanceTransaction): InspectableTransaction {
  const sign = row.kind === 'expense' ? '−' : row.kind === 'income' ? '+' : '';
  return {
    type: 'transaction',
    // The linkable id where finance has one (libs/links D3); the row key otherwise.
    id: row.source_id ? `fin:tx:${row.source_id}` : row.id,
    merchant: row.description,
    amount: `${sign}${money(row.amount_cents, row.currency)}`,
    date: row.date,
    category: row.category.split(':').slice(1).join(' · ') || row.category,
    trip: row.trip_id ?? undefined,
  };
}

/** The plan an entry belongs to, read only where trips wrote the entry: every other provider's
 *  payload is inert evidence (calendar `model.rs`, `Entry::payload`; libs/links LNK-8). */
export function planOf(entry: Pick<CalendarEntry, 'source' | 'payload'>): string | undefined {
  if (entry.source !== 'trips' || typeof entry.payload !== 'object' || entry.payload === null) return undefined;
  const plan = (entry.payload as { plan_id?: unknown }).plan_id;
  return typeof plan === 'string' ? plan : undefined;
}

export function eventItem(entry: CalendarEntry): InspectableEvent {
  return {
    type: 'event',
    id: entry.id,
    title: entry.title,
    startsAt: entry.starts_at,
    endsAt: entry.ends_at,
    allDay: entry.all_day,
    location: entry.location ?? undefined,
    commitment: entry.commitment,
    notes: entry.notes ?? undefined,
    tripId: planOf(entry),
  };
}

/** A reference row as an inspectable item. Every kind renders the same (libs/links D5). */
export function linkItem(row: Link): InspectableItem {
  return { type: 'link', id: row.id, kind: row.kind, title: row.title, at: row.at, meta: row.meta, via: row.via };
}

function idOf(item: InspectableItem): string | undefined {
  return 'id' in item ? item.id : undefined;
}

function asRelated(item: InspectableItem): Related {
  const key = idOf(item) ?? `${item.type}:${JSON.stringify(item)}`;
  switch (item.type) {
    case 'trip':
      return { key, title: item.title, meta: item.dates, item };
    case 'transaction':
      return { key, title: item.merchant, meta: `${item.date} · ${item.amount}`, item };
    case 'event':
      return {
        key,
        title: item.title,
        meta: item.allDay ? item.startsAt.slice(0, 10) : `${item.startsAt.slice(0, 10)} ${item.startsAt.slice(11, 16)}`,
        item,
      };
    case 'link':
      return { key, title: item.title, meta: [item.at?.slice(0, 10), item.meta].filter(Boolean).join(' · '), item };
    default:
      return { key, title: item.type, meta: '', item };
  }
}

const ICON: Record<string, ConnectionGroup['icon']> = { calendar: 'calendar', finance: 'wallet', trips: 'train' };

function failure(capability: string, err: unknown): string {
  return `${capability} did not answer: ${err instanceof Error ? err.message : String(err)}`;
}

async function group(
  capability: string,
  basis: Basis,
  label: string,
  read: () => Promise<{ items: InspectableItem[]; unlinkable?: number }>,
): Promise<ConnectionGroup> {
  const base = { capability, basis, label, icon: ICON[capability] ?? ('boxes' as const) };
  try {
    const { items, unlinkable } = await read();
    return { ...base, items: items.map(asRelated), unlinkable };
  } catch (err) {
    return { ...base, items: [], error: failure(capability, err) };
  }
}

const MAX_PER_GROUP = 8;
const cap = <T>(rows: T[]) => rows.slice(0, MAX_PER_GROUP);
const only = (items: InspectableItem[]) => ({ items });

/** What the item itself points at, and what shares its days. */
function readAround(item: InspectableItem): Promise<ConnectionGroup>[] {
  switch (item.type) {
    case 'trip':
      return [
        group('calendar', 'coincidence', 'Same days', async () => {
          const p = await trips.get(item.id);
          return only(cap(await calendar.entries.list(p.date_start, p.date_end)).map(eventItem));
        }),
      ];
    case 'transaction': {
      const groups = [
        group('calendar', 'coincidence', 'Same day', async () =>
          only(cap(await calendar.entries.list(item.date, item.date)).map(eventItem))),
      ];
      const tripId = item.trip;
      if (tripId) groups.unshift(group('trips', 'reference', 'Its trip', async () => only([tripItem(await trips.get(tripId))])));
      return groups;
    }
    case 'event': {
      const day = item.startsAt.slice(0, 10);
      const tripId = item.tripId;
      return [
        // The entry's own plan id is a reference it holds; `merge` keeps it out of the inference.
        ...(tripId ? [group('trips', 'reference', 'Its trip', async () => only([tripItem(await trips.get(tripId))]))] : []),
        group('trips', 'coincidence', 'Same days', async () =>
          only((await trips.list()).filter((p) => withinDays(day, p.date_start, p.date_end)).map(tripItem))),
        group('finance', 'coincidence', 'Same day', async () =>
          only(cap((await finance.dashboard({ start: day, end: day })).transactions).map(transactionItem))),
      ];
    }
    default:
      return [];
  }
}

/** References first, then coincidences without the rows the references already list. */
export function merge(groups: ConnectionGroup[]): ConnectionGroup[] {
  const references = groups.filter((g) => g.basis === 'reference');
  const listed = new Set(references.flatMap((g) => g.items.map((r) => r.key)));
  return [
    ...references,
    ...groups
      .filter((g) => g.basis === 'coincidence')
      .map((g) => ({ ...g, items: g.items.filter((r) => !listed.has(r.key)) })),
  ];
}

/** The groups for one item, given the registry the shell already polls. */
export async function connectionsFor(item: InspectableItem, registry: CapabilityView[]): Promise<ConnectionGroup[]> {
  const id = idOf(item);
  const back = id
    ? answering(registry, id).map((c) =>
        group(c.name, 'reference', c.name[0].toUpperCase() + c.name.slice(1), async () => {
          const answer = await links.find(c.name, id);
          return { items: answer.links.map(linkItem), unlinkable: answer.unlinkable };
        }))
    : [];
  return merge(await Promise.all([...back, ...readAround(item)]));
}
