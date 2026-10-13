import { ApiError, jsonInit, request } from './core';

/** One measured or wanted thing. Mirrors `capabilities/interior/src/store.rs::Item`. */
export interface InventoryItem {
  id: string;
  kind: 'piece' | 'slot' | 'gear';
  label: string;
  b: number | null;
  t: number | null;
  h: number | null;
  h_min: number | null;
  laenge: number | null;
  anzahl: number | null;
  unsicher: string[];
  zustaende: string[];
  preis_cent: number | null;
  kosten_min_cent: number | null;
  kosten_max_cent: number | null;
  link: string | null;
  quelle: string | null;
  gemessen_am: string | null;
  mitnahme: string | null;
  prioritaet: string | null;
  ziel: string | null;
  hinweis: string | null;
  begruendung: string | null;
  /** Ids of the items this one makes redundant: a wanted phone names the owned phone it replaces. */
  ersetzt: string[];

  /**
   * What this piece states it needs (PRD Q61 / B26).
   *
   * Fill any of them and the clearance check measures the piece against exactly these and stops
   * guessing from its name — the guess that once checked a coffee table against a sofa's rules,
   * because `^couch` caught `couchtisch`. Leave them null and the name still decides.
   *
   * `opens` and `expands_dir` are in the piece's own orientation and turn with `rot`.
   */
  opens: 'nord' | 'sued' | 'ost' | 'west' | null;
  open_clear: number | null;
  wall_ok: boolean | null;
  expands_dir: 'nord' | 'sued' | 'ost' | 'west' | null;
  /** Total extent when unfolded, not the increase — a product page names the total. */
  expands_to: number | null;
  access_sides: number | null;
  access_clear: number | null;
  /** Meant to stand free. Affects the search ranking only, never a verdict. */
  raumtrenner: boolean | null;
  /**
   * A picture of this piece, as a path below the private interior asset root.
   *
   * Served by `GET /interior/api/media/<path>` on request only — the reason the capability may
   * sit in a public repository is that the bundle carries no photograph.
   */
  bild: string | null;

  /**
   * What the piece is FOR — `kleidung`, `schlafen`, `kochen`, `elektronik`. Free text: the list
   * comes from the notes and no rule runs on it. This is what makes a garment a garment; the
   * `kind` stays `piece`, because a shirt is a thing you own and not a need with target sizes.
   */
  category: string | null;
  /**
   * Clothing, as the label writes it — `M`, `42`, `60x60`. Free text on purpose: an invented
   * scale (`S..XXL`) would be the next shop that does not fit it.
   */
  groesse: string | null;
  /** `weiss`, `dunkelblau`, `gestreift`. Free, for the same reason. */
  farbe: string | null;
  /**
   * When it is worn — `["ganzjahr"]`, `["winter"]`, `["uebergang"]`. A list, because a coat
   * is winter AND transition, not one or the other.
   */
  saison: string[];
  /** What it does on a trip — `style`, `comfort`, `warm`, `rain`. Free words, shown not ruled. */
  traits?: string[];
  /** Equipment fields (B51); optional because rows older than that never set them. */
  weight_g?: number | null;
  waterproof?: boolean | null;
  quick_dry?: boolean | null;
  pack_location?: string | null;
  /**
   * How often the row was written; the server owns it (PRD §10 A5). Send the value you read as
   * `revision` to `saveItem`/`patchItem`, and a write that raced another device fails with
   * `InventoryConflict` instead of overwriting it.
   */
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

export interface InventoryWishlist {
  items: InventoryItem[];
  summe_untere_kante_cent: number;
  summe_obere_kante_cent: number;
  posten_ohne_preis: number;
  monatssaldo: { median_cent: number; monate: number; von: string; bis: string } | null;
  monate_bis_bezahlt: number | null;
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
   * Prefer this over `saveItem` from a form. `PUT` replaces the whole row, and an entry has 40
   * fields while a form shows six — everything it does not send would be blanked. An explicit
   * `null` clears a field; an absent key leaves it. An unknown key is refused rather than
   * ignored, which is the same stance `deny_unknown_fields` takes on import.
   *
   * Pass the `revision` the form was opened with. If another device saved in between, this
   * throws `InventoryConflict` with the current values and writes nothing.
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
   * Append a state change. Appends, never overwrites — a wish that gets bought is a second row,
   * and that span is what the wishlist later joins to money with.
   *
   * `changed: false` means the state already held: not an error, and not an invented second row.
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
