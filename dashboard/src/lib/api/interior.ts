import { jsonInit, request } from './core';
import type { InventoryItem } from './inventory';

export interface InteriorViolation {
  rule: string;
  severity: 'hart' | 'weich';
  item: string | null;
  message: string;
  /**
   * The rule's own wording from the flat's `rules.toml`, for a house rule (R1…R8).
   *
   * Absent on invariants like `kollision` or `raumgrenze`: no flat declares that two pieces
   * may not overlap, so there is no text to quote. `message` says what this layout gets
   * wrong; `text` says what makes it a rule at all.
   */
  text?: string;
  measured: number | null;
  required: number | null;
}

/** A rule the flat declares and the engine does not check. */
export interface InteriorUncheckedRule {
  rule: string;
  text: string;
  /**
   * Why it did not run. Two kinds, one consequence.
   *
   * "not implemented" — the flat declares it, the engine has no check for it.
   * "not applicable"  — the engine has one, but a measurement or declaration is missing. This
   *                     is the dangerous kind: it reads as a pass. The wardrobe sat in the light
   *                     corridor with no measured height in three layouts, and R3 skipped it in
   *                     silence.
   */
  grund: string;
}

/**
 * How much room a check has left — recorded whether it fires or not.
 *
 * A verdict is one bit, and one bit cannot say whether a layout passes by 2 cm or by 40. Both
 * report `pass`, and only one survives a tape measure that was a centimetre out.
 */
export interface InteriorReserve {
  rule: string;
  item?: string;
  /** The other side of the comparison where there is one: the opening, the built-in, the route. */
  bezug?: string;
  measured: number;
  required: number;
  grenze: 'mindestens' | 'hoechstens';
  /** What is left. **Negative means missed by that much.** */
  slack: number;
  /** `cm`, `seiten`, `plaetze`, `stunden`. Only `cm` feeds `engste_reserve_cm`. */
  einheit: string;
  hart: boolean;
  /** The measurement hit its horizon: it is *at least* this much, not exactly. */
  gedeckelt: boolean;
  /**
   * Does a POSITIVE value here say anything about how much room is left?
   *
   * True for almost every rule. False for the room boundary: a wardrobe against the wall sits
   * a centimetre from it because that is where it belongs. Counting that would make every
   * sensibly furnished flat report "1 cm spare".
   */
  bindend: boolean;
}

/** How much measurement error a verdict survives. */
export type InteriorHaltbarkeit =
  | { art: 'faellt_durch' }
  | { art: 'nichts_geraten' }
  | { art: 'bis'; cm: number }
  | { art: 'ueber_horizont'; horizont_cm: number };

export interface InteriorRobustheit {
  layout: string;
  nominal_pass: boolean;
  engste_reserve_cm: number | null;
  haelt: InteriorHaltbarkeit;
  /** Which rules break first, one centimetre past `haelt`. */
  kippt_an: string[];
  geraten: { reference: string; label: string; fields: string[] }[];
  /** Pieces carrying their own `size:` in the layout — the perturbation does not reach them. */
  nicht_variiert: string[];
}

/** Does the piece get through the door? */
export type InteriorTuerpass =
  | { art: 'passt'; luft_cm: number; tuer_cm: number }
  | { art: 'passt_nicht'; fehlen_cm: number; tuer_cm: number }
  | { art: 'zerlegt_getragen'; fehlen_cm: number; tuer_cm: number }
  | { art: 'kein_eingang' };

export interface InteriorEinbringung {
  reference: string;
  b: number;
  t: number;
  tuer: InteriorTuerpass;
  erreichbar: boolean;
  schritte?: number;
  dreht: boolean;
  grund?: string;
}

export interface InteriorSonnenstunde {
  tag: string;
  stunde_lokal: number;
  hoehe_grad: number;
  azimut_grad: number;
  getroffen: string[];
}

export interface InteriorSonne {
  layout: string;
  stunden: InteriorSonnenstunde[];
  treffer_je_stueck: Record<string, number>;
  /** Glazing without measured heights throws no light here — and must. */
  ohne_glashoehen: string[];
}

export interface InteriorDeklaration {
  id: string;
  label: string;
  erklaert: boolean;
  geraten_als: string;
  geerbte_schwellen: [string, number][];
  in_layouts: string[];
  vorschlag?: {
    open_clear: number | null;
    access_sides: number | null;
    access_clear: number | null;
    expands_dir: string | null;
    expands_to: number | null;
    toml: string;
  };
  folgen?: {
    layouts: number;
    bestanden_vorher: number;
    bestanden_nachher: number;
    geaendert: string[];
  };
}

