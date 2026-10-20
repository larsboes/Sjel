<script lang="ts">
  import type { ProjectRecord } from "$lib/api";
  import { buildProjectGraph, type ProjectGraphNode } from "./database";

  let { records }: { records: ProjectRecord[] } = $props();

  let showHierarchy = $state(true);
  let showTasks = $state(true);
  let showNotes = $state(true);
  let showEntities = $state(true);
  let search = $state("");
  let activeId = $state<string | null>(null);

  const rawGraph = $derived(buildProjectGraph(records));

  const filteredGraph = $derived.by(() => {
    const q = search.trim().toLowerCase();
    const allowedKinds = new Set<ProjectGraphNode["kind"]>(["project"]);
    if (showTasks) allowedKinds.add("task");
    if (showNotes) allowedKinds.add("note");
    if (showEntities) allowedKinds.add("entity");

    const nodes = rawGraph.nodes.filter((n) => {
      if (!allowedKinds.has(n.kind)) return false;
      if (q && !n.label.toLowerCase().includes(q) && !n.id.toLowerCase().includes(q)) return false;
      return true;
    });
    const nodeIds = new Set(nodes.map((n) => n.id));
    const edges = rawGraph.edges.filter((e) => {
      if (!showHierarchy && e.via === "hierarchy") return false;
      return nodeIds.has(e.from) && nodeIds.has(e.to);
    });
    return { nodes, edges };
  });

  const positioned = $derived.by(() => {
    const { nodes, edges } = filteredGraph;
    const width = 760;
    const height = 420;
    const cx = width / 2;
    const cy = height / 2;

    const coords = new Map<string, { x: number; y: number }>();
    const hubs = nodes.filter((n) => n.kind === "project");
    const others = nodes.filter((n) => n.kind !== "project");

    hubs.forEach((h, i) => {
      const angle = (2 * Math.PI * i) / Math.max(1, hubs.length);
      const r = hubs.length === 1 ? 0 : 90;
      coords.set(h.id, {
        x: cx + r * Math.cos(angle),
        y: cy + r * Math.sin(angle),
      });
    });

    others.forEach((n, i) => {
      const angle = (2 * Math.PI * i) / Math.max(1, others.length);
      const ring = n.kind === "note" ? 145 : n.kind === "task" ? 175 : 195;
      coords.set(n.id, {
        x: cx + ring * Math.cos(angle),
        y: (cy + ring * Math.sin(angle) * 0.82),
      });
    });

    // Deterministic spring relaxation (12 iterations)
    for (let iter = 0; iter < 12; iter++) {
      for (let i = 0; i < nodes.length; i++) {
        for (let j = i + 1; j < nodes.length; j++) {
          const a = coords.get(nodes[i].id)!;
          const b = coords.get(nodes[j].id)!;
          const dx = b.x - a.x;
          const dy = b.y - a.y;
          const distSq = Math.max(25, dx * dx + dy * dy);
          if (distSq < 3600) {
            const push = (60 - Math.sqrt(distSq)) * 0.12;
            const ux = dx / Math.sqrt(distSq);
            const uy = dy / Math.sqrt(distSq);
            if (nodes[i].kind !== "project" || hubs.length > 1) {
              a.x -= ux * push;
              a.y -= uy * push;
            }
            if (nodes[j].kind !== "project" || hubs.length > 1) {
              b.x += ux * push;
              b.y += uy * push;
            }
          }
        }
      }
      for (const e of edges) {
        const a = coords.get(e.from);
        const b = coords.get(e.to);
        if (!a || !b) continue;
        const dx = b.x - a.x;
        const dy = b.y - a.y;
        const dist = Math.max(1, Math.hypot(dx, dy));
        const target = e.via === "wikilink" ? 110 : 145;
        const pull = (dist - target) * 0.04;
        const ux = dx / dist;
        const uy = dy / dist;
        a.x += ux * pull;
        a.y += uy * pull;
        b.x -= ux * pull;
        b.y -= uy * pull;
      }
      for (const n of nodes) {
        const p = coords.get(n.id)!;
        p.x = Math.max(48, Math.min(width - 48, p.x));
        p.y = Math.max(32, Math.min(height - 32, p.y));
      }
    }

    return {
      width,
      height,
      nodes: nodes.map((n) => ({ ...n, ...(coords.get(n.id) ?? { x: cx, y: cy }) })),
      edges: edges
        .map((e) => {
          const from = coords.get(e.from);
          const to = coords.get(e.to);
          return from && to ? { ...e, x1: from.x, y1: from.y, x2: to.x, y2: to.y } : null;
        })
        .filter((e): e is NonNullable<typeof e> => e !== null),
    };
  });

  const activeNode = $derived(positioned.nodes.find((n) => n.id === activeId) ?? null);
  const connectedIds = $derived.by(() => {
    if (!activeId) return null;
    const set = new Set<string>([activeId]);
    for (const e of positioned.edges) {
      if (e.from === activeId) set.add(e.to);
      if (e.to === activeId) set.add(e.from);
    }
    return set;
  });
</script>

