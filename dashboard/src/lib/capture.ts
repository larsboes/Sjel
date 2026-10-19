// The capture inbox (comms `/captures`) and where each item goes from it.
import { request, send } from './api';

export const TAGS = ['buy', 'look', 'go', 'learn'] as const;
export type Tag = (typeof TAGS)[number];

export interface Capture {
  id: string;
  text: string | null;
  url: string | null;
  tag: Tag | null;
  status: 'new' | 'done' | 'dismissed';
  routed_to: string | null;
  client: string | null;
  created_at: string;
}

export type Route = 'wishlist' | 'feed' | 'places';

/** Domains the comms feed has a reader for (capabilities/comms/src/media). */
const FEED_HOSTS = ['github.com', 'reddit.com', 'arxiv.org', 'huggingface.co', 'youtube.com', 'youtu.be', 'instagram.com'];

function host(url: string | null): string | null {
  try {
    return url ? new URL(url).hostname.replace(/^www\./, '') : null;
  } catch {
    return null;
  }
}

/** The tag the rules give an untagged capture. A set tag always wins. */
export function suggestTag(c: Pick<Capture, 'tag' | 'url'>): Tag | null {
  if (c.tag) return c.tag;
  const h = host(c.url);
  return h && FEED_HOSTS.some((f) => h === f || h.endsWith(`.${f}`)) ? 'look' : null;
}

/** Where a capture can go. Text-only notes without a tag stay here. */
export function routeFor(c: Pick<Capture, 'tag' | 'url'>): Route | null {
  const tag = suggestTag(c);
  if (tag === 'buy') return 'wishlist';
  if ((tag === 'look' || tag === 'learn') && c.url) return 'feed';
  if (tag === 'go') return 'places';
  return null;
}

import { centsFrom } from './inventory/money';

export function label(c: Pick<Capture, 'text' | 'url'>): string {
  return c.text || host(c.url) || c.url || '';
}

/**
 * Extracts a typed-in price from text at triage, e.g. "Linen Shirt 49.90 €" ->
 * `{ label: "Linen Shirt", price_cents: 4990 }`.
 */
export function extractPrice(text: string | null | undefined): { label: string; price_cents?: number } {
  if (!text) return { label: '' };
  const trimmed = text.trim();
  const words = trimmed.split(/\s+/);
  if (words.length >= 2) {
    const last2 = words.slice(-2).join(' ');
    const cents2 = centsFrom(last2);
    if (typeof cents2 === 'number') {
      return { label: words.slice(0, -2).join(' '), price_cents: cents2 };
    }
    const last1 = words[words.length - 1];
    const cents1 = centsFrom(last1);
    if (typeof cents1 === 'number') {
      return { label: words.slice(0, -1).join(' '), price_cents: cents1 };
    }
  }
  return { label: trimmed };
}

/** `Proper shoes (On?)` -> `proper-shoes-on`, the same shape inventory's own ids have. */
export function slug(text: string): string {
  return text
    .toLowerCase()
    .replace(/ä/g, 'a').replace(/ö/g, 'o').replace(/ü/g, 'u').replace(/ß/g, 's')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '') || 'wunsch';
}

const json = (method: string, body: unknown): RequestInit => ({
  method,
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify(body),
});

export const captures = {
  list: (status?: string) => request<Capture[]>(`/comms/captures${status ? `?status=${status}` : ''}`),
  add: (body: { text?: string; url?: string; tag?: Tag }) =>
    request<Capture>('/comms/captures', json('POST', { ...body, client: 'dashboard' })),
  patch: (id: string, patch: Partial<Pick<Capture, 'text' | 'tag' | 'status' | 'routed_to'>>) =>
    request<Capture>(`/comms/captures/${encodeURIComponent(id)}`, json('PATCH', patch)),
};

/** Sends a capture on and marks it done. The capture is left untouched when the target refuses. */
export async function route(c: Capture, to: Route): Promise<Capture> {
  let target: string;
  if (to === 'feed') {
    const item = await request<{ id: string }>('/comms/ingest', json('POST', { url: c.url, client: 'inbox' }));
    target = `feed:${item.id}`;
  } else if (to === 'places') {
    const text = c.text?.trim() || undefined;
    const url = c.url?.trim() || undefined;
    const name = text || host(url ?? null) || url || 'Unbenannter Ort';
    const data = await request<{ ok: boolean; id: string }>(
      '/places/api/places',
      json('POST', { name, link: url, note: text, query: text || url }),
    );
    target = `places:${data.id}`;
  } else {
    const { label: cleanLabel, price_cents } = extractPrice(c.text);
    const itemLabel = cleanLabel || host(c.url) || c.url || 'wunsch';
    const base = slug(itemLabel);
    for (let n = 1; ; n++) {
      const id = n === 1 ? base : `${base}-${n}`;
      const payload: Record<string, unknown> = {
        id,
        kind: 'piece',
        label: itemLabel,
        link: c.url,
        state: 'wanted',
        note: 'aus der Inbox',
      };
      if (typeof price_cents === 'number') {
        payload.price_cents = price_cents;
      }
      const answer = await send(
        '/inventory/api/items',
        json('POST', payload),
      );
      if (answer.ok) {
        target = `inventory:${id}`;
        break;
      }
      if (answer.status !== 409 || n > 20) throw new Error(answer.text || `inventory answered ${answer.status}`);
    }
  }
  return captures.patch(c.id, { status: 'done', routed_to: target });
}