export interface InteriorKaufposten {
  id: string;
  label: string;
  prioritaet: string | null;
  preis_cent: number | null;
  in_layouts: string[];
  kumuliert_cent: number;
  erreichbar_nach_monaten: number | null;
}

export interface InteriorKaufreihenfolge {
  posten: InteriorKaufposten[];
  /** Needs with no price at all. They are **not** in the order — zero would buy them first. */
  ohne_preis: InteriorKaufposten[];
  saldo: { median_cent: number; monate: number; von: string; bis: string } | null;
  /**
   * Priority words in the data this ranking does not know.
   *
   * They sort last and are *reported* rather than silently ranked. That is the lesson from the
   * bug itself: the first version knew three of the flat's eight words and still looked sorted.
   */
  unbekannte_prioritaeten: string[];
}

/**
 * A long computation, addressed by number.
 *
 * The exhaustive search checks millions of candidates and takes minutes. An HTTP request held
 * open that long is a bet on every proxy in between, which is why the search was reachable
 * only from the command line until 2026-08-31 — the most expensive calculation in the
 * capability, missing from the surface that needs it.
 */
export type InteriorAuftragsstand =
  | { zustand: 'laeuft'; seit_ms: number }
  | { zustand: 'fertig'; ergebnis: unknown }
  | { zustand: 'gescheitert'; grund: string };

export interface InteriorHit {
  places: Record<string, [number, number]>;
  soft: number;
  bottleneck_cm: number;
  engste_reserve_cm: number | null;
  /** On the Pareto front: not worse than another under any weighting of the four goals. */
  pareto: boolean;
  wandkontakt_cm: number;
}

export interface InteriorSearchReport {
  base: string;
  moved: string[];
  step: number;
  candidates_after_filter: number;
  fully_checked: number;
  hits: InteriorHit[];
  elapsed_ms: number;
}

export interface InteriorComposed {
  places: [string, [number, number], number][];
  pass: boolean;
  hard: string[];
  soft: string[];
  wandkontakt_cm: number;
  bottleneck_cm: number;
  free_m2: number;
  engste_reserve_cm: number | null;
  pareto: boolean;
}

/**
 * The measured room itself, as `room.toml` states it.
 *
 * `masse` is the main room's outer extent in centimetres, computed by the capability from the
 * same polygon every check rasterises — the page shows it and does not measure it, for the same
 * reason it does not decide a verdict.
 */
export interface InteriorModel {
  flat: { id: string; name: string };
  area_m2: number;
  masse: { b: number; t: number };
  /** 0 means not measured. `room.toml` invents no placeholder ceiling height. */
  hoehe: number;
  polygon: [number, number][];
  katalog_groesse: number;
  todo: string[];
}

export interface InteriorLayoutSummary {
  id: string;
  name: string;
  pass: boolean;
  hard: number;
  soft: number;
  corridors: { from: string; to: string; width_cm: number | null }[];
  occupied_m2: number;
}

/** The full check plus the finished plan. The SVG is built by the capability, never here. */
export interface InteriorLayoutDetail {
  layout: { name: string; id: string; item: InteriorPlacedItem[] };
  check: {
    layout: string;
    pass: boolean;
    hard: InteriorViolation[];
    soft: InteriorViolation[];
    uncertainties: { reference: string; label: string; fields: string[] }[];
    /**
     * Declared, not measured — so "passes" means "passes the rules that were checked".
     *
     * The real flat declares two of these (glare at the desk, the sightline from the door to
     * the bed). Before 2026-08-31 they fell out silently and a partial verdict read as a
     * complete one.
     */
    nicht_geprueft: InteriorUncheckedRule[];
    /** Pieces in this layout that have already been decided against (`prioritaet: verworfen`). */
    veraltet: string[];
    /** Every measurement with its threshold — the ones that passed too. */
    reserven: InteriorReserve[];
    /**
     * The tightest hard measurement in cm: by how much this layout passes.
     *
     * `null` when no hard rule measured in centimetres — not `0`, which would read as "passes
     * by nothing" where "nothing was measured" is meant. Negative when a hard rule is broken.
     */
    engste_reserve_cm: number | null;
    metrics: {
      room_area_m2: number;
      occupied_area_m2: number;
      free_area_m2: number;
      corridors: { from: string; to: string; width_cm: number | null }[];
    };
  };
  svg: string;
}

/**
 * The room planner.
 *
 * Every verdict, every corridor width and the plan SVG itself come from these endpoints.
 * Nothing on the page recomputes a clearance rule: a second implementation of one is exactly
 * the drift the capability exists to prevent (PRD B27).
 */
