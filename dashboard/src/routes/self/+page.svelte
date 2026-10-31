<script lang="ts">
  import { tip } from "$lib/tip";
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import {
    sjelStatus,
    type SelfModelResponse,
    type SelfUnit,
  } from "$lib/api";
  import RepoStatusCard from "$lib/RepoStatusCard.svelte";
  import UnitMap, { type MapNode, type MapEdge } from "$lib/UnitMap.svelte";
  import Collection from "$lib/ui/Collection.svelte";
  import type { Field } from "$lib/ui/collection";

  // The self-model is ~31 units and 16 couplings -- small enough to be a
  // picture, which is the altitude at which Sjel can actually be explained.

  let data = $state<SelfModelResponse | null>(null);
  let error = $state<string | null>(null);
  let selected = $state<string | null>(null);

  let lifecycleBusy = $state<string | null>(null);

  async function runLifecycle(name: string, action: "start" | "stop" | "restart"): Promise<void> {
    lifecycleBusy = name;
    error = null;
    try {
      if (action === "start") await sjelStatus.start(name);
      else if (action === "stop") await sjelStatus.stop(name);
      else await sjelStatus.restart(name);
      data = await sjelStatus.self();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      lifecycleBusy = null;
    }
  }

  onMount(() => {
    sjelStatus
      .self()
      .then((d) => (data = d))
      .catch((e) => (error = e instanceof Error ? e.message : String(e)));
  });

  const model = $derived(data?.model ?? null);
  const live = $derived(data?.live ?? {});

  // ── The unit map ──────────────────────────────────────────────────────────

  /**
   * Only coupled units go in the map.
   *
   * Most units compile nothing in and are compiled into nothing, and a force
   * layout has nowhere to put such a node except away from everything -- so
   * they ended up as a border of unrelated dots around the part worth reading,
   * taking most of the canvas with them. They are not missing: they are listed
   * under the map, where "couples to nothing" is the fact being shown.
   */
  const coupledNames = $derived(
    new Set((model?.coupling ?? []).flatMap((c) => [c.from, c.to])),
  );

  const toMapNode = (u: SelfUnit): MapNode => ({
    id: u.name,
    label: u.name,
    kind: u.kind,
    weight: 0.5,
    state: live[u.name] === true ? "up" : live[u.name] === false ? "down" : null,
  });

  const mapNodes = $derived<MapNode[]>(
    (model?.units ?? []).filter((u) => coupledNames.has(u.name)).map(toMapNode),
  );

  const isolated = $derived((model?.units ?? []).filter((u) => !coupledNames.has(u.name)));

  const mapEdges = $derived<MapEdge[]>(
    (model?.coupling ?? []).map((c) => ({ from: c.from, to: c.to, label: c.kinds.join(", ") })),
  );

  function select(name: string | null): void {
    selected = name;
  }

  /** Outgoing = what this unit compiles in. Incoming = who compiles it in. */
  const outgoing = $derived(
    selected && model ? model.coupling.filter((c) => c.from === selected) : [],
  );
  const incoming = $derived(
    selected && model ? model.coupling.filter((c) => c.to === selected) : [],
  );

  const related = (name: string): boolean =>
    !selected ||
    selected === name ||
    outgoing.some((c) => c.to === name) ||
    incoming.some((c) => c.from === name);

  const detail = $derived(
    selected && model ? (model.units.find((u) => u.name === selected) ?? null) : null,
  );

  function upClass(name: string): string {
    const up = live[name];
    if (up === true) return "up";
    if (up === false) return "down";
    return "unknown";
  }

  function upLabel(name: string): string {
    const up = live[name];
    if (up === true) return "running";
    if (up === false) return "off";
    return "no health check";
  }

  const degree = (name: string) => model?.coupling.filter((c) => c.from === name || c.to === name).length ?? 0;

  const unitFields: Field<SelfUnit>[] = [
    {
      id: "health", label: "", kind: "select", width: "2rem", value: (u) => upLabel(u.name),
      tone: (v) => (v === "running" ? "success" : v === "off" ? "warning" : "muted"), cell: healthCell,
    },
    { id: "unit", label: "Unit", value: (u) => u.name, cell: unitCell },
    { id: "kind", label: "Kind", kind: "select", width: "7rem", value: (u) => u.kind },
    { id: "port", label: "Port", kind: "number", width: "5rem", value: (u) => u.service?.port },
    { id: "requires", label: "Runtime dependencies", value: (u) => u.service?.requires.join(", "), cell: requiresCell },
    { id: "couplings", label: "Couplings", kind: "number", width: "6rem", value: (u) => degree(u.name) || null },
  ];

  function toggle(u: SelfUnit) {
    select(selected === u.name ? null : u.name);
  }
