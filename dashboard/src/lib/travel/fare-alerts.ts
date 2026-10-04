// Local notifications for a new Sparpreis low.
//
// tools/sparpreis-watch writes a `note` item, `sparpreis-low:<watch>:<day>`, when a
// fare falls below every earlier observation. That note is the durable alert (travel PRD
// R4). This module turns an unseen one into a phone notification when the app opens or
// returns to the foreground. It is local: no APNs and no push key (operator's choice,
// 2026-09-25), so an alert waits until the app is next opened.

import { inTauri } from '../mac-bridge';
import { trips } from '../api';

const SEEN_KEY = 'axon.fare-alerts.seen';
const LAST_CHECK_KEY = 'axon.fare-alerts.last-check';
/** One check reads every upcoming plan, so it runs at most this often. */
const MIN_INTERVAL_MS = 10 * 60 * 1000;

export interface FareAlert {
  id: string;
  title: string;
}

interface NoteLike {
  item_type?: string;
  external_id?: string;
  title?: string;
}

/** The new-low notes in `items` that have not been announced yet. */
export function unseenLows(items: NoteLike[], seen: ReadonlySet<string>): FareAlert[] {
  return items
    .filter(
      (item) =>
        item.item_type === 'note' &&
        item.external_id?.startsWith('sparpreis-low:') &&
        !seen.has(item.external_id),
    )
    .map((item) => ({ id: item.external_id!, title: item.title ?? 'A fare fell to a new low' }));
}

/** Whether a plan is still ahead or running: its last day is today or later. */
export function upcoming(plan: { date_start: string; date_end?: string | null }, today: string): boolean {
  return (plan.date_end ?? plan.date_start) >= today;
}

function readSeen(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(SEEN_KEY) ?? '[]') as string[]);
  } catch {
    return new Set();
  }
}

/** Checks upcoming plans for new lows and notifies once per note. Never throws. */
export async function checkFareAlerts(now = Date.now()): Promise<void> {
  if (!inTauri()) return;
  const last = Number(localStorage.getItem(LAST_CHECK_KEY) ?? 0);
  if (now - last < MIN_INTERVAL_MS) return;
  localStorage.setItem(LAST_CHECK_KEY, String(now));
  try {
    const today = new Date(now).toISOString().slice(0, 10);
    const seen = readSeen();
    const alerts: FareAlert[] = [];
    for (const plan of (await trips.list()).filter((p) => upcoming(p, today))) {
      const details = await trips.get(plan.id);
      alerts.push(...unseenLows(details.items ?? [], seen));
    }
    if (!alerts.length) return;

    const { isPermissionGranted, requestPermission, sendNotification } = await import(
      '@tauri-apps/plugin-notification'
    );
    let granted = await isPermissionGranted();
    if (!granted) granted = (await requestPermission()) === 'granted';
    if (!granted) return;
    for (const alert of alerts) {
      sendNotification({ title: 'New low fare', body: alert.title });
      seen.add(alert.id);
    }
    localStorage.setItem(SEEN_KEY, JSON.stringify([...seen]));
  } catch (err) {
    console.warn('fare alert check failed', err);
  }
}
