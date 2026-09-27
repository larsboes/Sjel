<script module lang="ts">
  /**
   * A force-directed map, drawn as SVG, with no graph library behind it.
   *
   * The knowledge-graph panel this replaces pulled vis-network from a CDN and
   * handed it all 6665 graphify nodes at once with `improvedLayout` on. That
   * layout never converged, so the page rendered a black rectangle -- and the
   * CDN made a local-first surface depend on unpkg being reachable. Both go
   * away here: the layout is ~80 lines of Fruchterman-Reingold, and every
   * caller passes a bounded set (31 units, or one unit's capped file list).
   *
   * Deterministic on purpose. The seed layout is a circle indexed by position
   * in the node list, never `Math.random`, so the same graph draws the same
   * picture twice and a screenshot means something.
   */
  export interface MapNode {
    id: string;
    label: string;
    /** Drives colour. Free-form so both the unit view and the file view can use it. */
    kind: string;
    /** 0..1, drives radius. Relative within the set the caller passes. */
    weight?: number;
    /** Live state, drawn as a ring. `null` means nothing declares a check. */
    state?: "up" | "down" | null;
  }

  export interface MapEdge {
    from: string;
    to: string;
    label?: string;
  }
</script>

<script lang="ts">
  import { onDestroy } from "svelte";

  let {
    nodes = [],
    edges = [],
    selected = null,
    height = 420,
    onSelect,
  }: {
    nodes: MapNode[];
    edges: MapEdge[];
    selected?: string | null;
    height?: number;
    onSelect?: (id: string | null) => void;
  } = $props();

  interface Placed {
    id: string;
    x: number;
    y: number;
    dx: number;
    dy: number;
  }

  /**
   * The layout runs in a coordinate space whose aspect ratio matches the
   * element, so the viewBox never letterboxes. A fixed 1000x620 space inside a
   * wide container left ~48% of the width as empty gutter and squeezed the
   * graph into the middle third of the page.
   */
  const HEIGHT = 620;
  let WIDTH = $state(1000);

  let placed = $state<Placed[]>([]);
  let hovered = $state<string | null>(null);
  let view = $state({ x: 0, y: 0, scale: 1 });
  let svg: SVGSVGElement;
  let frame = 0;
  /**
   * `moved` is what separates a click from a drag.
   *
   * Selection cannot hang off `onclick`: the pan/drag handler puts pointer
   * capture on the `<svg>`, and a captured pointer retargets the click to the
   * capturing element, so a node's own click handler never ran. Deciding on
   * pointerup by how far the pointer travelled removes the interaction
   * entirely, and stops a drag that ends on a node from selecting it.
   */
  let dragging: { id: string | null; lastX: number; lastY: number; moved: number } | null = null;
  const CLICK_SLOP = 4;

  const reducedMotion =
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

  /** Adjacency, for the hover/selection dimming and for the spring pass. */
  const neighbours = $derived.by(() => {
    const map = new Map<string, Set<string>>();
    for (const n of nodes) map.set(n.id, new Set());
    for (const e of edges) {
      map.get(e.from)?.add(e.to);
      map.get(e.to)?.add(e.from);
    }
    return map;
  });

  const focus = $derived(hovered ?? selected);

  /** A node is lit when nothing is focused, or when it touches what is. */
  function lit(id: string): boolean {
    if (!focus) return true;
    return id === focus || (neighbours.get(focus)?.has(id) ?? false);
  }

  function edgeLit(e: MapEdge): boolean {
    if (!focus) return true;
    return e.from === focus || e.to === focus;
  }

  /**
   * Past this many nodes, every label drawn at once is a grey smear and the
   * picture stops carrying its own shape. Above it the map shows structure and
   * names only what the pointer is on -- which is the question a dense graph
   * actually gets asked ("what is that one?").
   */
  const LABEL_DENSITY_LIMIT = 80;
  const dense = $derived(nodes.length > LABEL_DENSITY_LIMIT);

  function radius(n: MapNode): number {
    const base = 5 + Math.sqrt(Math.max(0, Math.min(1, n.weight ?? 0.35))) * 11;
    return dense ? base * 0.62 : base;
  }

  function labelled(id: string): boolean {
    return !dense || hovered === id || selected === id;
  }

  // ── Layout ────────────────────────────────────────────────────────────────

  /**
   * Fruchterman-Reingold with a weak pull to centre.
   *
   * One tick is O(n²) in the repulsion pass, which is why the callers are
   * bounded rather than why the algorithm is clever: at 400 nodes that is 80k
   * pairs, and the rAF loop below spends a few ticks per frame so the settling
   * is visible instead of blocking.
   */
  function tick(state: Placed[], temperature: number): void {
    const area = WIDTH * HEIGHT;
    const k = Math.sqrt(area / Math.max(1, state.length));

    for (const p of state) {
      p.dx = 0;
      p.dy = 0;
    }

    // Repulsion is local, not global. Summed over every pair it grows with n,
    // so at 400 nodes it overwhelmed the centre pull and parked most of the
    // graph against the clamp in straight lines along the walls. A cutoff is
    // what Barnes-Hut approximates anyway, and it makes the pass cheaper.
    const cutoff = k * 2.5;

    for (let i = 0; i < state.length; i++) {
      for (let j = i + 1; j < state.length; j++) {
        const a = state[i];
        const b = state[j];
        let ox = a.x - b.x;
        let oy = a.y - b.y;
        let dist = Math.hypot(ox, oy);
        if (dist > cutoff) continue;
        if (dist < 0.01) {
          // Two nodes exactly on top of each other have no direction to
          // separate along. Nudge by index so the choice stays deterministic.
          ox = ((i % 7) - 3) / 10 || 0.1;
          oy = ((j % 7) - 3) / 10 || 0.1;
          dist = Math.hypot(ox, oy);
        }
        const force = (k * k) / dist;
        const fx = (ox / dist) * force;
        const fy = (oy / dist) * force;
        a.dx += fx;
        a.dy += fy;
        b.dx -= fx;
        b.dy -= fy;
      }
    }

    const index = new Map(state.map((p) => [p.id, p]));
    for (const e of edges) {
      const a = index.get(e.from);
      const b = index.get(e.to);
      if (!a || !b) continue;
      const ox = a.x - b.x;
      const oy = a.y - b.y;
      const dist = Math.max(0.01, Math.hypot(ox, oy));
      const force = (dist * dist) / k;
      const fx = (ox / dist) * force;
      const fy = (oy / dist) * force;
      a.dx -= fx;
      a.dy -= fy;
      b.dx += fx;
      b.dy += fy;
    }

    for (const p of state) {
      // Centre pull, so a disconnected node drifts back into frame instead of
      // being pushed to infinity by repulsion it never balances.
      p.dx += (WIDTH / 2 - p.x) * 0.03;
      p.dy += (HEIGHT / 2 - p.y) * 0.03;

      const move = Math.max(0.01, Math.hypot(p.dx, p.dy));
      const step = Math.min(move, temperature);
      p.x = clamp(p.x + (p.dx / move) * step, 20, WIDTH - 20);
      p.y = clamp(p.y + (p.dy / move) * step, 20, HEIGHT - 20);
    }
  }

  function clamp(v: number, lo: number, hi: number): number {
    return v < lo ? lo : v > hi ? hi : v;
  }

  function seed(): Placed[] {
    const radiusSeed = Math.min(WIDTH, HEIGHT) * 0.36;
    return nodes.map((n, i) => {
      const angle = (i / Math.max(1, nodes.length)) * Math.PI * 2;
      return {
        id: n.id,
        x: WIDTH / 2 + Math.cos(angle) * radiusSeed,
        y: HEIGHT / 2 + Math.sin(angle) * radiusSeed,
        dx: 0,
        dy: 0,
      };
    });
  }

  function relayout(): void {
    cancelAnimationFrame(frame);
    const state = seed();
    const total = nodes.length > 120 ? 220 : 320;
    let done = 0;

    if (reducedMotion) {
      for (let i = 0; i < total; i++) tick(state, WIDTH / 10 / (1 + i * 0.06));
      placed = state;
      return;
    }

    // A handful of ticks per frame: enough that the shape resolves in under a
    // second, few enough that the tab stays responsive while it does.
    const perFrame = nodes.length > 120 ? 2 : 6;
    const step = () => {
      for (let i = 0; i < perFrame && done < total; i++, done++) {
        tick(state, WIDTH / 10 / (1 + done * 0.06));
      }
      placed = [...state];
      if (done < total) frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
  }

  let lastKey = "";
  $effect(() => {
    // Re-layout only when the graph or the shape of the box changes --
    // selection and hover must never restart the simulation.
    const key = `${nodes.map((n) => n.id).join("|")}#${edges.length}#${Math.round(WIDTH)}`;
    if (key === lastKey) return;
    lastKey = key;
    view = { x: 0, y: 0, scale: 1 };
    relayout();
  });

  /**
   * Track the element's aspect ratio, not its pixel size: the layout is
   * resolution-independent, so only a real shape change is worth re-running
   * the simulation for. The 4% band keeps a drag-resize from thrashing it.
   */
  let observer: ResizeObserver | null = null;
  $effect(() => {
    if (!svg) return;
    observer?.disconnect();
    observer = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      if (width <= 0 || height <= 0) return;
      const next = Math.round(HEIGHT * (width / height));
      if (Math.abs(next - WIDTH) / WIDTH > 0.04) WIDTH = next;
    });
    observer.observe(svg);
  });

  onDestroy(() => {
    cancelAnimationFrame(frame);
    observer?.disconnect();
  });

  const at = $derived(new Map(placed.map((p) => [p.id, p])));

  /**
   * Map the settled layout into the frame, instead of asking the forces to
   * land there on their own.
   *
   * Repulsion pushes an uncoupled unit outward until it hits the clamp, so a
   * pure force layout parks every isolated node on the border with its label
   * half outside the box. Fitting afterwards is a render transform: the
   * simulation keeps its own coordinates, and nothing is ever clipped.
   */
  const fit = $derived.by(() => {
    if (!placed.length) return { scale: 1, tx: 0, ty: 0 };
    const xs = placed.map((p) => p.x);
    const ys = placed.map((p) => p.y);
    const minX = Math.min(...xs);
    const maxX = Math.max(...xs);
    const minY = Math.min(...ys);
    const maxY = Math.max(...ys);
    // Asymmetric padding: a label sits below its dot and overhangs it on both
    // sides, so the bottom and the sides need more room than the top.
    const padX = 70;
    const padTop = 26;
    const padBottom = 40;
    const scale = Math.min(
      (WIDTH - padX * 2) / Math.max(1, maxX - minX),
      (HEIGHT - padTop - padBottom) / Math.max(1, maxY - minY),
      1.8,
    );
    return {
      scale,
      tx: padX + (WIDTH - padX * 2 - (maxX - minX) * scale) / 2 - minX * scale,
      ty: padTop + (HEIGHT - padTop - padBottom - (maxY - minY) * scale) / 2 - minY * scale,
    };
  });

  // ── Interaction ───────────────────────────────────────────────────────────

  function toLocal(event: PointerEvent): { x: number; y: number } {
    const box = svg.getBoundingClientRect();
    const scale = WIDTH / box.width;
    return { x: (event.clientX - box.left) * scale, y: (event.clientY - box.top) * scale };
  }

  function onPointerDown(event: PointerEvent, id: string | null): void {
    const local = toLocal(event);
    dragging = { id, lastX: local.x, lastY: local.y, moved: 0 };
    svg.setPointerCapture(event.pointerId);
  }

  function onPointerMove(event: PointerEvent): void {
    if (!dragging) return;
    const local = toLocal(event);
    const dx = local.x - dragging.lastX;
    const dy = local.y - dragging.lastY;
    dragging.lastX = local.x;
    dragging.lastY = local.y;
    dragging.moved += Math.hypot(dx, dy);

    if (dragging.id) {
      const node = at.get(dragging.id);
      if (node) {
        // Screen delta back into simulation coordinates: through the user's
        // zoom, then through the fit transform the render applies on top.
        node.x += dx / view.scale / fit.scale;
        node.y += dy / view.scale / fit.scale;
        placed = [...placed];
      }
    } else {
      view = { ...view, x: view.x + dx, y: view.y + dy };
    }
  }

  function onPointerUp(event: PointerEvent): void {
    if (dragging?.id && dragging.moved < CLICK_SLOP) pick(dragging.id);
    dragging = null;
    svg.releasePointerCapture?.(event.pointerId);
  }

  function onWheel(event: WheelEvent): void {
    event.preventDefault();
    const next = clamp(view.scale * (event.deltaY < 0 ? 1.12 : 0.89), 0.4, 4);
    view = { ...view, scale: next };
  }

  function pick(id: string): void {
    onSelect?.(selected === id ? null : id);
  }
