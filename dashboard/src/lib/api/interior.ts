import { jsonInit, request } from './core';
import type { InventoryItem, MonthlyBalance } from './inventory';

export interface InteriorViolation {
  rule: string;
  severity: 'hard' | 'soft';
  item?: string;
  message: string;
  /** The rule's wording from `rules.toml`; absent on invariants like `collision`. */
  text?: string;
  measured?: number;
  required?: number;
}

/** A rule the flat declares and the engine did not check, and why. */
export interface InteriorUncheckedRule {
  rule: string;
  text: string;
  reason: string;
}

/** How much room a check has left, recorded whether it fires or not. */
export interface InteriorReserve {
  rule: string;
  item?: string;
  /** The other side of the comparison where there is one: the opening, fixture or route. */
  subject?: string;
  measured: number;
  required: number;
  limit: 'at_least' | 'at_most';
  /** What is left; negative means missed by that much. */
  slack: number;
  /** `cm`, `sides` or `seats`. Only `cm` feeds `tightest_reserve_cm`. */
  unit: string;
  hard: boolean;
  /** The measurement hit its horizon: at least this much, not exactly. */
  capped: boolean;
  /** Whether a positive value says how much room is left; false for the room boundary. */
  binding: boolean;
}

/** How much measurement error a verdict survives. */
export type InteriorHolds =
  | { kind: 'fails' }
  | { kind: 'nothing_guessed' }
  | { kind: 'up_to'; cm: number }
  | { kind: 'beyond_horizon'; horizon_cm: number };

export interface InteriorRobustness {
  layout: string;
  nominal_pass: boolean;
  tightest_reserve_cm: number | null;
  holds: InteriorHolds;
  /** Which rules break first, one centimetre past `holds`. */
  tips_at: string[];
  guessed: { reference: string; label: string; fields: string[] }[];
  /** Pieces carrying their own `size:` in the layout; the perturbation does not reach them. */
  not_varied: string[];
}

/** Does the piece get through the door? */
export type InteriorDoorFit =
  | { kind: 'fits'; air_cm: number; door_cm: number }
  | { kind: 'does_not_fit'; missing_cm: number; door_cm: number }
  | { kind: 'carried_apart'; missing_cm: number; door_cm: number }
  | { kind: 'no_entrance' };

export interface InteriorDelivery {
  reference: string;
  width: number;
  depth: number;
  door: InteriorDoorFit;
  affordable: boolean;
  steps?: number;
  rotates: boolean;
  reason?: string;
}

export interface InteriorSunHour {
  day: string;
  hour_local: number;
  altitude_deg: number;
  azimuth_deg: number;
  hit: string[];
}

export interface InteriorSun {
  layout: string;
  hours: InteriorSunHour[];
  hits_per_piece: Record<string, number>;
  /** Glazing without measured heights throws no light here, and should. */
  without_glass_heights: string[];
}

export interface InteriorDeclaration {
  id: string;
  label: string;
  declared: boolean;
  guessed_as: string;
  inherited_thresholds: [string, number][];
  in_layouts: string[];
  proposal?: {
    open_clear: number | null;
    access_sides: number | null;
    access_clear: number | null;
    expands_dir: string | null;
    expands_to: number | null;
    toml: string;
  };
  consequences?: {
    layouts: number;
    passing_before: number;
    passing_after: number;
    changed: string[];
  };
}

export interface InteriorPurchaseLine {
  id: string;
  label: string;
  priority: string | null;
  price_cents: number | null;
  in_layouts: string[];
  cumulative_cents: number;
  affordable_after_months: number | null;
}

export interface InteriorPurchaseOrder {
  lines: InteriorPurchaseLine[];
  /** Needs with no price at all; not in the order, since zero would buy them first. */
  unpriced: InteriorPurchaseLine[];
  balance: MonthlyBalance | null;
  /** Priority words in the data the ranking does not know; reported, not silently ranked. */
  unknown_priorities: string[];
}

/** A long computation, addressed by number: the exhaustive search takes minutes. */
export type InteriorJobState =
  | { state: 'running'; since_ms: number }
  | { state: 'done'; outcome: unknown }
  | { state: 'failed'; reason: string };

export interface InteriorHit {
  places: Record<string, [number, number]>;
  soft: number;
  bottleneck_cm: number;
  tightest_reserve_cm: number | null;
  /** On the Pareto front: not worse than another under any weighting of the four goals. */
  pareto: boolean;
  wall_contact_cm: number;
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
  wall_contact_cm: number;
  bottleneck_cm: number;
  free_m2: number;
  tightest_reserve_cm: number | null;
  pareto: boolean;
}

/** The measured room as `room.toml` states it; `dimensions` is computed from the polygon. */
export interface InteriorModel {
  flat: { id: string; name: string };
  area_m2: number;
  dimensions: { width: number; depth: number };
  /** 0 means not measured. */
  height: number;
  polygon: [number, number][];
  catalogue_size: number;
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
    /** Declared but not measured, so "passes" means passes the rules that were checked. */
    unchecked: InteriorUncheckedRule[];
    /** Pieces in this layout already decided against (`priority: rejected`). */
    outdated: string[];
    /** Every measurement with its threshold, the passing ones too. */
    reserves: InteriorReserve[];
    /** The tightest hard measurement in cm. `null` when none was measured; negative when broken. */
    tightest_reserve_cm: number | null;
    metrics: {
      room_area_m2: number;
      occupied_area_m2: number;
      free_area_m2: number;
      corridors: { from: string; to: string; width_cm: number | null }[];
    };
  };
  svg: string;
}

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

