import { ApiError, jsonInit, request } from './core';

export type Side = 'north' | 'south' | 'east' | 'west';

/** One owned or wanted thing; lengths in cm, money in cents. Mirrors `capabilities/inventory/src/store.rs::Item`. */
export interface InventoryItem {
  id: string;
  kind: 'piece' | 'slot' | 'gear';
  label: string;
  width: number | null;
  depth: number | null;
  height: number | null;
  height_min: number | null;
  length: number | null;
  count: number | null;
  /** Which dimensions are guesses. */
  estimated: string[];
  fold_states: string[];
  price_cents: number | null;
  cost_min_cents: number | null;
  cost_max_cents: number | null;
  link: string | null;
  source: string | null;
  measured_at: string | null;
  on_move: string | null;
  priority: string | null;
  target: string | null;
  notes: string | null;
  reason: string | null;
  /** Ids of the items this one makes redundant. */
  replaces: string[];

  // What the piece needs around it; null leaves the name heuristic in charge. Sides are in the
  // piece's own orientation and turn with `rot`.
  opens: Side | null;
  open_clear: number | null;
  wall_ok: boolean | null;
  expands_dir: Side | null;
  /** Total extent unfolded, not the increase. */
  expands_to: number | null;
  access_sides: number | null;
  access_clear: number | null;
  /** Meant to stand free. Affects search ranking only. */
  room_divider: boolean | null;
  /** A path below the private interior asset root, served on request only. */
  image: string | null;

  /** Free text, e.g. `kleidung`, `elektronik`. */
  category: string | null;
  size: string | null;
  color: string | null;
  seasons: string[];
  traits?: string[];
  weight_g?: number | null;
  waterproof?: boolean | null;
  quick_dry?: boolean | null;
  pack_location?: string | null;
  /** Owned by the server. Send it back to `saveItem`/`patchItem` so a raced write fails with `InventoryConflict`. */
  revision: number;
}

/**
 * What `PUT`/`PATCH /api/items/:id` answer on success. In the app, an edit made while the Mac
 * cannot be reached is queued on the device instead (`src-tauri/src/sync.rs`): the answer then
 * carries `queued: true`, and `revision` only once the queue has sent it (`state: 'sent'`).
 */
export type InventoryWriteResult =
  | { id: string; ok: boolean; revision: number; queued?: undefined }
  | { queued: true; outbox_id: number; state: 'pending' | 'sent' | 'conflict' | 'failed'; revision?: number };

/**
 * One inventory row. `pending` and `conflict` are set only in the app, by the device's outbox:
 * `pending` means the item shows an edit that has not reached the Mac yet.
 */
export interface InventoryRow {
  item: InventoryItem;
  state: InventoryState | null;
  pending?: boolean;
  conflict?: boolean;
}

/**
 * A write carried a stale revision: another device changed the entry after this page read it.
 *
 * `current` is the entry as it stands now, so the page can show it instead of retrying and
 * overwriting. It is read again after the 409 because `request()` keeps only the message of an
 * error body; `null` if that re-read fails.
 */
export class InventoryConflict extends ApiError {
  current: { item: InventoryItem; state: InventoryState | null } | null;
  constructor(message: string, current: InventoryConflict['current']) {
    super(409, message);
    this.name = 'InventoryConflict';
    this.current = current;
  }
}

/** `init` with `If-Match: "<revision>"` added, or unchanged when no revision is known. */
function ifMatch(init: RequestInit, revision: number | undefined): RequestInit {
  if (revision === undefined) return init;
  return { ...init, headers: { ...(init.headers as Record<string, string>), 'If-Match': `"${revision}"` } };
}

/** One conditional item write. A 409 becomes `InventoryConflict`; it is never retried. */
async function writeItem(id: string, init: RequestInit, revision: number | undefined): Promise<InventoryWriteResult> {
  try {
    return await request<InventoryWriteResult>(`/inventory/api/items/${encodeURIComponent(id)}`, ifMatch(init, revision));
  } catch (caught) {
    if (!(caught instanceof ApiError) || caught.status !== 409) throw caught;
    const current = await request<{ item: InventoryItem; state: InventoryState | null }[]>('/inventory/api/inventory')
      .then((rows) => rows.find((row) => row.item.id === id) ?? null)
      .catch(() => null);
    throw new InventoryConflict(caught.message, current);
  }
}

export type InventoryState = 'owned' | 'wanted' | 'gone';

export interface MonthlyBalance {
  median_cents: number;
  months: number;
  from: string;
  to: string;
}

export interface InventoryWishlist {
  items: InventoryItem[];
  total_low_cents: number;
  total_high_cents: number;
  unpriced_count: number;
  monthly_balance: MonthlyBalance | null;
  months_to_pay: number | null;
}

/** The household's rows (capabilities/inventory): owned, wanted and gone, and the wishlist budget. */
export const inventory = {
  rows: () => request<InventoryRow[]>('/inventory/api/inventory'),
  wishlist: () => request<InventoryWishlist>('/inventory/api/wishlist'),
  /**
   * Replace an entry. Pass the `revision` you read: a stale one throws `InventoryConflict`.
   * Without it the later write wins, which a client that can be offline must not rely on.
   */
  saveItem: (id: string, item: InventoryItem, revision?: number) =>
    writeItem(id, jsonInit('PUT', item), revision),

  /**
   * Change named fields and leave the rest alone.
   *
   * Prefer this over `saveItem` from a form: `PUT` blanks every field the form does not send.
   * An explicit `null` clears a field, an absent key leaves it, an unknown key is refused.
   */
  patchItem: (id: string, patch: Partial<InventoryItem>, revision?: number) =>
    writeItem(id, jsonInit('PATCH', patch), revision),

  /** Create an entry. `state` is required: without it the row joins to nothing and is invisible. */
  createItem: (item: Partial<InventoryItem> & { id: string; kind: 'piece' | 'slot'; label: string; state: InventoryState; note?: string }) =>
    request<{ id: string; state: InventoryState; ok: boolean }>(
      '/inventory/api/items',
      jsonInit('POST', item),
    ),

  /**
   * Append a state change; a bought wish is a second row. `changed: false` means the state
   * already held.
   */
  setState: (id: string, state: InventoryState, note?: string) =>
    request<{ id: string; state: InventoryState; changed: boolean }>(
      `/inventory/api/items/${encodeURIComponent(id)}/state`,
      jsonInit('POST', { state, note }),
    ),

  stateHistory: (id: string) =>
    request<{ state: InventoryState; since: string; note: string | null }[]>(
      `/inventory/api/items/${encodeURIComponent(id)}/state`,
    ),
};
