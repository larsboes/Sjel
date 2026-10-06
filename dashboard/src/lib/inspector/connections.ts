import { calendar, finance, trips, type CalendarEntry, type FinanceTransaction, type TripPlan } from '../api';
import { link } from '../nav';
import { money } from '../travel/plan-search';
import type { InspectableItem } from './inspector.svelte';

/**
 * What the inspected item touches in other capabilities, read live when the inspector
 * opens. The joins are the ones the capabilities already publish: finance tags a
 * transaction with the trips plan id (`FinanceTransaction.trip_id`), a trip has a date
 * range, and a calendar entry has a start. Nothing is inferred beyond that.
 *
 * A group whose capability did not answer carries `error` and names the capability; it is
 * shown, not dropped, so an empty list never stands in for an unanswered one.
 */

export interface Related {
  key: string;
  title: string;
  meta: string;
  item: InspectableItem;
}

export interface ConnectionGroup {
  capability: 'calendar' | 'finance' | 'trips';
  label: string;
  icon: 'calendar' | 'wallet' | 'train';
  items: Related[];
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
  }
}

/** Inclusive on both ends; all three are YYYY-MM-DD, so string order is date order. */
export function withinDays(day: string, start: string, end: string): boolean {
  return day >= start && day <= end;
}

export function tripItem(plan: TripPlan): InspectableItem {
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

export function transactionItem(row: FinanceTransaction): InspectableItem {
  const sign = row.kind === 'expense' ? '−' : row.kind === 'income' ? '+' : '';
  return {
    type: 'transaction',
    id: row.id,
    merchant: row.description,
    amount: `${sign}${money(row.amount_cents, row.currency)}`,
    date: row.date,
    category: row.category.split(':').slice(1).join(' · ') || row.category,
    trip: row.trip_id ?? undefined,
  };
}

export function eventItem(entry: CalendarEntry): InspectableItem {
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
  };
}

function asRelated(item: InspectableItem): Related {
  switch (item.type) {
    case 'trip':
      return { key: `trip:${item.id}`, title: item.title, meta: item.dates, item };
    case 'transaction':
      return { key: `tx:${item.id}`, title: item.merchant, meta: `${item.date} · ${item.amount}`, item };
    case 'event':
      return {
        key: `ev:${item.id ?? item.startsAt}`,
        title: item.title,
        meta: item.allDay ? item.startsAt.slice(0, 10) : `${item.startsAt.slice(0, 10)} ${item.startsAt.slice(11, 16)}`,
        item,
      };
    default:
      return { key: `${item.type}:${'id' in item ? item.id : ''}`, title: item.type, meta: '', item };
  }
}

async function group(
  capability: ConnectionGroup['capability'],
  label: string,
  icon: ConnectionGroup['icon'],
  read: () => Promise<InspectableItem[]>,
): Promise<ConnectionGroup> {
  try {
    return { capability, label, icon, items: (await read()).map(asRelated) };
  } catch (err) {
    return { capability, label, icon, items: [], error: `${capability} did not answer: ${err instanceof Error ? err.message : String(err)}` };
  }
}

const MAX_PER_GROUP = 8;
const cap = <T>(rows: T[]) => rows.slice(0, MAX_PER_GROUP);

/** The groups for one item, or none for a type with no published join yet. */
export async function connectionsFor(item: InspectableItem): Promise<ConnectionGroup[]> {
  switch (item.type) {
    case 'trip': {
      const plan = trips.get(item.id);
      return Promise.all([
        group('calendar', 'During this trip', 'calendar', async () => {
          const p = await plan;
          return cap(await calendar.entries.list(p.date_start, p.date_end)).map(eventItem);
        }),
        // ponytail: the dashboard scopes to one currency (EUR by default), so a trip paid in
        // another currency shows only its EUR rows. A `trip_id` filter on finance fixes both.
        group('finance', 'Spent on this trip', 'wallet', async () => {
          const { transactions } = await finance.dashboard();
          return cap(transactions.filter((t) => t.trip_id === item.id)).map(transactionItem);
        }),
      ]);
    }
    case 'transaction': {
      const groups = [
        group('calendar', 'That day', 'calendar', async () =>
          cap(await calendar.entries.list(item.date, item.date)).map(eventItem)),
      ];
      if (item.trip) {
        const tripId = item.trip;
        groups.unshift(group('trips', 'Part of trip', 'train', async () => [tripItem(await trips.get(tripId))]));
      }
      return Promise.all(groups);
    }
    case 'event': {
      const day = item.startsAt.slice(0, 10);
      return Promise.all([
        group('trips', 'Trip around it', 'train', async () =>
          (await trips.list()).filter((p) => p.id === item.tripId || withinDays(day, p.date_start, p.date_end)).map(tripItem)),
        group('finance', 'Spent that day', 'wallet', async () =>
          cap((await finance.dashboard({ start: day, end: day })).transactions).map(transactionItem)),
      ]);
    }
    default:
      return [];
  }
}