/** The room planner. Every verdict, width and plan comes from these endpoints; the page computes none. */
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

  /** Replace a layout's positions and get the fresh verdict and plan back. The file header survives. */
  saveLayout: (id: string, items: InteriorPlacedItem[]) =>
    request<InteriorLayoutDetail>(
      `/interior/api/layouts/${encodeURIComponent(id)}`,
      jsonInit('PUT', { items }),
    ),

  /** Verdict and plan without writing; needed for rotation, where sizes and declared sides turn. */
  previewLayout: (id: string, items: InteriorPlacedItem[]) =>
    request<InteriorLayoutDetail>(
      `/interior/api/layouts/${encodeURIComponent(id)}/preview`,
      jsonInit('POST', { items }),
    ),

  /** Where a piece's top-left corner may sit, as runs per grid row: hard edges for dragging. */
  allowedPositions: (layout: string, ref: string, rot: number) =>
    request<InteriorAllowed>(
      `/interior/api/layouts/${encodeURIComponent(layout)}/allowed?ref=${encodeURIComponent(ref)}&rot=${rot}`,
    ),

  /**
   * Create a layout; never overwrites one. Empty (neither `from` nor `items`), a copy (`from`),
   * or placed (`items`). `note` becomes the file's first comment line.
   */
  createLayout: (plan: {
    id: string;
    name?: string;
    from?: string;
    items?: InteriorPlacedItem[];
    note?: string;
  }) =>
    request<InteriorLayoutDetail & { id: string }>(
      '/interior/api/layouts',
      jsonInit('POST', plan),
    ),

  /** Take a layout off the list. It moves to `layouts/archiv/`; nothing is deleted. */
  archiveLayout: (id: string) =>
    request<{ id: string; archived: string; deleted: boolean }>(
      `/interior/api/layouts/${encodeURIComponent(id)}`,
      { method: 'DELETE' },
    ),

  /** Record where the pieces actually stand, as opposed to what a layout proposes. */
  savePlacements: (items: InteriorPlacedItem[]) =>
    request<{ flat: string; placed: number }>(
      '/interior/api/placements',
      jsonInit('PUT', { items }),
    ),

  /** Where the pieces actually stand, one row per piece. */
  placements: (flat: string) =>
    request<Array<{ item_id: string; flat: string; x: number; y: number; rot: number }>>(
      `/interior/api/placements/${encodeURIComponent(flat)}`,
    ),

  /** The as-is arrangement's plan and verdict; the arrangement comes in the body. */
  previewPlacements: (items: InteriorPlacedItem[]) =>
    request<InteriorLayoutDetail>('/interior/api/placements/preview', jsonInit('POST', { items })),

  /** Hard edges for dragging in the as-is layer. */
  allowedPositionsFor: (items: InteriorPlacedItem[], ref: string, rot: number) =>
    request<InteriorAllowed>(
      '/interior/api/placements/allowed',
      jsonInit('POST', { items, ref, rot }),
    ),

  /** URL for a picture stored in the overlay, fetched when shown. */
  mediaUrl: (path: string) =>
    `/interior/api/media/${path.split('/').map(encodeURIComponent).join('/')}`,

  /** What a change would do to the verdicts, without writing it. */
  impact: (id: string, patch: Partial<InventoryItem>) =>
    request<InteriorImpact>(
      `/interior/api/items/${encodeURIComponent(id)}/impact`,
      jsonInit('POST', patch),
    ),

  /** Up to how much measurement error the verdict holds, and what breaks first past it. */
  tolerance: (layout: string) =>
    request<InteriorRobustness>(
      `/interior/api/layouts/${encodeURIComponent(layout)}/tolerance`,
    ),

  /** When over the year each piece stands in direct sun. Needs a location in room.toml. */
  sun: (layout: string) =>
    request<InteriorSun>(`/interior/api/layouts/${encodeURIComponent(layout)}/sun`),

  /** Does each piece get through the door and to its place? */
  delivery: (layout: string) =>
    request<InteriorDelivery[]>(
      `/interior/api/layouts/${encodeURIComponent(layout)}/delivery`,
    ),

  /** The question before buying: does a piece this size fit through the entrance at all? */
  fits: (width: number, depth: number, disassembles = false) =>
    request<InteriorDoorFit>(
      `/interior/api/fits?width=${width}&depth=${depth}&disassembles=${disassembles}`,
    ),

  /** Which items are still judged by name, what their declaration would be, and what it changes. */
  declarations: () => request<InteriorDeclaration[]>('/interior/api/declarations'),

  /** Which need first, cumulated, and when the monthly balance affords it. */
  purchaseOrder: () => request<InteriorPurchaseOrder>('/interior/api/purchase-order'),

  /** Start a search. Answers with a job number, not a result. */
  search: (layout: string, move_refs: string[], step: number, limit: number) =>
    request<{ job: number }>(
      '/interior/api/search',
      jsonInit('POST', { layout, move_refs, step, limit }),
    ),

  /** Have the machine lay out the whole flat. Also a job: the beam search takes minutes. */
  compose: (refs: string[], step: number, beam: number, limit: number) =>
    request<{ job: number }>(
      '/interior/api/compose',
      jsonInit('POST', { refs, step, beam, limit }),
    ),

  job: (id: number) =>
    request<{ id: number; state: InteriorJobState }>(
      `/interior/api/jobs/${id}`,
    ),
};

export interface InteriorPlacedItem {
  ref: string;
  x: number;
  y: number;
  rot: number;
  /** Overrides the catalogue footprint, e.g. the unfolded state of a folding piece. */
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
  passing_before: number;
  passing_after: number;
  changed: {
    layout: string;
    before: { pass: boolean; hard: string[]; soft: string[] };
    after: { pass: boolean; hard: string[]; soft: string[] };
  }[];
}
