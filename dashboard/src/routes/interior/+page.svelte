<script lang="ts">
  /**
   * Rooms, and what stands in them.
   *
   * **This page renders and does not compute.** Every verdict, every corridor width and the
   * plan drawing itself arrive finished from `/interior/api/…`. A clearance rule reimplemented
   * here would be a second answer to a question that already has one, and drift between the
   * two is the failure the capability exists to prevent (PRD B27).
   *
   * The one arithmetic below is cents to euros for display.
   */
  import { onMount, tick, untrack } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import { tip } from "$lib/tip";
  import { link } from "$lib/nav";
  import {
    sjelStatus,
    interior,
    inventory,
    InventoryConflict,
    type InventoryItem,
    type InventoryRow,
    type InteriorLayoutDetail,
    type InteriorLayoutSummary,
    type InventoryState,
    type InventoryWishlist,
    type InteriorImpact,
    type InteriorPlacedItem,
    type InteriorReserve,
    type InteriorAllowed,
    type InteriorRobustness,
    type InteriorSun,
    type InteriorDelivery,
    type InteriorModel,
    type InteriorRoomPlanReference,
  } from "$lib/api";
  import { capabilities } from "$lib/capabilities.svelte";
  import RoomCapturePanel from "$lib/interior/RoomCapturePanel.svelte";
  import RoomPlanRevisionReview from "$lib/interior/RoomPlanRevisionReview.svelte";
  import InteriorSolverView from "$lib/interior/InteriorSolverView.svelte";
  import InteriorPurchaseView from "$lib/interior/InteriorPurchaseView.svelte";
  import SunGrid from "$lib/interior/SunGrid.svelte";
  import DataTable, { type Column } from "$lib/ui/DataTable.svelte";
  import { isRoomPlanPhone } from "$lib/roomplan";
  import { SYNC_CHANGED_EVENT } from "$lib/sync-status";
  import { bridgedSrc } from "$lib/bridged-url";
  import { getConnectionSettings } from "$lib/mac-bridge";
  import { modal } from "$lib/modal";

  type View = "plans" | "solve" | "buy";

  let view = $state<View>("plans");
  let loading = $state(true);
  let error = $state<string | null>(null);
  let phoneOnly = $state(false);
  let starting = $state(false);

  let layouts = $state<InteriorLayoutSummary[]>([]);
  let selected = $state<string | null>(null);
  let detail = $state<InteriorLayoutDetail | null>(null);
  let detailError = $state<string | null>(null);

  /** A walk between two rooms, as the check reports it. */
  type Corridor = { from: string; to: string; width_cm: number | null };
  /** A measurement with its place in the list: one rule can be measured more than once. */
  type ReserveRow = InteriorReserve & { rank: number };

  const walkColumns: Column<Corridor>[] = [
    { id: "walk", label: "Walk", cell: walkCell },
    { id: "width", label: "Narrowest", width: "9rem", align: "end", cell: widthCell },
  ];

  const reserveColumns: Column<ReserveRow>[] = [
    { id: "rule", label: "Rule", wrap: true, cell: ruleCell },
    { id: "measured", label: "Measured", width: "8rem", align: "end", cell: measuredCell },
    { id: "slack", label: "Slack", width: "7rem", align: "end", cell: slackCell },
  ];

  const reserves: ReserveRow[] = $derived(
    (detail?.check.reserves ?? []).map((reserve, index) => ({ ...reserve, rank: index })),
  );

  /** A hard rule that binds, with under 10 units left, is a number somebody has to read. */
  const tight = (row: ReserveRow): boolean => row.hard && row.binding && row.slack < 10;

  let rows = $state<InventoryRow[]>([]);
  /** The flat itself: its name, its outer measurements, its area. Measured, not drawn from. */
  let room = $state<InteriorModel | null>(null);
  let roomplanReference = $state<InteriorRoomPlanReference | null>(null);
  let roomplanRevisions = $state<import("$lib/roomplan").RoomPlanDraft[]>([]);
  let RoomPlanViewer = $state<typeof import("$lib/interior/RoomPlanViewer.svelte").default | null>(null);

  /**
   * The three questions a verdict alone cannot answer, fetched per layout and never guessed:
   * by how much does it pass, does the furniture even get in, and when is the sun on it.
   *
   * Loaded beside the layout rather than with it: each is a separate endpoint and a separate
   * cost, and a plan that will not draw until the sun is computed would be a worse plan.
   */
  let robust = $state<InteriorRobustness | null>(null);
  let sun = $state<InteriorSun | null>(null);
  let sunError = $state<string | null>(null);
  let moveIn = $state<InteriorDelivery[]>([]);


  const size = (i: InventoryItem): string =>
    i.width !== null && i.depth !== null ? `${i.width} × ${i.depth}${i.height !== null ? ` × ${i.height}` : ""} cm` : "—";

  // ─── Dragging on the plan ──────────────────────────────────────────────────
  //
  // The capability draws the plan and every piece arrives as a `<g data-ref>` carrying its
  // model coordinates in centimetres. The SVG's viewBox is in centimetres too, so screen → model
  // is one matrix inversion; the page never holds a scale factor of its own, which would be a
  // second version of the geometry.
  //
  // A drag is UI state until it is saved. Nothing is written while you move.

  let dragging = $state(false);
  let dirty = $state(false);

  /**
   * The as-is layer has no layout file, so the selector needs a name for it — and a name no real
   * id can collide with. The leading underscores make it an illegal file name, which is the point:
   * `layouts/__as-is__.toml` cannot exist, so this entry can never shadow a real plan.
   */
  const ASIS = "__as-is__";
  let asisItems = $state<InteriorPlacedItem[]>([]);
  const inAsis = $derived(selected === ASIS);
  let planError = $state<string | null>(null);
  let savingPlan = $state(false);
  let saveAs = $state("");

  /** Snapped to the grid the checker itself rasterises on, so a drop cannot land between cells. */
  const SNAP = 5;

  /**
   * The legal region for the piece currently being dragged.
   *
   * Fetched from the capability when a piece is picked up, and again after a rotation, because
   * turning swaps width and depth and therefore changes where the corner may sit. The page only
   * looks positions up in it — the geometry that produced it stays in one place.
   */
  let allowed = $state<InteriorAllowed | null>(null);

  /** Nearest legal position to a free-dragged one. Walls become edges you cannot cross. */
  function snapToAllowed(x: number, y: number): { x: number; y: number } {
    const a = allowed;
    if (!a || a.rows.length === 0) return { x, y };
    let bestRow = a.rows[0];
    for (const r of a.rows) {
      if (Math.abs(r.y - y) < Math.abs(bestRow.y - y)) bestRow = r;
    }
    let bx = bestRow.x[0][0];
    let bd = Infinity;
    for (const [lo, hi] of bestRow.x) {
      const cand = Math.min(Math.max(x, lo), hi);
      const dist = Math.abs(cand - x);
      if (dist < bd) {
        bd = dist;
        bx = cand;
      }
    }
    return { x: bx, y: bestRow.y };
  }

  /**
   * Where a piece is now, as opposed to where the capability drew it.
   *
   * `data-x`/`data-y` are the RENDERED position and are never written to: an SVG `transform` is
   * measured from where the element was drawn, so overwriting them made every drag after the
   * first render in the wrong place — it looked like the piece snapped back. `nx`/`ny` hold the
   * dragged position instead, and the transform is always taken against the rendered one.
   */
  const posOf = (g: SVGGElement) => ({
    x: Number(g.dataset.nx ?? g.dataset.x ?? 0),
    y: Number(g.dataset.ny ?? g.dataset.y ?? 0),
  });

  /** The position the capability drew this piece at. Fixed for as long as the SVG lives. */
  const drawnAt = (g: SVGGElement) => ({
    x: Number(g.dataset.x ?? 0),
    y: Number(g.dataset.y ?? 0),
  });

  /**
   * The current arrangement: position out of the drawing, every other field out of the layout.
   *
   * A drag can change x, y and rot and nothing else, so nothing else is read back off the SVG.
   * Until 2026-08-31 this built each entry from scratch and `size` fell out — the override that
   * says a table is folded OUT. Eight of the flat's layouts carry one, so moving any piece in
   * them quietly resized a different piece back to its catalogue footprint.
   */
  function itemsFromPlan(root: HTMLElement): InteriorPlacedItem[] {
    const known = new Map((detail?.layout.item ?? []).map((it) => [it.ref, it]));
    return [...root.querySelectorAll<SVGGElement>("g[data-ref]")].map((g) => {
      const ref = g.dataset.ref ?? "";
      return {
        ...known.get(ref),
        ref,
        ...posOf(g),
        rot: Number(g.dataset.rot ?? 0),
      };
    });
  }

  /** Attach drag handlers to a freshly rendered plan. Re-runs whenever the SVG is replaced. */
  function draggable(root: HTMLElement) {
    let active: SVGGElement | null = null;
    let startX = 0;
    let startY = 0;
    let originX = 0;
    let originY = 0;

    const toModel = (svg: SVGSVGElement, clientX: number, clientY: number) => {
      const pt = svg.createSVGPoint();
      pt.x = clientX;
      pt.y = clientY;
      const ctm = svg.getScreenCTM();
      if (!ctm) return { x: 0, y: 0 };
      const p = pt.matrixTransform(ctm.inverse());
      return { x: p.x, y: p.y };
    };

    // Hovering names the piece the detail line describes; picking one up pins it. Without the
    // pin the line would empty itself the moment the pointer left the rectangle, which is
    // exactly when someone is reading it.
    const over = (e: PointerEvent) => {
      const g = (e.target as Element).closest<SVGGElement>("g[data-ref]");
      if (g) focusRef = g.dataset.ref ?? null;
    };

    const out = () => {
      if (!dragging) focusRef = picked;
    };

    const down = (e: PointerEvent) => {
      const g = (e.target as Element).closest<SVGGElement>("g[data-ref]");
      if (!g) return;
      const svg = g.ownerSVGElement;
      if (!svg) return;
      active = g;
      picked = g.dataset.ref ?? null;
      focusRef = picked;
      dragging = true;
      allowed = null;
      if (picked && selected) {
        // Fetched, not computed. Until it arrives the drag is free and the drop still snaps,
        // because the answer is applied on release rather than per frame. Which base it must fit
        // around is the only difference between the two: a file in one case, the posted rows in
        // the other.
        void (inAsis
          ? interior.allowedPositionsFor(asisItems, picked, Number(g.dataset.rot ?? 0))
          : interior.allowedPositions(selected, picked, Number(g.dataset.rot ?? 0))
        )
          .then((a) => (allowed = a))
          .catch(() => (allowed = null));
      }
      const p = toModel(svg, e.clientX, e.clientY);
      startX = p.x;
      startY = p.y;
      const now = posOf(g);
      originX = now.x;
      originY = now.y;
      // A stale delta from the previous drag would be reused by an `up` with no `move`.
      delete g.dataset.dx;
      delete g.dataset.dy;
      g.style.cursor = "grabbing";
      (e.target as Element).setPointerCapture?.(e.pointerId);
      e.preventDefault();
    };

    const move = (e: PointerEvent) => {
      if (!active) return;
      const svg = active.ownerSVGElement;
      if (!svg) return;
      const p = toModel(svg, e.clientX, e.clientY);
      const dx = Math.round((p.x - startX) / SNAP) * SNAP;
      const dy = Math.round((p.y - startY) / SNAP) * SNAP;
      // Against where it was DRAWN, not where it logically is — the two differ after one drag.
      const base = drawnAt(active);
      active.setAttribute(
        "transform",
        `translate(${originX + dx - base.x} ${originY + dy - base.y})`,
      );
      active.dataset.dx = String(dx);
      active.dataset.dy = String(dy);
    };

    const up = () => {
      if (!active) return;
      const dx = Number(active.dataset.dx ?? 0);
      const dy = Number(active.dataset.dy ?? 0);
      const want = { x: originX + dx, y: originY + dy };
      const landed = snapToAllowed(want.x, want.y);
      active.dataset.nx = String(landed.x);
      active.dataset.ny = String(landed.y);
      // Redraw where it actually landed, measured from where it was drawn.
      const base = drawnAt(active);
      active.setAttribute(
        "transform",
        `translate(${landed.x - base.x} ${landed.y - base.y})`,
      );
      if (landed.x !== want.x || landed.y !== want.y) {
        planError = "Walls are hard: snapped to the nearest position that fits.";
      }
      active.style.cursor = "";
      active = null;
      dragging = false;
      if (landed.x !== originX || landed.y !== originY) dirty = true;
    };

    root.addEventListener("pointerdown", down);
    root.addEventListener("pointerover", over);
    root.addEventListener("pointerout", out);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return {
      destroy() {
        root.removeEventListener("pointerdown", down);
        root.removeEventListener("pointerover", over);
        root.removeEventListener("pointerout", out);
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
      },
    };
  }

  let planRoot = $state<HTMLElement | null>(null);
  let picked = $state<string | null>(null);
  let focusRef = $state<string | null>(null);

  /**
   * The focused piece's footprint AS DRAWN, read back out of the plan.
   *
   * Not derived from the catalogue: a 90° turn swaps b and t, and `footprint` is what performs
   * that swap. Recomputing it here would be a second copy of it, and a second copy is what this
   * page exists not to have (PRD B27). The capability writes the answer onto the group.
   */
  let focusPlan = $state<{ w: number; d: number; rot: number } | null>(null);

  // The SVG is replaced wholesale on every re-render, so the highlight and the measured
  // footprint are reapplied rather than kept — there is no element to keep them on.
  $effect(() => {
    if (!planRoot) return;
    const want = picked;
    for (const g of planRoot.querySelectorAll<SVGGElement>("g[data-ref]")) {
      g.classList.toggle("picked", g.dataset.ref === want);
    }
    const g = focusRef
      ? planRoot.querySelector<SVGGElement>(`g[data-ref="${CSS.escape(focusRef)}"]`)
      : null;
    focusPlan = g
      ? {
          w: Number(g.dataset.w ?? 0),
          d: Number(g.dataset.d ?? 0),
          rot: Number(g.dataset.rot ?? 0),
        }
      : null;
  });

  function onKey(e: KeyboardEvent): void {
    const target = e.target as HTMLElement | null;
    if (target && ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName)) return;
    if (e.key === "r" || e.key === "R") {
      e.preventDefault();
      void rotatePicked();
    }
    if (e.key === "Escape") {
      picked = null;
      focusRef = null;
    }
  }

  /**
   * Turn the selected piece by 90°.
   *
   * Only multiples of 90 exist: `footprint` swaps width and depth near 90°, and
   * `Seite::gedreht` refuses anything else outright rather than round it a second way. The new
   * position keeps the piece's CENTRE, because rotating about a corner is not what anyone means
   * by turning a wardrobe — the capability re-renders, so the swapped footprint is its answer
   * and not ours.
   */
  async function rotatePicked(): Promise<void> {
    if (!planRoot || picked === null || selected === null) return;
    const items = itemsFromPlan(planRoot).map((it) => {
      if (it.ref !== picked) return it;
      const g = planRoot!.querySelector<SVGGElement>(`g[data-ref="${CSS.escape(it.ref)}"]`);
      const w = Number(g?.dataset.w ?? 0);
      const d = Number(g?.dataset.d ?? 0);
      const cx = it.x + w / 2;
      const cy = it.y + d / 2;
      const snap = (v: number) => Math.round(v / SNAP) * SNAP;
      // After the turn the footprint is swapped, so the centre implies a new top-left.
      return { ...it, rot: (it.rot + 90) % 360, x: snap(cx - d / 2), y: snap(cy - w / 2) };
    });
    savingPlan = true;
    planError = null;
    try {
      detail = inAsis
        ? await interior.previewPlacements(items)
        : await interior.previewLayout(selected!, items);
      dirty = true;
    } catch (caught) {
      planError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      savingPlan = false;
    }
  }

  async function savePlan(): Promise<void> {
    if (!planRoot || selected === null) return;
    savingPlan = true;
    planError = null;
    const items = itemsFromPlan(planRoot);
    try {
      if (inAsis) {
        // Two calls and not one: a layout's `PUT` answers with the verdict and the new plan,
        // while a placement is a row and answers with a count. Saving where the pieces stand
        // should leave the verdict on screen, so the preview is asked for separately.
        await interior.savePlacements(items);
        asisItems = items;
        detail = await interior.previewPlacements(items);
        dirty = false;
      } else {
        detail = await interior.saveLayout(selected, items);
        dirty = false;
        layouts = await interior.layouts();
      }
    } catch (caught) {
      planError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      savingPlan = false;
    }
  }

  async function savePlanAs(): Promise<void> {
    if (!planRoot || !saveAs.trim()) return;
    savingPlan = true;
    planError = null;
    try {
      const id = saveAs.trim();
      const stamp = new Date().toISOString().slice(0, 10);
      await interior.createLayout({
        id,
        items: itemsFromPlan(planRoot),
        note: `Placed in the dashboard on ${stamp}, starting from ${selected ?? "an empty plan"}.`,
      });
      saveAs = "";
      dirty = false;
      layouts = await interior.layouts();
      await select(id);
    } catch (caught) {
      planError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      savingPlan = false;
    }
  }

  // ─── A new plan ────────────────────────────────────────────────────────────

  let planNew = $state(false);
  let newId = $state("");
  /** Start from the layout on screen, or from an empty room. */
  let newCopy = $state(true);

  /**
   * Why the name is refused, or null.
   *
   * The same shape the ids on disk already have, and a file name that needs no escaping: the
   * capability turns this into `layouts/<id>.toml`. Checked here so the answer arrives while
   * typing, and again in the capability, which is the one that owns the file.
   */
  function nameProblem(v: string): string | null {
    const id = v.trim();
    if (id === "") return null;
    if (!/^[a-z0-9]+(-[a-z0-9]+)*$/.test(id)) {
      return "lowercase letters, digits and single hyphens only";
    }
    if (layouts.some((l) => l.id === id)) return `${id} already exists`;
    return null;
  }
  const newProblem = $derived(nameProblem(newId));

  async function createPlan(): Promise<void> {
    const id = newId.trim();
    if (!id || newProblem) return;
    savingPlan = true;
    planError = null;
    try {
      // `von` copies server-side: the page has no business rebuilding an arrangement it
      // already asked for once. An empty plan sends neither, and the capability writes a
      // header saying the date and that the API made it — not a provenance it does not have.
      await interior.createLayout({ id, from: newCopy && selected ? selected : undefined });
      newId = "";
      planNew = false;
      dirty = false;
      layouts = await interior.layouts();
      await select(id);
    } catch (caught) {
      planError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      savingPlan = false;
    }
  }

  /** Second click confirms. Nothing is destroyed — the file moves to `layouts/archiv/`. */
  let archiveArmed = $state<string | null>(null);

  async function archivePlan(): Promise<void> {
    if (selected === null) return;
    if (archiveArmed !== selected) {
      archiveArmed = selected;
      return;
    }
    savingPlan = true;
    planError = null;
    try {
      const r = await interior.archiveLayout(selected);
      planError = `Moved to ${r.archived}. Nothing was deleted.`;
      archiveArmed = null;
      selected = null;
      detail = null;
      layouts = await interior.layouts();
      if (layouts.length > 0) await select(layouts[0].id);
    } catch (caught) {
      planError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      savingPlan = false;
    }
  }

  /** Record the arrangement as where things actually stand, not as a proposal. */
  async function markAsActual(): Promise<void> {
    if (!planRoot) return;
    savingPlan = true;
    planError = null;
    try {
      const r = await interior.savePlacements(itemsFromPlan(planRoot));
      planError = `Recorded ${r.placed} pieces as actually placed.`;
    } catch (caught) {
      planError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      savingPlan = false;
    }
  }

  // ─── Editing ───────────────────────────────────────────────────────────────
  //
  // Every write goes through the capability. The page holds no rule and computes no verdict:
  // even the impact preview below is the capability re-checking every layout, not the browser
  // guessing. A second implementation of a clearance rule is the drift this whole thing exists
  // to prevent (PRD B27).

  const byId = $derived(new Map(rows.map((r) => [r.item.id, r])));
  const allOwned = $derived(rows.filter((r) => r.state === "owned"));

  async function loadRoomplanReference(): Promise<void> {
    try {
      roomplanReference = await interior.roomplanReference();
    } catch {
      roomplanReference = null;
    }
    try {
      roomplanRevisions = (await interior.roomplanRevisions()).revisions;
    } catch {
      roomplanRevisions = [];
    }
  }

  async function load(): Promise<void> {
    loading = true;
    error = null;
    try {
      const [l, inv, m] = await Promise.all([interior.layouts(), inventory.rows(), interior.model()]);
      layouts = l;
      rows = inv;
      room = m;
      asisItems = (await interior.placements(m.flat.id)).map((p) => ({
        ref: p.item_id,
        x: p.x,
        y: p.y,
        rot: p.rot,
      }));
      if (selected === null && l.length > 0) await select(l[0].id);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      loading = false;
    }
  }

  /**
   * The as-is arrangement: the store's rows, drawn by the capability like any other plan.
   *
   * No `toleranz`, `einbringung` or `sonne` beside it: all three take a layout file, and there is
   * none here. The verdict is not missing though — it comes back with the preview, because
   * "does what is standing there now break my own rules" is a real question and the same one.
   */
  async function selectAsis(): Promise<void> {
    selected = ASIS;
    detail = null;
    detailError = null;
    picked = null;
    focusRef = null;
    dirty = false;
    robust = null;
    sun = null;
    moveIn = [];
    try {
      detail = await interior.previewPlacements(asisItems);
    } catch (caught) {
      detailError = caught instanceof Error ? caught.message : String(caught);
    }
  }

  async function select(id: string): Promise<void> {
    selected = id;
    detail = null;
    detailError = null;
    picked = null;
    focusRef = null;
    archiveArmed = null;
    dirty = false;
    robust = null;
    sun = null;
    sunError = null;
    moveIn = [];
    try {
      detail = await interior.layout(id);
    } catch (caught) {
      detailError = caught instanceof Error ? caught.message : String(caught);
      return;
    }
    // Beside the plan, not before it. Each answers its own question and each may fail on its
    // own — a flat without `[lage]` has no sun report and still has a layout.
    void interior
      .tolerance(id)
      .then((r) => (robust = r))
      .catch(() => (robust = null));
    void interior
      .delivery(id)
      .then((r) => (moveIn = r))
      .catch(() => (moveIn = []));
    void interior
      .sun(id)
      .then((r) => (sun = r))
      .catch((e) => (sunError = e instanceof Error ? e.message : String(e)));
  }

  /** How much error the verdict survives, in one sentence. */
  const holdsUpTo = (r: InteriorRobustness): string => {
    switch (r.holds.kind) {
      case "fails":
        return "fails already at the measurements on file";
      case "nothing_guessed":
        return "no measurement in this layout is marked as a guess";
      case "up_to":
        return `holds up to ${r.holds.cm} cm of measurement error`;
      case "beyond_horizon":
        return `holds past ${r.holds.horizon_cm} cm — further was not searched`;
    }
  };

  const doorLine = (e: InteriorDelivery): string => {
    switch (e.door.kind) {
      case "fits":
        return `through the ${e.door.door_cm} cm door, ${e.door.air_cm} cm to spare`;
      case "does_not_fit":
        return `does NOT fit the ${e.door.door_cm} cm door — ${e.door.missing_cm} cm too wide`;
      case "carried_apart":
        return `${e.door.missing_cm} cm wider than the ${e.door.door_cm} cm door, carried in parts`;
      case "no_entrance":
        return "no opening is declared as the entrance";
    }
  };


  async function start(): Promise<void> {
    starting = true;
    error = null;
    try {
      await sjelStatus.start("interior");
      await capabilities.refresh();
      await load();
      await loadRoomplanReference();
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      starting = false;
    }
  }

  onMount(() => {
    if (isRoomPlanPhone()) {
      // On the phone, layouts and inventory come from the canonical node through the native
      // bridge. Without a node address the page keeps to capture only, as before.
      void getConnectionSettings()
        .then((settings) => settings.canonical_base_url)
        .catch(() => null)
        .then((baseUrl) => {
          if (!baseUrl) {
            phoneOnly = true;
            loading = false;
            return;
          }
          void import("$lib/interior/RoomPlanViewer.svelte").then(({ default: viewer }) => {
            RoomPlanViewer = viewer;
          });
          void load();
          void loadRoomplanReference();
        });
      return;
    }
    capabilities.subscribe();
    void import("$lib/interior/RoomPlanViewer.svelte").then(({ default: viewer }) => {
      RoomPlanViewer = viewer;
    });
    void load();
    void loadRoomplanReference();
  });
