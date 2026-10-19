// Who you know near each leg of a plan, and the meetups the plan holds.
//
// The people come from capabilities/entities (GET /entities/api/located, PRD Q117).
// Names never leave this machine: the join runs in the dashboard, and nothing here is
// sent to a model (PRD §6.1, C2).

import type { LocatedPerson, PlanItem, TripStage } from '../api';

/** places' own disclosure radius (capabilities/places/README.md, D4). */
export const AROUND_RADIUS_KM = 50;

export interface AroundPerson {
  entityId: string;
  person: string;
  placeName: string;
  distanceKm: number;
}

export interface Meetup {
  itemId: string;
  title: string;
  people: string[];
  status: string;
  day: string | null;
  place: string | null;
}

export function haversineKm(a: [number, number], b: [number, number]): number {
  const rad = (d: number) => (d * Math.PI) / 180;
  const dLat = rad(b[0] - a[0]);
  const dLon = rad(b[1] - a[1]);
  const h = Math.sin(dLat / 2) ** 2 + Math.cos(rad(a[0])) * Math.cos(rad(b[0])) * Math.sin(dLon / 2) ** 2;
  return 2 * 6371 * Math.asin(Math.sqrt(h));
}



/** The meetups on a plan: `activity` items whose payload names people in `with`. */
export function meetupsOf(items: PlanItem[]): Meetup[] {
  const meetups: Meetup[] = [];
  for (const item of items) {
    if (item.item_type !== 'activity') continue;
    const payload = (item.payload ?? {}) as { with?: Array<{ person?: string }>; status?: string; place?: string };
    const people = (payload.with ?? []).map((entry) => entry.person?.trim() ?? '').filter(Boolean);
    if (!people.length) continue;
    meetups.push({
      itemId: item.id,
      title: item.title,
      people,
      status: payload.status ?? 'idea',
      day: item.day,
      place: payload.place ?? null,
    });
  }
  return meetups.sort((a, b) => (a.day ?? '9999').localeCompare(b.day ?? '9999'));
}

/** The coordinate a leg's destination carries, if any. */
export function stageCoordinate(stage: TripStage): [number, number] | null {
  const { latitude, longitude } = stage.destination;
  return latitude != null && longitude != null ? [latitude, longitude] : null;
}

export interface Host {
  entityId: string;
  person: string;
  placeName: string;
  distanceKm: number;
  note: string | null;
}


/** The stays already on the plan, for the leg's view of where to sleep. */
export function staysOf(items: PlanItem[]): { title: string; checkIn: string | null; checkOut: string | null }[] {
  return items
    .filter((item) => item.item_type === 'stay')
    .map((item) => {
      const payload = (item.payload ?? {}) as { check_in?: string; check_out?: string };
      return { title: item.title, checkIn: payload.check_in ?? null, checkOut: payload.check_out ?? null };
    });
}

/**
 * People within the radius of a leg's destination, from entities' `/api/located` for the
 * leg's day (PRD Q117: entities is the system of record). Nearest first. `hosts` are those
 * whose sleeping option is `ask` or `yes`.
 */
export function aroundFromLocated(
  destination: [number, number] | null,
  located: LocatedPerson[],
): { around: AroundPerson[]; hosts: Host[] } {
  if (!destination) return { around: [], hosts: [] };
  const around: (AroundPerson & { host: boolean; note: string | null })[] = [];
  for (const person of located) {
    if (person.latitude == null || person.longitude == null) continue;
    const distanceKm = haversineKm(destination, [person.latitude, person.longitude]);
    if (distanceKm > AROUND_RADIUS_KM) continue;
    around.push({
      entityId: person.entity_id,
      person: person.name,
      placeName: person.place,
      distanceKm,
      host: person.sleeping_option === 'ask' || person.sleeping_option === 'yes',
      note: person.sleeping_note,
    });
  }
  around.sort((a, b) => a.distanceKm - b.distanceKm);
  return {
    around: around.map(({ entityId, person, placeName, distanceKm }) => ({ entityId, person, placeName, distanceKm })),
    hosts: around
      .filter((p) => p.host)
      .map(({ entityId, person, placeName, distanceKm, note }) => ({ entityId, person, placeName, distanceKm, note })),
  };
}