</script>

<PageHeader
  badge="Self-model"
  title="Sjel about Sjel"
  desc="What is here and how the units connect. Generated from tracked files and supplemented with live state."
/>

<!--
  Version identity belongs on the self-model page for the same reason the rest of it
  does: it is a fact about this repo, not about this machine. The home page carries the
  same card in its compact form, so the link is one glance away without this page.
-->
<RepoStatusCard detailed />

{#if error}
  <p class="err">
    <Icon name="alert" size={14} /> {error}
  </p>
{:else if !model}
  <p class="loading"><Icon name="loader" size={14} /> loading…</p>
{:else}
  <ul class="stats">
    <li><b>{model.units.length}</b><span>Units</span></li>
    <li><b>{model.coupling.length}</b><span>Couplings</span></li>
    <li><b>{model.upstreams.length}</b><span>Upstreams</span></li>
  </ul>

  <!--
    The map is the teaching surface, so it says what it means before it draws.
    A reader who cannot tell a coupling edge from a runtime dependency learns
    the wrong thing from a picture, confidently.
  -->
  <div class="mapwrap">
    <p class="maplead">
      The <b>{mapNodes.length}</b> units that couple to something, ringed when this machine has
      them running. A line means one unit <b>compiles the other in</b> — not that it calls it at
      runtime. Click to select, drag to move, scroll to zoom.
    </p>
    <UnitMap nodes={mapNodes} edges={mapEdges} {selected} onSelect={select} height={460} />

    <ul class="legend">
      <li><span class="swatch kind-capability"></span>capability</li>
      <li><span class="swatch kind-lib"></span>lib</li>
      <li><span class="swatch kind-spine"></span>spine</li>
      <li><span class="swatch kind-pack"></span>pack</li>
      <li><span class="swatch ring-up"></span>running</li>
      <li><span class="swatch ring-down"></span>off</li>
    </ul>

    {#if isolated.length}
      <p class="isolated-lead">
        <b>{isolated.length}</b> units compile nothing in and are compiled into nothing. That is
        the normal state for a self-contained capability, not a gap.
      </p>
      <ul class="chips">
        {#each isolated as u (u.name)}
          <li>
            <button class:sel={selected === u.name} onclick={() => toggle(u)}>
              <span class="dot {upClass(u.name)}"></span>{u.name}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  {#if selected && detail}
    <div class="card detail">
      <header>
        <strong>{detail.name}</strong>
        <span class="kind">{detail.kind}</span>
          </button>
        {/if}
        {#if detail.service}
          {@const isUp = live[detail.name] === true}
          <button
            class="inside"
            disabled={lifecycleBusy === detail.name}
            onclick={() => runLifecycle(detail.name, "restart")}
          >
            {lifecycleBusy === detail.name ? "restarting…" : "↻ Restart"}
          </button>
          {#if detail.name !== "sjel-status"}
            <button
              class="inside"
              disabled={lifecycleBusy === detail.name}
              onclick={() => runLifecycle(detail.name, isUp ? "stop" : "start")}
            >
              {isUp ? "Stop" : "Start"}
            </button>
          {/if}
        {/if}
        <button class="x" onclick={() => select(null)} aria-label="Clear selection">
          <Icon name="close" size={13} />
        </button>
      </header>
      <dl>
        {#if detail.service}
          <div>
            <dt>Service</dt>
            <dd>
              {detail.service.kind}{detail.service.port ? ` · Port ${detail.service.port}` : ""}
            </dd>
          </div>
          <div>
            <dt>runtime dependencies</dt>
            <dd>{detail.service.requires.length ? detail.service.requires.join(", ") : "—"}</dd>
          </div>
        {/if}
        <div>
          <dt>compiles in</dt>
          <dd>{outgoing.length ? outgoing.map((c) => c.to).join(", ") : "—"}</dd>
        </div>
        <div>
          <dt>compiled into</dt>
          <dd>{incoming.length ? incoming.map((c) => c.from).join(", ") : "—"}</dd>
        </div>
      </dl>
      {#if outgoing.length}
        <ul class="evidence">
          {#each outgoing as c (c.to)}
            <li>
              <span class="mono">{c.to}</span>
              {#each c.kinds as k (k)}<span class="tag">{k}</span>{/each}
              <span class="mono files">{c.evidence.join(" · ")}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}

  <p class="hint">
    Select a row to see its couplings. <b>runtime dependencies</b> come from
    <span class="mono">requires=</span>; <b>compiles in</b> comes from
    <span class="mono">#[path]</span> or a Cargo path dependency. They are different relationships.
  </p>

  <div class="wrap">
    <Collection
      id="units"
      rows={model.units}
      fields={unitFields}
      key={(u) => u.name}
      title={(u) => u.name}
      onOpen={toggle}
      selected={selected}
      inactive={(u) => !related(u.name)}
    />
  </div>
{/if}

{#snippet healthCell(u: SelfUnit)}<span class="dot {upClass(u.name)}" use:tip={upLabel(u.name)}></span>{/snippet}
{#snippet unitCell(u: SelfUnit)}<span class="mono name">{u.name}</span>{/snippet}
{#snippet requiresCell(u: SelfUnit)}<span class="req">{u.service?.requires.length ? u.service.requires.join(", ") : "—"}</span>{/snippet}

<style>
  .stats {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    list-style: none;
    padding: 0;
    margin: 0 0 1rem;
  }
  .stats li {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    padding: 0.55rem 0.9rem;
    border: 1px solid var(--line, #2a2a3e);
    border-radius: 8px;
    min-width: 6.5rem;
  }
  .stats b {
    font-size: 1.25rem;
    font-variant-numeric: tabular-nums;
  }
  .stats span {
    font-size: 0.72rem;
    opacity: 0.6;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .hint {
    font-size: 0.8rem;
    opacity: 0.65;
    margin: 0 0 0.6rem;
    line-height: 1.5;
  }

  .mapwrap {
    margin: 0 0 1rem;
  }
  .maplead {
    font-size: 0.8rem;
    color: var(--text-secondary);
    line-height: 1.55;
    margin: 0 0 0.5rem;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem 0.9rem;
    list-style: none;
    padding: 0;
    margin: 0.5rem 0 0;
    font-size: 0.72rem;
    color: var(--text-secondary);
  }
  .legend li {
    display: flex;
    align-items: center;
    gap: 0.35rem;
  }
  .swatch {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--swatch, var(--text-tertiary));
  }
  /* The legend mirrors UnitMap's palette. Two copies of six hex values is the
     price of not exporting a colour table from a component; if a third surface
     ever needs them they move into app.css as tokens. */
  .legend .kind-capability {
    --swatch: #0e7490;
  }
  .legend .kind-lib {
    --swatch: #7c3aed;
  }
  .legend .kind-spine {
    --swatch: #b45309;
  }
  .legend .kind-pack {
    --swatch: #15803d;
  }
  :global(:root.dark) .legend .kind-capability {
    --swatch: #22d3ee;
  }
  :global(:root.dark) .legend .kind-lib {
    --swatch: #a78bfa;
  }
  :global(:root.dark) .legend .kind-spine {
    --swatch: #fbbf24;
  }
  :global(:root.dark) .legend .kind-pack {
    --swatch: #4ade80;
  }
  .legend .ring-up,
  .legend .ring-down {
    background: none;
    border: 2px solid var(--success);
  }
  .legend .ring-down {
    border-color: var(--warning);
  }

  .isolated-lead {
    font-size: 0.78rem;
    color: var(--text-secondary);
    margin: 0.9rem 0 0.4rem;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    list-style: none;
    padding: 0;
    margin: 0;
  }
  .chips button {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font: inherit;
    font-size: 0.74rem;
    padding: 0.2rem 0.55rem;
    border: 1px solid var(--card-border);
    border-radius: 999px;
    background: var(--card-bg);
    color: var(--text-secondary);
    cursor: pointer;
  }
  .chips button:hover {
    color: var(--text-primary);
    border-color: var(--card-border-hover);
  }
  .chips button.sel {
    color: var(--text-primary);
    border-color: var(--primary);
  }

  .inside {
    margin-left: auto;
    font: inherit;
    font-size: 0.72rem;
    display: flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.22rem 0.6rem;
    border: 1px solid var(--card-border);
    border-radius: 6px;
    background: var(--card-bg);
    color: var(--text-secondary);
    cursor: pointer;
  }
  .inside:hover:not(:disabled) {
    color: var(--text-primary);
    border-color: var(--card-border-hover);
  }
  .inside:disabled {
    opacity: 0.55;
    cursor: default;
  }

  .err.small {
    font-size: 0.78rem;
    margin: 0.5rem 0 0;
  }

  .err,
  .loading {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    font-size: 0.85rem;
    margin: 0 0 0.9rem;
  }
  .err {
    color: var(--warn, #e0a458);
  }

  /*
   * min-width: 0 guards the flex-item trap: `main` is a flex item whose default
   * min-width: auto refuses to shrink below its content, which would let a table of
   * nowrap columns widen the page instead of scrolling inside its own box. Kept as a
   * guard, not a fix for an observed bug — measured scrollWidth equals clientWidth at
   * this viewport, so the table already fits. It earns its place at narrower widths.
   */
  .wrap {
    overflow-x: auto;
    min-width: 0;
    max-width: 100%;
  }
  .name {
    font-weight: 500;
  }
  .req {
    opacity: 0.75;
  }

  .kind {
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.6;
  }

  .dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-tertiary);
  }
  .dot.up {
    background: var(--success);
  }
  .dot.down {
    background: var(--warning);
  }

  .detail {
    padding: 0.9rem 1rem;
    margin: 0 0 0.9rem;
    min-width: 0;
    max-width: 100%;
  }
  .detail header {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    margin-bottom: 0.6rem;
  }
  /* `.inside` claims the auto margin when it is present, so the close button
     only takes it when the unit has no service buttons. */
  .detail header .x {
    margin-left: auto;
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
    opacity: 0.6;
    padding: 0.2rem;
    display: flex;
  }
  .detail header .inside ~ .x {
    margin-left: 0;
  }
  .detail header .x:hover {
    opacity: 1;
  }
  .detail dl {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
    gap: 0.5rem 1rem;
    margin: 0;
  }
  .detail dt {
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.55;
  }
  .detail dd {
    margin: 0.1rem 0 0;
    font-size: 0.84rem;
  }

  .evidence {
    list-style: none;
    padding: 0.7rem 0 0;
    margin: 0.7rem 0 0;
    border-top: 1px solid var(--line, #2a2a3e);
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  .evidence li {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    flex-wrap: wrap;
    font-size: 0.78rem;
  }
  .evidence .files {
    opacity: 0.5;
    font-size: 0.72rem;
    /* Repo-relative paths are long and unbreakable; let them break rather than widen
       the page. Same reason as .wrap's min-width. */
    overflow-wrap: anywhere;
  }
  .tag {
    font-size: 0.65rem;
    padding: 0.05rem 0.35rem;
    border: 1px solid var(--line, #2a2a3e);
    border-radius: 4px;
    opacity: 0.75;
  }
</style>
