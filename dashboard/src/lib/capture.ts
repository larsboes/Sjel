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

export type Route = 'wishlist' | 'feed';

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

/** Where a capture can go. `go` and text-only notes have no destination yet; they stay here. */
export function routeFor(c: Pick<Capture, 'tag' | 'url'>): Route | null {
  const tag = suggestTag(c);
  if (tag === 'buy') return 'wishlist';
  if ((tag === 'look' || tag === 'learn') && c.url) return 'feed';
  return null;
}

export function label(c: Pick<Capture, 'text' | 'url'>): string {
  return c.text || host(c.url) || c.url || '';
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
  } else {
    const base = slug(label(c));
    for (let n = 1; ; n++) {
      const id = n === 1 ? base : `${base}-${n}`;
      const answer = await send(
        '/inventory/api/items',
        json('POST', { id, kind: 'piece', label: label(c), link: c.url, state: 'wanted', note: 'aus der Inbox' }),
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
