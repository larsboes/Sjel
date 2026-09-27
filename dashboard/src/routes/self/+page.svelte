<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import {
    axonStatus,
    knowledgeGraph,
    type SelfModelResponse,
    type SelfUnit,
    type UnitGraph,
  } from "$lib/api";
  import RepoStatusCard from "$lib/RepoStatusCard.svelte";
  import UnitMap, { type MapNode, type MapEdge } from "$lib/UnitMap.svelte";

  // Two altitudes, one page. The self-model is ~31 units and 16 couplings --
  // small enough to be a picture, which is the altitude at which Sjel can
  // actually be explained. graphify's 6.6k-node graph is the drill-down under
  // it, fetched one unit at a time and capped, never rendered whole. The
  // knowledge-graph panel that used to live on :4244 did render it whole, which
  // is why it was a black rectangle; this page is where that view went.

  let data = $state<SelfModelResponse | null>(null);
  let error = $state<string | null>(null);
  let selected = $state<string | null>(null);

  /** Drill-down state: null until a unit is opened at file level. */
  let inside = $state<UnitGraph | null>(null);
  let insideError = $state<string | null>(null);
  let insideLoading = $state(false);
  /** Which file node is picked inside a unit. Its own selection, not the unit's. */
  let insideSelected = $state<string | null>(null);

  onMount(() => {
    axonStatus
      .self()
      .then((d) => (data = d))
      .catch((e) => (error = e instanceof Error ? e.message : String(e)));
  });

  const model = $derived(data?.model ?? null);
  const live = $derived(data?.live ?? {});

  // ── The unit map ──────────────────────────────────────────────────────────

  /** Largest unit by file count, so radius is relative to this repo, not absolute. */
  const largestUnit = $derived(
    Math.max(1, ...(model?.units ?? []).map((u) => u.code?.files ?? 0)),
  );

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
    weight: (u.code?.files ?? 0) / largestUnit,
    state: live[u.name] === true ? "up" : live[u.name] === false ? "down" : null,
  });

  const mapNodes = $derived<MapNode[]>(
    (model?.units ?? []).filter((u) => coupledNames.has(u.name)).map(toMapNode),
  );

  const isolated = $derived((model?.units ?? []).filter((u) => !coupledNames.has(u.name)));

  const mapEdges = $derived<MapEdge[]>(
    (model?.coupling ?? []).map((c) => ({ from: c.from, to: c.to, label: c.kinds.join(", ") })),
  );

  /** Inside a unit, colour by what the file is rather than by what it belongs to. */
  function fileKind(fileType: string): string {
    if (fileType === "doc" || fileType === "markdown") return "doc";
    if (fileType === "config" || fileType === "json") return "config";
    return "file";
  }

  const insideNodes = $derived<MapNode[]>(
    (inside?.nodes ?? []).map((n) => ({
      id: n.id,
      label: n.label,
      kind: fileKind(n.file_type),
      weight: 0.3,
    })),
  );

  const insideEdges = $derived<MapEdge[]>(
    (inside?.edges ?? []).map((e) => ({ from: e.from, to: e.to, label: e.label })),
  );

  /**
   * Open a unit at file level.
   *
   * knowledge-graph starts on demand, so a first call while it is down is the
   * normal case rather than a fault: ask axon-status to bring it up, then try
   * once more. A second failure is reported as itself.
   */
  const insideDetail = $derived(
    insideSelected ? (inside?.nodes.find((n) => n.id === insideSelected) ?? null) : null,
  );

  async function openInside(unit: string): Promise<void> {
    insideLoading = true;
    insideError = null;
    insideSelected = null;
    try {
      inside = await knowledgeGraph.unit(unit);
    } catch {
      try {
        await axonStatus.start("knowledge-graph");
        inside = await knowledgeGraph.unit(unit);
      } catch (e) {
        inside = null;
        insideError = e instanceof Error ? e.message : String(e);
      }
    } finally {
      insideLoading = false;
    }
  }

  function closeInside(): void {
    inside = null;
    insideError = null;
    insideSelected = null;
  }

  function select(name: string | null): void {
    selected = name;
    if (inside && inside.unit !== name) closeInside();
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

  function toggle(u: SelfUnit) {
    select(selected === u.name ? null : u.name);
  }
</script>

<PageHeader
  badge="Self-model"
  title="Sjel about Sjel"
  desc="What is here, how the units connect, and how large each one is. Generated from tracked files and supplemented with live state."
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
    <li><b>{model.graph.present ? model.graph.nodes : "—"}</b><span>Graph nodes</span></li>
    <li><b>{model.upstreams.length}</b><span>Upstreams</span></li>
  </ul>

  {#if model.graph.stale.length}
    <p class="err">
      <Icon name="alert" size={14} />
      {model.graph.stale.length} graph paths no longer exist — <span class="mono"
        >tools/graphify.sh</span
      >
    </p>
  {/if}

  <!--
    The map is the teaching surface, so it says what it means before it draws.
    A reader who cannot tell a coupling edge from a runtime dependency learns
    the wrong thing from a picture, confidently.
  -->
  <div class="mapwrap">
    {#if inside}
      <p class="maplead">
        Inside <b>{inside.unit}</b> — one dot per file or symbol graphify extracted, one line
        per reference between them.
        {#if inside.truncated}
          Showing the <b>{inside.returned}</b> most-connected of
          <b>{inside.total}</b>; the rest are real, just not drawn.
        {:else}
          All <b>{inside.returned}</b> of them.
        {/if}
        <button class="link" onclick={closeInside}>← back to units</button>
      </p>
      <UnitMap
        nodes={insideNodes}
        edges={insideEdges}
        selected={insideSelected}
        onSelect={(id) => (insideSelected = id)}
        height={520}
      />
      <!--
        Names are hidden at this density, so a picked node has to say what it
        is somewhere. This line is that somewhere -- the path matters more than
        the symbol, because it is what a reader can go open.
      -->
      <p class="picked">
        {#if insideDetail}
          <b>{insideDetail.label}</b>
          <span class="mono">{insideDetail.source_file}</span>
          <span class="tag">{insideDetail.file_type}</span>
        {:else}
          Click any dot to name it. Hover reads one out without selecting it.
        {/if}
      </p>
    {:else}
      <p class="maplead">
        The <b>{mapNodes.length}</b> units that couple to something, sized by how much code they
        hold and ringed when this machine has them running. A line means one unit
        <b>compiles the other in</b> — not that it calls it at runtime. Click to select, drag to
        move, scroll to zoom.
      </p>
      <UnitMap nodes={mapNodes} edges={mapEdges} {selected} onSelect={select} height={460} />
    {/if}

    <ul class="legend">
      {#if inside}
        <li><span class="swatch kind-file"></span>code</li>
        <li><span class="swatch kind-doc"></span>docs</li>
        <li><span class="swatch kind-config"></span>config</li>
      {:else}
        <li><span class="swatch kind-capability"></span>capability</li>
        <li><span class="swatch kind-lib"></span>lib</li>
        <li><span class="swatch kind-spine"></span>spine</li>
        <li><span class="swatch kind-pack"></span>pack</li>
        <li><span class="swatch ring-up"></span>running</li>
        <li><span class="swatch ring-down"></span>off</li>
      {/if}
    </ul>

    {#if !inside && isolated.length}
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
        {#if detail.code}
          <button
            class="inside"
            disabled={insideLoading || inside?.unit === detail.name}
            onclick={() => openInside(detail.name)}
          >
            {#if insideLoading}
              <Icon name="loader" size={12} /> opening…
            {:else if inside?.unit === detail.name}
              open below
            {:else}
              Look inside
            {/if}
          </button>
        {/if}
        <button class="x" onclick={() => select(null)} aria-label="Clear selection">
          <Icon name="close" size={13} />
        </button>
      </header>
      {#if insideError}
        <p class="err small">
          <Icon name="alert" size={13} /> knowledge-graph unavailable — {insideError}. Run
          <span class="mono">tools/graphify.sh</span> if the graph was never built.
        </p>
      {/if}
      <dl>
        {#if detail.code}
          <div><dt>Code</dt><dd>{detail.code.files} files · {detail.code.nodes} nodes</dd></div>
        {/if}
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
    <table>
      <thead>
        <tr>
          <th scope="col"></th>
          <th scope="col">Unit</th>
          <th scope="col">Kind</th>
          <th class="num" scope="col">Files</th>
          <th class="num" scope="col">Port</th>
          <th scope="col">Runtime dependencies</th>
          <th class="num" scope="col">Couplings</th>
        </tr>
      </thead>
      <tbody>
        {#each model.units as u (u.name)}
          {@const deg = model.coupling.filter((c) => c.from === u.name || c.to === u.name).length}
          <tr
            class:dim={!related(u.name)}
            class:sel={selected === u.name}
            onclick={() => toggle(u)}
          >
            <td>
              <span class="dot {upClass(u.name)}" title={upLabel(u.name)}></span>
            </td>
            <td class="mono name">{u.name}</td>
            <td><span class="kind">{u.kind}</span></td>
            <td class="num">{u.code?.files ?? "—"}</td>
            <td class="num mono">{u.service?.port ?? "—"}</td>
            <td class="req">{u.service?.requires.length ? u.service.requires.join(", ") : "—"}</td>
            <td class="num">{deg || "—"}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

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
  .link {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    color: var(--primary);
    cursor: pointer;
    text-decoration: underline;
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
  .legend .kind-file {
    --swatch: #0e7490;
  }
  .legend .kind-doc {
    --swatch: #15803d;
  }
  .legend .kind-config {
    --swatch: #b45309;
  }
  :global(:root.dark) .legend .kind-capability,
  :global(:root.dark) .legend .kind-file {
    --swatch: #22d3ee;
  }
  :global(:root.dark) .legend .kind-lib {
    --swatch: #a78bfa;
  }
  :global(:root.dark) .legend .kind-spine,
  :global(:root.dark) .legend .kind-config {
    --swatch: #fbbf24;
  }
  :global(:root.dark) .legend .kind-pack,
  :global(:root.dark) .legend .kind-doc {
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

  .picked {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
    font-size: 0.78rem;
    color: var(--text-secondary);
    margin: 0.5rem 0 0;
    min-height: 1.2rem;
  }
  .picked b {
    color: var(--text-primary);
  }
  .picked .mono {
    overflow-wrap: anywhere;
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
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.84rem;
  }
  th {
    text-align: left;
    font-weight: 500;
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    opacity: 0.55;
    padding: 0 0.55rem 0.4rem;
    white-space: nowrap;
  }
  td {
    padding: 0.42rem 0.55rem;
    border-top: 1px solid var(--line, #2a2a3e);
    white-space: nowrap;
  }
  tbody tr {
    cursor: pointer;
  }
  tbody tr:hover td {
    background: var(--hover, rgba(255, 255, 255, 0.04));
  }
  tr.dim {
    opacity: 0.28;
  }
  tr.sel td {
    background: var(--hover, rgba(255, 255, 255, 0.07));
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
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
    background: var(--muted, #555);
  }
  .dot.up {
    background: var(--ok, #4ba36a);
  }
  .dot.down {
    background: var(--warn, #e0a458);
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
     only takes it when the unit has no code to look inside of. */
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