<div class="graph-shell">
  <div class="graph-toolbar">
    <div class="graph-filters">
      <input
        type="search"
        class="graph-search"
        aria-label="Filter graph nodes"
        placeholder="Filter nodes…"
        bind:value={search}
      />
      <label class="filter-chip">
        <input type="checkbox" bind:checked={showNotes} />
        <span>Notes</span>
      </label>
      <label class="filter-chip">
        <input type="checkbox" bind:checked={showTasks} />
        <span>Tasks</span>
      </label>
      <label class="filter-chip">
        <input type="checkbox" bind:checked={showEntities} />
        <span>Typed IDs</span>
      </label>
      <label class="filter-chip">
        <input type="checkbox" bind:checked={showHierarchy} />
        <span>Hub edges</span>
      </label>
    </div>
    <span class="graph-counts">
      {positioned.nodes.length} node{positioned.nodes.length === 1 ? "" : "s"} · {positioned.edges.length} link{positioned.edges.length === 1 ? "" : "s"}
    </span>
  </div>

  <div class="graph-canvas-wrap">
    <svg
      viewBox={`0 0 ${positioned.width} ${positioned.height}`}
      class="graph-svg"
      role="img"
      aria-label="Project knowledge and link graph"
    >
      {#each positioned.edges as e (`${e.from}->${e.to}:${e.via}`)}
        {@const dimmed = connectedIds !== null && !(connectedIds.has(e.from) && connectedIds.has(e.to))}
        <line
          x1={e.x1}
          y1={e.y1}
          x2={e.x2}
          y2={e.y2}
          class="edge edge-{e.via}"
          class:dimmed
        />
      {/each}

      {#each positioned.nodes as n (n.id)}
        {@const dimmed = connectedIds !== null && !connectedIds.has(n.id)}
        {@const selected = activeId === n.id}
        <g
          class="node node-{n.kind}"
          class:dimmed
          class:selected
          transform={`translate(${n.x}, ${n.y})`}
          role="button"
          tabindex="0"
          aria-label={`${n.kind}: ${n.label}`}
          onclick={() => (activeId = activeId === n.id ? null : n.id)}
          onkeydown={(ev) => {
            if (ev.key === "Enter" || ev.key === " ") {
              ev.preventDefault();
              activeId = activeId === n.id ? null : n.id;
            }
          }}
        >
          <circle r={n.kind === "project" ? 11 : n.kind === "note" ? 8 : 6.5} />
          <text dy={n.kind === "project" ? 22 : 18} text-anchor="middle">
            {n.label.length > 26 ? `${n.label.slice(0, 24)}…` : n.label}
          </text>
        </g>
      {/each}
    </svg>
  </div>

  {#if activeNode}
    <div class="graph-inspector">
      <div class="inspector-meta">
        <span class="inspector-kind kind-{activeNode.kind}">{activeNode.kind}</span>
        <strong>{activeNode.label}</strong>
        <span class="inspector-deg">{activeNode.degree} link{activeNode.degree === 1 ? "" : "s"}</span>
      </div>
      <div class="inspector-actions">
        <a class="btn btn-sm" href={activeNode.href}>Open {activeNode.kind} ↗</a>
        <button type="button" class="btn btn-soft btn-sm" onclick={() => (activeId = null)}>Clear</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .graph-shell {
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
    background: var(--card-bg, #ffffff);
    overflow: hidden;
    margin-top: 0.5rem;
  }

  .graph-toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.6rem;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--rule);
    background: var(--surface);
  }

  .graph-filters {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.65rem;
  }

  .graph-search {
    font-size: var(--text-xs);
    padding: 0.25rem 0.5rem;
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
    background: var(--card-bg, #fff);
  }

  .filter-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    font-size: var(--text-xs);
    color: var(--text-secondary);
    cursor: pointer;
  }

  .graph-counts {
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .graph-canvas-wrap {
    width: 100%;
    overflow-x: auto;
  }

  .graph-svg {
    display: block;
    width: 100%;
    height: auto;
    min-height: 22rem;
    max-height: 28rem;
  }

  .edge {
    stroke: var(--rule, #cbd5e1);
    stroke-width: 1.4;
    transition: opacity var(--motion-fast) ease;
  }

  .edge-wikilink {
    stroke: var(--primary, #2563eb);
    stroke-opacity: 0.65;
    stroke-width: 1.75;
  }

  .edge-typed-id {
    stroke: #059669;
    stroke-dasharray: 4 3;
    stroke-width: 1.6;
  }

  .edge-hierarchy {
    stroke-dasharray: 2 3;
    stroke-opacity: 0.55;
  }

  .edge.dimmed,
  .node.dimmed {
    opacity: 0.22;
  }

  .node {
    cursor: pointer;
    transition: opacity var(--motion-fast) ease;
  }

  .node circle {
    stroke: var(--card-bg, #ffffff);
    stroke-width: 2;
  }

  .node-project circle {
    fill: var(--primary, #2563eb);
  }

  .node-note circle {
    fill: #0d9488;
  }

  .node-task circle {
    fill: #d97706;
  }

  .node-entity circle {
    fill: #7c3aed;
  }

  .node.selected circle {
    stroke: var(--text-primary, #0f172a);
    stroke-width: 3;
  }

  .node text {
    font-size: 10.5px;
    fill: var(--text-primary, #1e293b);
    font-weight: 500;
    pointer-events: none;
  }

  .graph-inspector {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.75rem;
    padding: 0.55rem 0.8rem;
    border-top: 1px solid var(--rule);
    background: var(--surface);
    font-size: var(--text-xs);
  }

  .inspector-meta {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .inspector-kind {
    text-transform: uppercase;
    font-size: 0.65rem;
    font-weight: 700;
    padding: 0.1rem 0.35rem;
    border-radius: 3px;
    background: var(--primary-soft, #eff6ff);
    color: var(--primary, #2563eb);
  }

  .inspector-deg {
    color: var(--text-secondary);
  }

  .inspector-actions {
    display: flex;
    gap: 0.4rem;
  }
</style>