</script>

<div class="map" style="--map-height: {height}px">
  <svg
    bind:this={svg}
    viewBox="0 0 {WIDTH} {HEIGHT}"
    role="application"
    aria-label="Map of Sjel units and how they connect"
    onpointerdown={(e) => onPointerDown(e, null)}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}
    onwheel={onWheel}
  >
    <g
      transform="translate({view.x} {view.y}) scale({view.scale}) translate({fit.tx} {fit.ty}) scale({fit.scale})"
    >
      {#each edges as e (e.from + "->" + e.to)}
        {@const a = at.get(e.from)}
        {@const b = at.get(e.to)}
        {#if a && b}
          <line
            x1={a.x}
            y1={a.y}
            x2={b.x}
            y2={b.y}
            class="edge"
            class:faded={!edgeLit(e)}
          />
        {/if}
      {/each}

      {#each nodes as n (n.id)}
        {@const p = at.get(n.id)}
        {#if p}
          {@const r = radius(n)}
          <g
            class="node kind-{n.kind}"
            class:faded={!lit(n.id)}
            class:sel={selected === n.id}
            transform="translate({p.x} {p.y})"
            role="button"
            tabindex="0"
            aria-label="{n.label} ({n.kind})"
            aria-pressed={selected === n.id}
            onpointerdown={(e) => {
              e.stopPropagation();
              onPointerDown(e, n.id);
            }}
            onkeydown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                pick(n.id);
              }
            }}
            onmouseenter={() => (hovered = n.id)}
            onmouseleave={() => (hovered = null)}
            onfocus={() => (hovered = n.id)}
            onblur={() => (hovered = null)}
          >
            <!--
              A dense map draws 4px dots, which is not a click target. This
              invisible disc is the one the pointer actually hits, so hover and
              select work at the size the picture needs rather than the size a
              finger needs.
            -->
            <circle class="hit" r={Math.max(r + 6, 11)} />
            {#if n.state === "up" || n.state === "down"}
              <circle class="ring {n.state}" r={r + 3.5} />
            {/if}
            <circle class="dot" {r} />
            {#if labelled(n.id)}
              <text class:pinned={dense} y={r + 12} text-anchor="middle">{n.label}</text>
            {/if}
          </g>
        {/if}
      {/each}
    </g>
  </svg>

  <button class="reset" onclick={() => (view = { x: 0, y: 0, scale: 1 })}>Reset view</button>
</div>

<style>
  .map {
    position: relative;
    height: var(--map-height);
    border: 1px solid var(--card-border);
    border-radius: 10px;
    background: var(--surface);
    overflow: hidden;
  }
  svg {
    width: 100%;
    height: 100%;
    display: block;
    touch-action: none;
    cursor: grab;
  }
  svg:active {
    cursor: grabbing;
  }

  .edge {
    stroke: var(--card-border-hover);
    stroke-width: 1.1;
    transition: opacity 0.15s ease;
  }
  .edge.faded {
    opacity: 0.15;
  }

  .node {
    cursor: pointer;
    transition: opacity 0.15s ease;
  }
  .node.faded {
    opacity: 0.18;
  }
  .node:focus {
    outline: none;
  }
  .node:focus-visible .dot {
    stroke: var(--text-primary);
    stroke-width: 3;
  }

  .hit {
    fill: transparent;
  }

  .dot {
    fill: var(--node-fill, var(--text-tertiary));
    stroke: var(--card-bg);
    stroke-width: 1.5;
  }
  .node.sel .dot {
    stroke: var(--text-primary);
    stroke-width: 2.5;
  }

  /* Four kinds, four hues -- the legend on the page names them. Set as a
     custom property so a nested rule never has to repeat the fill. */
  .kind-capability {
    --node-fill: #0e7490;
  }
  .kind-lib {
    --node-fill: #7c3aed;
  }
  .kind-spine {
    --node-fill: #b45309;
  }
  .kind-pack {
    --node-fill: #15803d;
  }
  .kind-file {
    --node-fill: #0e7490;
  }
  .kind-doc {
    --node-fill: #15803d;
  }
  .kind-config {
    --node-fill: #b45309;
  }
  :global(:root.dark) .kind-capability {
    --node-fill: #22d3ee;
  }
  :global(:root.dark) .kind-lib {
    --node-fill: #a78bfa;
  }
  :global(:root.dark) .kind-spine {
    --node-fill: #fbbf24;
  }
  :global(:root.dark) .kind-pack {
    --node-fill: #4ade80;
  }
  :global(:root.dark) .kind-file {
    --node-fill: #22d3ee;
  }
  :global(:root.dark) .kind-doc {
    --node-fill: #4ade80;
  }
  :global(:root.dark) .kind-config {
    --node-fill: #fbbf24;
  }

  .ring {
    fill: none;
    stroke-width: 2;
  }
  .ring.up {
    stroke: var(--success);
  }
  .ring.down {
    stroke: var(--warning);
  }

  text {
    fill: var(--text-secondary);
    font-size: 11px;
    pointer-events: none;
    user-select: none;
  }
  .node.sel text {
    fill: var(--text-primary);
    font-weight: 600;
  }
  /* The one label a dense map shows has no neighbours to be read against, so
     it carries its own contrast rather than borrowing the secondary tone. */
  text.pinned {
    fill: var(--text-primary);
    font-weight: 600;
    paint-order: stroke;
    stroke: var(--surface);
    stroke-width: 3px;
    stroke-linejoin: round;
  }

  .reset {
    position: absolute;
    right: 0.6rem;
    top: 0.6rem;
    font: inherit;
    font-size: 0.72rem;
    padding: 0.25rem 0.55rem;
    border: 1px solid var(--card-border);
    border-radius: 6px;
    background: var(--card-bg);
    color: var(--text-secondary);
    cursor: pointer;
  }
  .reset:hover {
    color: var(--text-primary);
    border-color: var(--card-border-hover);
  }

  @media (prefers-reduced-motion: reduce) {
    .edge,
    .node {
      transition: none;
    }
  }
</style>
