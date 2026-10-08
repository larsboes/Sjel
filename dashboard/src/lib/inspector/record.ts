import { link } from '../nav';
import { expand, kindOf } from './connections';
import type { InspectableItem } from './inspector.svelte';

/**
 * A record opened as a page (2026-10-08): the third layer after a row and its side peek or
 * inspector, at a URL of its own (`/record?id=`). Notion's peek/page split.
 *
 * An item the shell already holds is handed over in memory, so any inspectable record opens.
 * After a reload only ids `expand` can read again (`cal:entry`, `trip:plan`) resolve; the page
 * says so for the rest rather than showing an empty record.
 */
const handed = new Map<string, InspectableItem>();

/** The record page for an item, or null when the item has no id to put in a URL. */
export function recordHref(item: InspectableItem): string | null {
  if (!('id' in item) || !item.id) return null;
  handed.set(item.id, item);
  return link(`/record?id=${encodeURIComponent(item.id)}`);
}

export async function resolveRecord(id: string): Promise<InspectableItem | null> {
  const held = handed.get(id);
  if (held) return held;
  const kind = kindOf(id);
  if (!kind) return null;
  const item = await expand({ type: 'link', id, kind, title: id, via: '' });
  return item.type === 'link' ? null : item;
}

export interface RecordView {
  title: string;
  kind: string;
  properties: { label: string; value: string }[];
}

/** The record's title and its fields as a property list. Empty fields are left out. */
export function recordView(item: InspectableItem): RecordView {
  const props = (pairs: [string, string | number | undefined | null][]) =>
    pairs.filter(([, v]) => v != null && v !== '').map(([label, v]) => ({ label, value: String(v) }));
  switch (item.type) {
    case 'event':
      return {
        title: item.title,
        kind: 'Event',
        properties: props([
          ['Starts', item.allDay ? item.startsAt.slice(0, 10) : item.startsAt.slice(0, 16).replace('T', ' ')],
          ['Ends', item.endsAt?.slice(0, 16).replace('T', ' ')],
          ['Where', item.location],
          ['Commitment', item.commitment],
          ['With', item.attendees?.join(', ')],
          ['Notes', item.notes],
        ]),
      };
    case 'person':
      return {
        title: item.name,
        kind: 'Person',
        properties: props([
          ['Relationship', item.relationship ?? item.role],
          ['Where', item.location],
          ['Status', item.status],
          ['Email', item.email],
          ['Notes', item.notes],
        ]),
      };
    case 'trip':
      return {
        title: item.title,
        kind: 'Trip',
        properties: props([
          ['Where', item.destination],
          ['Dates', item.dates],
          ['With', item.companions?.join(', ')],
          ['Budget', item.budget],
          ['Spent', item.spent],
        ]),
      };
    case 'transaction':
      return {
        title: item.merchant,
        kind: 'Transaction',
        properties: props([
          ['Amount', item.amount],
          ['Date', item.date],
          ['Category', item.category],
          ['Notes', item.notes],
        ]),
      };
    case 'layout':
      return {
        title: item.name,
        kind: 'Layout',
        properties: props([
          ['Clearances', item.pass ? 'Pass' : 'Need attention'],
          ['Items', item.itemsCount],
          ['Cost', item.totalCost],
        ]),
      };
    case 'link':
      return {
        title: item.title,
        kind: item.kind,
        properties: props([
          ['When', item.at?.slice(0, 10)],
          ['Detail', item.meta],
          ['Via', item.via],
        ]),
      };
  }
}