export interface InteriorRoomPlanReference {
  flat: string;
  status: "raw-only";
  revision: string | null;
  asset: { format: "usdz"; byte_length: number; sha256: string; url: string };
  observation: {
    export_observation: { room_groups: number; mesh_assets: number; category_counts: Record<string, number> };
  };
  manifest: unknown;
}

export interface InteriorRoomPlanRevisions {
  flat: string;
  revisions: import("$lib/roomplan").RoomPlanDraft[];
}

export const interior = {
  roomplanReference: () => request<InteriorRoomPlanReference>('/interior/api/roomplan/reference'),
  roomplanRevisions: () => request<InteriorRoomPlanRevisions>('/interior/api/roomplan/revisions'),
  reviewRoomplanRevision: (revisionId: string, decision: 'accept' | 'reject', note?: string) =>
    request<{ decision: string }>('/interior/api/roomplan/revisions/' + encodeURIComponent(revisionId) + '/review', jsonInit('POST', { decision, note })),
  roomplanAssetUrl: () => '/interior/api/roomplan/asset',
  model: () => request<InteriorModel>('/interior/api/model'),
  layouts: () => request<InteriorLayoutSummary[]>('/interior/api/layouts'),
  layout: (name: string) =>
    request<InteriorLayoutDetail>(`/interior/api/layouts/${encodeURIComponent(name)}`),
  /**
   * What a change would do to the verdicts, without writing it.
   *
   * The clearance fields are exactly the ones whose effect you cannot see before setting them:
   * declaring which side the wardrobe opens costs 2 or 4 layouts depending on the direction,
   * and nothing said so until it was computed by hand. The form shows the same arithmetic
   * before you save.
   */
  /** Replace a layout's positions and get the fresh verdict + plan back. The file's header survives. */
  saveLayout: (id: string, items: InteriorPlacedItem[]) =>
    request<InteriorLayoutDetail>(
      `/interior/api/layouts/${encodeURIComponent(id)}`,
      jsonInit('PUT', { items }),
    ),

  /**
   * Verdict and plan for an arrangement, without writing it.
   *
   * Needed for rotation: at 90° width and depth swap, and `opens`/`expands_dir` turn with the
   * piece. Rebuilding that in the browser would be a second copy of `footprint` and
   * `Seite::gedreht` — so the page asks instead of guessing.
   */
  previewLayout: (id: string, items: InteriorPlacedItem[]) =>
    request<InteriorLayoutDetail>(
      `/interior/api/layouts/${encodeURIComponent(id)}/preview`,
      jsonInit('POST', { items }),
    ),

  /**
   * Where a piece's top-left corner may sit, as run-lengths per grid row.
   *
   * Hard edges for dragging. The page receives a LIST and computes nothing: the main room is a
   * hexagon, so clamping to a bounding box would park furniture in the notch the bathroom
   * occupies, and doing it properly means `point_in_polygon` — which belongs in one place.
   */
  allowedPositions: (layout: string, ref: string, rot: number) =>
    request<InteriorAllowed>(
      `/interior/api/layouts/${encodeURIComponent(layout)}/allowed?ref=${encodeURIComponent(ref)}&rot=${rot}`,
    ),

  /**
   * Create a layout. Never overwrites an existing one.
   *
   * Three ways to start, one body: empty (neither `von` nor `items`), a copy of another layout
   * (`von`), or a finished arrangement (`items`). `notiz` becomes the file's first comment line
   * and is where the reasoning goes; left out, the capability writes the date and the fact that
   * the API made it, rather than claiming a provenance nobody has.
   */
  createLayout: (plan: {
    id: string;
    name?: string;
    von?: string;
    items?: InteriorPlacedItem[];
    notiz?: string;
  }) =>
    request<InteriorLayoutDetail & { id: string }>(
      '/interior/api/layouts',
      jsonInit('POST', plan),
    ),

  /**
   * Take a layout out of the list. It moves to `layouts/archiv/`; nothing is deleted.
   *
   * The file's header carries why a piece was ruled out (PRD Q60), and that argument is needed
   * exactly when the same piece comes up again. Deleting the file deletes the argument.
   */
  archiveLayout: (id: string) =>
    request<{ id: string; archiviert: string; geloescht: boolean }>(
      `/interior/api/layouts/${encodeURIComponent(id)}`,
      { method: 'DELETE' },
    ),

  /** Record where the pieces actually stand, as opposed to what a layout proposes. */
  savePlacements: (items: InteriorPlacedItem[]) =>
    request<{ flat: string; gesetzt: number }>(
      '/interior/api/placements',
      jsonInit('PUT', { items }),
    ),

  /**
   * Where the pieces actually stand, as rows.
   *
   * The as-is layer, and not a proposal: a placement is one row per piece, a layout is a file.
   * The `ref` the page drags is called `item_id` here, because that is what the column is named.
   */
  placements: (flat: string) =>
    request<Array<{ item_id: string; flat: string; x: number; y: number; rot: number }>>(
      `/interior/api/placements/${encodeURIComponent(flat)}`,
    ),

  /**
   * What the as-is arrangement looks like, and whether it would pass.
   *
   * A second endpoint beside `previewLayout` because the arrangement arrives in the body: a
   * placement is a row in the store, not a file whose header carries the reason a piece stands
   * where it does (PRD Q60). The plan and the verdict are the same computation either way.
   */
  previewPlacements: (items: InteriorPlacedItem[]) =>
    request<InteriorLayoutDetail>('/interior/api/placements/preview', jsonInit('POST', { items })),

  /** Hard edges for dragging in the as-is layer. The arrangement it must fit around is the body. */
  allowedPositionsFor: (items: InteriorPlacedItem[], ref: string, rot: number) =>
    request<InteriorAllowed>(
      '/interior/api/placements/allowed',
      jsonInit('POST', { items, ref, rot }),
    ),

  /** URL for a picture stored in the overlay. Nothing is embedded; it is fetched when shown. */
  mediaUrl: (path: string) =>
    `/interior/api/media/${path.split('/').map(encodeURIComponent).join('/')}`,

  impact: (id: string, patch: Partial<InventoryItem>) =>
    request<InteriorImpact>(
      `/interior/api/items/${encodeURIComponent(id)}/impact`,
      jsonInit('POST', patch),
    ),

  /** Up to how much measurement error the verdict holds, and what breaks first past it. */
  toleranz: (layout: string) =>
    request<InteriorRobustheit>(
      `/interior/api/layouts/${encodeURIComponent(layout)}/toleranz`,
    ),

  /** When over the year each piece stands in direct sun. Needs `[lage]` in the flat's room.toml. */
  sonne: (layout: string) =>
    request<InteriorSonne>(`/interior/api/layouts/${encodeURIComponent(layout)}/sonne`),

  /** Does each piece get through the door and to its place? */
  einbringung: (layout: string) =>
    request<InteriorEinbringung[]>(
      `/interior/api/layouts/${encodeURIComponent(layout)}/einbringung`,
    ),

  /** The question before buying: does a piece this size fit through the entrance at all? */
  passt: (b: number, t: number, zerlegbar = false) =>
    request<InteriorTuerpass>(
      `/interior/api/passt?b=${b}&t=${t}&zerlegbar=${zerlegbar}`,
    ),

  /** Who is still judged by name, what the line would say, and what it changes. */
  deklaration: () => request<InteriorDeklaration[]>('/interior/api/deklaration'),

  /** Which need first, cumulative, and when it is reached out of the monthly balance. */
  kaufen: () => request<InteriorKaufreihenfolge>('/interior/api/kaufen'),

  /** Start a search. Answers with a job number, not a result. */
  search: (layout: string, move_refs: string[], step: number, limit: number) =>
    request<{ auftrag: number }>(
      '/interior/api/search',
      jsonInit('POST', { layout, move_refs, step, limit }),
    ),

  /** Have the machine lay out the whole flat. Also a job: the beam search takes minutes. */
  compose: (refs: string[], step: number, beam: number, limit: number) =>
    request<{ auftrag: number }>(
      '/interior/api/compose',
      jsonInit('POST', { refs, step, beam, limit }),
    ),

  auftrag: (id: number) =>
    request<{ id: number; stand: InteriorAuftragsstand }>(
      `/interior/api/auftraege/${id}`,
    ),
};

export interface InteriorPlacedItem {
  ref: string;
  x: number;
  y: number;
  rot: number;
  /** Overrides the catalogue footprint — the second state of a folding piece. */
  size?: [number, number] | null;
  kind?: string | null;
}

export interface InteriorAllowed {
  reference: string;
  rot: number;
  w: number;
  d: number;
  step: number;
  /** Per row: the inclusive x ranges the top-left corner may take. */
  rows: { y: number; x: [number, number][] }[];
}

export interface InteriorImpact {
  item: string;
  layouts: number;
  bestanden_vorher: number;
  bestanden_nachher: number;
  geaendert: {
    layout: string;
    vorher: { pass: boolean; hard: string[]; soft: string[] };
    nachher: { pass: boolean; hard: string[]; soft: string[] };
  }[];
}