</script>

{#snippet walkCell(c: Corridor)}
  {c.from} → {c.to}
{/snippet}

{#snippet widthCell(c: Corridor)}
  {c.width_cm === null ? "no way through" : `${c.width_cm} cm`}
{/snippet}

{#snippet ruleCell(r: ReserveRow)}
  {r.rule}{#if r.item}<span class="of"> · {r.item}</span>{/if}{#if r.subject}<span class="of"> · {r.subject}</span>{/if}
{/snippet}

{#snippet measuredCell(r: ReserveRow)}
  {r.measured}/{r.required} {r.unit}
{/snippet}

{#snippet slackCell(r: ReserveRow)}
  <span class:neg={r.slack < 0}>{r.capped ? "≥ " : ""}{r.slack > 0 ? "+" : ""}{r.slack}</span>
{/snippet}

<svelte:window onkeydown={onKey} />

  <PageHeader
    badge="Interior"
    title="Rooms and what stands in them"
    desc="Layouts judged against the flat's own clearance rules, and an inventory that outlives the flat."
  />

<RoomCapturePanel />

{#if phoneOnly}
  <div class="card offer">
    <p class="lead">Room capture is available offline on this iPhone. Set the Sjel connection (page footer) to load layouts and inventory from the canonical node.</p>
  </div>
{:else if error}
  <div class="card offer">
    <p class="lead"><Icon name="alert" size={15} /> {error}</p>
    <!-- On-demand is the design: nothing but the shell runs until you open something. -->
    <button onclick={start} disabled={starting}>
      {starting ? "Starting…" : "Start interior"}
    </button>
  </div>
{:else if loading}
  <p class="empty">Reading the model…</p>
{:else}
  <!-- On a phone: one sticky line under the header, scrolling sideways if it must. -->
  <nav class="views" aria-label="Interior views">
    <button class:active={view === "plans"} aria-pressed={view === "plans"} onclick={() => (view = "plans")}>
      Plans <span class="count">{layouts.length}</span>
    </button>
    <a href={link("/inventory")}>Inventory <span class="count">{rows.length}</span></a>
    <button
      class:active={view === "solve"}
      aria-pressed={view === "solve"}
      onclick={() => (view = "solve")}
    >
      Solve
    </button>
    <button
      class:active={view === "buy"}
      aria-pressed={view === "buy"}
      onclick={() => (view = "buy")}
    >
      Buy &amp; declare
    </button>
  </nav>
{/if}

{#if !loading && !error && view === "plans"}
  {#if layouts.length === 0}
    <p class="empty">No layout on disk for this flat.</p>
  {:else}
    <div class="plans">
      <!-- One grid column: the list of plans, and the way to add one. -->
      <div class="side">
        <ul class="variants">
          <li>
            <button class:active={inAsis} onclick={() => void selectAsis()}>
              <span class="verdict">as-is</span>
              <span class="name">Where the pieces stand</span>
              <span class="tally mono">
                {asisItems.length} placed · not a proposal
              </span>
            </button>
          </li>
          {#each layouts as l (l.id)}
            <li>
              <button class:active={selected === l.id} onclick={() => select(l.id)}>
                <span class="verdict" class:pass={l.pass}>{l.pass ? "passes" : "fails"}</span>
                <span class="name">{l.name}</span>
                <span class="tally mono">
                  {l.hard} hard · {l.soft} soft
                </span>
              </button>
            </li>
          {/each}
        </ul>

        <!--
          A plan of one's own. The name becomes `layouts/<id>.toml`, so it is checked against
          the shape the ids on disk already have — and against the ones that exist, because the
          capability refuses to overwrite and this says so before the round trip.
        -->
        <div class="newplan">
          <button class="ghost" onclick={() => (planNew = !planNew)}>
            {planNew ? "Cancel" : "+ New plan"}
          </button>
          {#if planNew}
            <input aria-label="Id for the new plan" placeholder="bett-an-der-ostwand" bind:value={newId} />
            <label>
              <input type="checkbox" bind:checked={newCopy} disabled={selected === null} />
              start from {selected ?? "an empty room"}
            </label>
            {#if newProblem}<p class="bad">{newProblem}</p>{/if}
            <button
              disabled={newId.trim() === "" || newProblem !== null || savingPlan}
              onclick={createPlan}
            >
              {savingPlan ? "Creating…" : "Create"}
            </button>
          {/if}
        </div>
      </div>

      <section class="plan">
        {#if detailError}
          <p class="error"><Icon name="alert" size={15} /> {detailError}</p>
        {:else if detail === null}
          <p class="empty">Drawing…</p>
        {:else}
          <!--
            Which plan, and how big the room it is drawn in actually is. The measurements come
            from the capability, which reads them off the same polygon every check rasterises.
          -->
          {#if roomplanReference}
            <section class="scan-review" aria-labelledby="scan-review-heading">
              <div class="scan-review-head">
                <div>
                  <p class="eyebrow">Native RoomPlan · read-only reference</p>
                  <h3 id="scan-review-heading">3D scan beside the 2D apartment model</h3>
                </div>
                <span class="mono">{roomplanReference.asset.byte_length.toLocaleString()} bytes</span>
              </div>
              {#if RoomPlanViewer}
                <RoomPlanViewer assetUrl={roomplanReference.asset.url} />
              {:else}
                <p class="note">Loading the 3D viewer…</p>
              {/if}
              <p class="note">
                The 3D file stays native USDZ. The plan below remains the measured 2D model used
                for clearances; neither view overwrites the other.
              </p>
              <RoomPlanRevisionReview revisions={roomplanRevisions} />
            </section>
          {/if}
          <p class="where">
            <strong>{detail.layout.name}</strong>
            <span class="mono id">{selected}</span>
            {#if room}
              <span class="of">
                {room.flat.name} · {room.dimensions.width} × {room.dimensions.depth} cm ·
                {room.area_m2.toFixed(1)} m²
              </span>
            {/if}
          </p>
          <!-- The capability draws it. eslint-disable-next-line svelte/no-at-html-tags -->
          <!--
            Drag a piece; nothing is written until you save. The verdict below is the
            capability's, recomputed on the arrangement you land on — the page decides nothing.
          -->
          {#key detail.svg}
            <div class="svg" class:dragging bind:this={planRoot} use:draggable>{@html detail.svg}</div>
          {/key}

          <!--
            What the piece under the pointer measures. The footprint is the one the capability
            DREW: a 90° turn swaps b and t, and `footprint` is what swaps them. Everything below
            it is the catalogue row, including the clearances the piece states for itself (Q61)
            — a wardrobe that needs 65 cm in front of its doors says so here, not in a rule.
          -->
          {#if focusRef}
            {@const it = byId.get(focusRef)?.item}
            <p class="piece">
              <span class="mono ref">{focusRef}</span>
              {#if it}<span class="what">{it.label}</span>{/if}
              {#if focusPlan}
                <strong class="mono">{focusPlan.w} × {focusPlan.d} cm</strong>
                <span class="of">
                  in the plan{focusPlan.rot ? `, turned ${focusPlan.rot}°` : ""}
                </span>
              {/if}
              {#if it}
                <span class="of mono">
                  h {it.height ?? "—"}{it.height_min !== null ? ` · h_min ${it.height_min}` : ""} cm
                </span>
                {#if it.width !== null && it.depth !== null}
                  <span class="of mono">catalogue {it.width} × {it.depth}</span>
                {/if}
                {#if it.open_clear !== null}
                  <span class="needs mono">
                    open_clear {it.open_clear} cm{it.opens ? ` ${it.opens}` : ""}
                  </span>
                {/if}
                {#if it.access_sides !== null}
                  <span class="needs mono">
                    access_sides {it.access_sides}{it.access_clear !== null
                      ? ` × ${it.access_clear} cm`
                      : ""}
                  </span>
                {/if}
                {#if it.expands_to !== null}
                  <span class="needs mono">
                    expands {it.expands_dir ?? "?"} to {it.expands_to} cm
                  </span>
                {/if}
                {#if it.room_divider}<span class="needs mono">raumtrenner</span>{/if}
                {#if it.estimated.length > 0}
                  <span class="guess" use:tip={"measured? no."}>~ {it.estimated.join(", ")}</span>
                {/if}
              {:else}
                <span class="of">no inventory row — the plan draws it from its own size</span>
              {/if}
            </p>
          {/if}
          <div class="planbar">
            <button class="ghost" disabled={picked === null || savingPlan} onclick={rotatePicked}>
              {picked === null ? "Rotate (pick a piece)" : `Rotate ${picked} 90°`}
            </button>
            {#if dirty}<span class="moved">moved — not saved</span>{/if}
            <button class="ghost" disabled={!dirty || savingPlan} onclick={savePlan}>
              {savingPlan ? "Saving…" : "Save into this layout"}
            </button>
            <input aria-label="Id to save this layout as" placeholder="save as new id…" bind:value={saveAs} />
            <button class="ghost" disabled={!saveAs.trim() || savingPlan} onclick={savePlanAs}>
              Save as
            </button>
            <button class="ghost" disabled={savingPlan} onclick={markAsActual}>
              This is how it stands
            </button>
            <!-- Not a delete. The file moves to layouts/archiv/ and keeps its reasoning. -->
            <button
              class="ghost"
              class:armed={archiveArmed === selected}
              disabled={savingPlan}
              onclick={archivePlan}
            >
              {archiveArmed === selected ? "Archive — click again" : "Archive"}
            </button>
          </div>
          {#if planError}<p class="note">{planError}</p>{/if}

          <!--
            By how much, not whether. Two layouts that both report `pass` can have 2 cm and
            40 cm of air, and until 2026-08-31 they were the same word in every report.
          -->
          {#if detail.check.tightest_reserve_cm !== null}
            <p class="reserve" class:tight={detail.check.tightest_reserve_cm < 10}>
              <strong class="mono">{detail.check.tightest_reserve_cm} cm</strong>
              {detail.check.tightest_reserve_cm < 0
                ? "short at its tightest hard rule"
                : "is all the room the tightest hard rule has left"}
              {#if robust}<span class="holds">· {holdsUpTo(robust)}</span>{/if}
              {#if robust && robust.tips_at.length > 0}
                <span class="holds">· breaks first at {robust.tips_at.join(", ")}</span>
              {/if}
            </p>
          {/if}

          <dl class="metrics mono">
            <div><dt>room</dt><dd>{detail.check.metrics.room_area_m2.toFixed(2)} m²</dd></div>
            <div><dt>occupied</dt><dd>{detail.check.metrics.occupied_area_m2.toFixed(2)} m²</dd></div>
            <div><dt>free</dt><dd>{detail.check.metrics.free_area_m2.toFixed(2)} m²</dd></div>
          </dl>

          <DataTable
            rows={detail.check.metrics.corridors}
            columns={walkColumns}
            key={(c) => `${c.from} → ${c.to}`}
            caption="Walks, at their narrowest point"
            rowHeader
            empty="No walk between two rooms is declared."
          />

          {#if detail.check.hard.length > 0}
            <ul class="violations hard">
              {#each detail.check.hard as v, i (v.rule + i)}
                <li>
                  <span class="rule mono">{v.rule}</span>
                  {v.message}
                  {#if v.text}<span class="rule-text">{v.text}</span>{/if}
                </li>
              {/each}
            </ul>
          {/if}
          {#if detail.check.soft.length > 0}
            <ul class="violations soft">
              {#each detail.check.soft as v, i (v.rule + i)}
                <li>
                  <span class="rule mono">{v.rule}</span>
                  {v.message}
                  {#if v.text}<span class="rule-text">{v.text}</span>{/if}
                </li>
              {/each}
            </ul>
          {/if}
          <!--
            Declared and not measured. Shown on a passing layout too, and that is the point:
            "passes" means "passes the rules that were checked", and this is the difference
            between the two. It was invisible until 2026-08-31.
          -->
          {#if detail.check.unchecked.length > 0}
            <p class="unchecked">
              <Icon name="alert" size={14} />
              Not measured here:
              {detail.check.unchecked.map((r) => `${r.rule} (${r.reason})`).join(" · ")}
            </p>
          {/if}
          <!--
            Furniture already decided against. A verdict about pieces that are not coming
            answers no question that is still open — and one of the failing layouts fails on
            exactly such a piece.
          -->
          {#if detail.check.outdated.length > 0}
            <p class="unchecked">
              <Icon name="alert" size={14} />
              Places furniture already rejected: {detail.check.outdated.join(", ")} — this
              verdict is about a plan you have moved on from.
            </p>
          {/if}
          {#if detail.check.uncertainties.length > 0}
            <p class="guessed">
              <Icon name="alert" size={14} />
              Every number above inherits a guess:
              {detail.check.uncertainties.map((u) => `${u.label} (${u.fields.join(", ")})`).join("; ")}
            </p>
          {/if}

          <!--
            Every measurement, including the ones that passed. The table is the difference
            between "this layout works" and "this layout works, and here is where it is thin".
          -->
          {#if detail.check.reserves.length > 0}
            <details class="reserves">
              <summary>Every measurement, passed or not ({detail.check.reserves.length})</summary>
              <DataTable
                rows={reserves}
                columns={reserveColumns}
                key={(r) => String(r.rank)}
                rowHeader
                warn={tight}
              />
            </details>
          {/if}

          <!--
            A layout can satisfy every clearance rule and still never happen, because the piece
            does not fit through the front door. Until 2026-08-31 the answer to that was
            "passes" — computed right, wrong question.
          -->
          {#if moveIn.length > 0}
            <details class="reserves" open={moveIn.some((e) => e.door.kind === "does_not_fit" || !e.affordable)}>
              <summary>Getting it in ({moveIn.length})</summary>
              <ul class="movein">
                {#each moveIn as e (e.reference)}
                  <li class:bad={e.door.kind === "does_not_fit" || !e.affordable}>
                    <span class="mono">{e.reference}</span>
                    <span class="dims mono">{e.width}×{e.depth}</span>
                    <span>{doorLine(e)}</span>
                    <span class="way">
                      {e.affordable
                        ? `route free${e.rotates ? ", with a turn" : ""}`
                        : (e.reason ?? "no way to its place")}
                    </span>
                  </li>
                {/each}
              </ul>
            </details>
          {/if}

          <!--
            Four days carry the whole year: the two solstices are the extremes, the equinoxes
            the middle. Every other day lies between them.
          -->
          {#if sun}
            <details class="reserves">
              <summary>Sun over the year</summary>
              <SunGrid {sun} />
              <ul class="sunlist">
                {#each Object.entries(sun.hits_per_piece) as [ref, n] (ref)}
                  <li><span class="mono">{ref}</span> {n} hours in direct sun</li>
                {/each}
              </ul>
              {#if sun.without_glass_heights.length > 0}
                <p class="note">
                  No glazing heights on file for {sun.without_glass_heights.join(", ")} — those
                  windows throw no light here, and they should.
                </p>
              {/if}
            </details>
          {:else if sunError}
            <p class="note">No sun report: {sunError}</p>
          {/if}
        {/if}
      </section>
    </div>
  {/if}
{/if}

{#if !loading && !error && view === "solve"}
  <InteriorSolverView {selected} {detail} {allOwned} />
{/if}

{#if !loading && !error && view === "buy"}
  <InteriorPurchaseView />
{/if}

<style>
  .views {
    display: flex;
    gap: 0.5rem;
    margin: 1rem 0 1.25rem;
  }
  .views button,
  .views a {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    cursor: pointer;
    font-size: var(--text-sm);
    padding: 0.4rem 0.85rem;
    text-decoration: none;
  }
  .views button.active {
    border-color: var(--primary);
    color: var(--text-primary);
  }
  .count {
    color: var(--text-tertiary);
    margin-left: 0.35rem;
  }

  .plans {
    display: grid;
    gap: 1.25rem;
    grid-template-columns: minmax(14rem, 20rem) 1fr;
  }
  @media (max-width: 60rem) {
    .plans {
      grid-template-columns: 1fr;
    }
  }

  .variants {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .variants button {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    cursor: pointer;
    display: grid;
    gap: 0.15rem;
    padding: 0.6rem 0.75rem;
    text-align: left;
    width: 100%;
  }
  .variants button.active {
    border-color: var(--primary);
  }
  .variants .name {
    color: var(--text-primary);
    font-size: 0.875rem;
  }
  .variants .tally {
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }
  .verdict {
    color: var(--danger);
    font-size: var(--text-2xs);
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .verdict.pass {
    color: var(--success);
  }

  .scan-review {
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    margin-bottom: 1rem;
    padding: 0.85rem;
  }
  .scan-review-head {
    align-items: baseline;
    display: flex;
    gap: 1rem;
    justify-content: space-between;
    margin-bottom: 0.7rem;
  }
  .scan-review h3 {
    color: var(--text-primary);
    font-size: var(--text-base);
    margin: 0;
  }
  .scan-review .eyebrow {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    letter-spacing: 0.08em;
    margin: 0 0 0.2rem;
    text-transform: uppercase;
  }
  .scan-review .note {
    color: var(--text-tertiary);
    font-size: var(--text-xs);
    margin: 0.6rem 0 0;
  }

  /* Which plan, in which room. Above the drawing, because that is where the eye starts. */
  .where {
    align-items: baseline;
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin: 0 0 0.5rem;
  }
  .where strong {
    color: var(--text-primary);
    font-size: var(--text-base);
  }
  .where .id {
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  /* The list of plans and the way to add one are one grid column, not two. */
  .side {
    display: flex;
    flex-direction: column;
  }

  .newplan {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    margin-top: 0.6rem;
  }
  .newplan button {
    background: transparent;
    border: 1px dashed var(--card-border);
    border-radius: 6px;
    color: var(--text-secondary);
    cursor: pointer;
    font: inherit;
    font-size: var(--text-xs);
    padding: 0.35rem 0.7rem;
  }
  .newplan button:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  .newplan input:not([type="checkbox"]) {
    background: var(--input-bg);
    border: 1px solid var(--card-border);
    border-radius: 5px;
    color: var(--text-primary);
    font: inherit;
    font-size: var(--text-xs);
    padding: 0.3rem 0.45rem;
    width: 100%;
  }
  .newplan label {
    align-items: center;
    color: var(--text-tertiary);
    display: flex;
    font-size: 0.72rem;
    gap: 0.35rem;
  }
  .newplan .bad {
    color: var(--warning-ink);
    font-size: 0.72rem;
    margin: 0;
  }

  /* The measurements of one piece, while the pointer is on it. */
  .piece {
    align-items: baseline;
    border-top: 1px solid var(--card-border);
    display: flex;
    flex-wrap: wrap;
    font-size: 0.78rem;
    gap: 0.15rem 0.7rem;
    margin: 0.6rem 0 0;
    padding-top: 0.45rem;
  }
  .piece .ref {
    color: var(--primary);
  }
  .piece .what {
    color: var(--text-secondary);
  }
  .piece strong {
    color: var(--text-primary);
  }
  /* What the piece states it needs, in the vocabulary the file uses for it (PRD Q61). */
  .piece .needs {
    color: var(--warning-ink);
    font-size: 0.72rem;
  }

  .planbar button.armed {
    border-color: var(--warning);
    color: var(--warning-ink);
  }

  .plan .svg :global(svg) {
    background: var(--surface);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    height: auto;
    max-width: 100%;
  }

  .metrics {
    display: flex;
    flex-wrap: wrap;
    gap: 1.25rem;
    margin: 0.85rem 0 0;
  }
  .metrics div {
    display: flex;
    gap: 0.4rem;
  }
  .metrics dt {
    color: var(--text-tertiary);
  }
  .metrics dd {
    color: var(--text-primary);
    margin: 0;
  }

  .violations {
    display: grid;
    gap: 0.3rem;
    list-style: none;
    margin: 0.85rem 0 0;
    padding: 0;
  }
  .violations li {
    border-left: 2px solid var(--danger);
    color: var(--text-secondary);
    font-size: var(--text-sm);
    padding-left: 0.6rem;
  }
  .violations.soft li {
    border-left-color: var(--warning);
  }
  .rule {
    color: var(--text-tertiary);
    margin-right: 0.4rem;
  }

  /* The rule's own wording, under the violation it explains. Quiet: the reader came for what
     is wrong, and stays for why it counts as wrong. */
  .rule-text {
    color: var(--text-tertiary);
    display: block;
    font-size: var(--text-xs);
    margin-top: 0.15rem;
  }

  /* ─── Dragging ────────────────────────────────────────────────────────── */

  .plan .svg :global(g[data-ref]) {
    cursor: grab;
  }

  /* No text selection while a piece is being moved. */
  .plan .svg.dragging {
    user-select: none;
  }

  .plan .svg :global(g[data-ref]:hover rect) {
    stroke: var(--primary);
  }

  .plan .svg :global(g[data-ref].picked rect) {
    stroke: var(--primary);
    stroke-width: 6;
  }

  .planbar {
    align-items: center;
    display: flex;
    flex-wrap: wrap;
    gap: 0.45rem;
    margin-top: 0.7rem;
  }

  .planbar button {
    background: transparent;
    border: 1px solid var(--card-border);
    border-radius: 6px;
    color: var(--text-secondary);
    cursor: pointer;
    font: inherit;
    font-size: var(--text-xs);
    padding: 0.3rem 0.7rem;
  }

  .planbar button:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }

  .planbar input {
    background: var(--input-bg);
    border: 1px solid var(--card-border);
    border-radius: 5px;
    color: var(--text-primary);
    font: inherit;
    font-size: var(--text-xs);
    padding: 0.3rem 0.45rem;
    width: 9rem;
  }

  .planbar .moved {
    color: var(--warning-ink);
    font-size: var(--text-xs);
    margin-right: 0.2rem;
  }

  /* ─── Editing ─────────────────────────────────────────────────────────── */


















  /* The arithmetic that used to have to be done by hand, one direction at a time. */












  .guessed,
  .unchecked {
    align-items: center;
    color: var(--warning-ink);
    display: flex;
    font-size: var(--text-xs);
    gap: 0.35rem;
    margin: 0.85rem 0 0;
  }



  /* A column, not a grid: a card in a tall row keeps its lines together and puts its
     actions at the bottom, where a grid spread the spare height between every line and
     stretched the buttons (measured at 1280px, 2026-09-25). */
  .dims {
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }
  /* Size, price and the guess marker on one line. A part that is not known is left out. */
  .guess {
    color: var(--warning-ink);
    font-size: var(--text-xs);
  }

  .card {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
  }
  .offer {
    padding: 1rem 1.15rem;
  }
  .offer .lead {
    align-items: center;
    color: var(--danger);
    display: flex;
    gap: 0.4rem;
    margin: 0 0 0.75rem;
  }
  .offer button {
    background: var(--primary);
    border: none;
    border-radius: var(--radius-sm);
    color: white;
    cursor: pointer;
    font-size: var(--text-sm);
    padding: 0.45rem 0.9rem;
  }
  .empty,
  .error {
    color: var(--text-tertiary);
    font-size: var(--text-sm);
  }
  .error {
    align-items: center;
    color: var(--danger);
    display: flex;
    gap: 0.4rem;
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* By how much, not whether. */
  .reserve {
    color: var(--text-secondary);
    font-size: var(--text-sm);
    margin: 0.85rem 0 0;
  }
  .reserve.tight {
    color: var(--warning-ink);
  }
  .reserve strong {
    color: var(--text-primary);
  }
  .holds,
  .of {
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  .reserves {
    margin-top: 0.85rem;
  }
  .reserves summary {
    color: var(--text-tertiary);
    cursor: pointer;
    font-size: var(--text-xs);
  }
  .neg {
    color: var(--danger);
  }

  .movein {
    list-style: none;
    margin: 0.5rem 0 0;
    padding: 0;
  }
  .movein li {
    border-top: 1px solid var(--card-border);
    display: grid;
    font-size: var(--text-xs);
    gap: 0.5rem;
    grid-template-columns: 10rem 5rem 1fr 1fr;
    padding: 0.35rem 0;
  }
  .movein li.bad {
    color: var(--danger);
  }
  .movein .dims,
  .movein .way {
    color: var(--text-tertiary);
  }

  .sunlist {
    color: var(--text-secondary);
    font-size: var(--text-xs);
    list-style: none;
    margin: 0.5rem 0 0;
    padding: 0;
  }

  /* ─── The editor as a bottom sheet (rendered only below the phone breakpoint) ─── */

  /* Above the header (50) and the bottom tab bar (45): a modal covers the shell. */
  /* The one part that scrolls; `position` makes it the offset parent `showInSheet` measures from. */

  /* ─── Phone width (the 38rem breakpoint `app.css` names) ─────────────────── */

  @media (width < 38rem) {
    /* One segmented line, pinned under the header. The header is its bar plus the top
       safe area plus a 1px rule; the band runs edge to edge over main's padding. */
    /* Opaque, not glass: labels over moving text were unreadable wherever the blur was
       missing, and a strip this thin gains nothing from it. */
    .views {
      background-color: var(--page-bg);
      border-bottom: 1px solid var(--header-border);
      gap: 0;
      margin: var(--space-4) calc(-1 * var(--space-5)) var(--space-4);
      overflow-x: auto;
      padding: var(--space-2) var(--space-4);
      position: sticky;
      scrollbar-width: none;
      top: calc(var(--header-h) + env(safe-area-inset-top, 0px) + 1px);
      z-index: 30;
    }
    .views::-webkit-scrollbar {
      display: none;
    }
    .views button,
    .views a {
      align-items: center;
      border-color: transparent;
      border-radius: var(--radius-md);
      display: inline-flex;
      flex: 0 0 auto;
      gap: var(--space-1);
      min-height: 2.5rem;
      padding: 0 var(--space-3);
      white-space: nowrap;
    }
    .views button:not(.active),
    .views a {
      background: transparent;
    }
    .views button.active {
      box-shadow: var(--card-shadow);
    }
    .views .count {
      background: var(--surface);
      border-radius: var(--radius-sm);
      font-size: var(--text-2xs);
      font-variant-numeric: tabular-nums;
      line-height: var(--leading-normal);
      margin: 0;
      padding: 0 var(--space-1);
    }

    /* Denser cards: a photo becomes a thumbnail beside the text, the actions a short row. */
    /* Every number and sentence stays; only the air between them goes. */
    /* A warning that wraps keeps its icon on the first line. */
    .unchecked,
    .guessed {
      align-items: flex-start;
    }

    /* Fields in rows that match what they measure: the three dimensions together, price
       beside priority. 16px or more, or iOS zooms the page on focus; 44px tall to tap. */
  }
</style>
