import { describe, expect, test } from 'bun:test';
import { meetupsOf } from '../src/lib/travel/who-is-around';
import type { PlanItem } from '../src/lib/api';

const BONN: [number, number] = [50.7374, 7.0982];

describe('meetupsOf', () => {
  const item = (id: string, payload: unknown, day: string | null = null) =>
    ({ id, plan_id: 'p', item_type: 'activity', day, external_id: id, title: id, payload, created_at: '0' }) as PlanItem;

  test('only activities that name people are meetups, dated ones first', () => {
    const meetups = meetupsOf([
      item('museum', { status: 'proposed' }),
      item('later', { with: [{ person: 'B' }] }),
      item('boulder', { with: [{ person: 'A' }], status: 'asked', place: 'Bonn' }, '2026-10-14'),
    ]);
    expect(meetups.map((m) => [m.itemId, m.status, m.people])).toEqual([
      ['boulder', 'asked', ['A']],
      ['later', 'idea', ['B']],
    ]);
  });
});

describe('aroundFromLocated', () => {
  test('people near the leg from entities, with hosts by sleeping option', async () => {
    const { aroundFromLocated } = await import('../src/lib/travel/who-is-around');
    const located = [
      { entity_id: 'ent:1', name: 'Ron', predicate: 'home_base', place: 'Bonn', latitude: 50.735, longitude: 7.1, sleeping_option: 'ask', sleeping_note: 'sofa' },
      { entity_id: '2', name: 'Far', predicate: 'away', place: 'Lisbon', latitude: 38.72, longitude: -9.14, sleeping_option: 'yes', sleeping_note: null },
      { entity_id: 'ent:3', name: 'Anna', predicate: 'home_base', place: 'Köln', latitude: 50.9375, longitude: 6.9603, sleeping_option: 'none', sleeping_note: null },
      { entity_id: '4', name: 'Unplaced', predicate: 'home_base', place: 'X', latitude: null, longitude: null, sleeping_option: 'yes', sleeping_note: null },
    ] as const;
    const { around, hosts } = aroundFromLocated(BONN, located as never);
    expect(around.map((p) => p.person)).toEqual(['Ron', 'Anna']);
    expect(hosts.map((h) => [h.entityId, h.person, h.note])).toEqual([['ent:1', 'Ron', 'sofa']]);
    expect(around.map((p) => p.entityId)).toEqual(['ent:1', 'ent:3']);
    expect(aroundFromLocated(null, located as never)).toEqual({ around: [], hosts: [] });
  });
});
