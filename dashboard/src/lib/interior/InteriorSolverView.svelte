<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import DataTable, { type Column } from "$lib/ui/DataTable.svelte";
  import {
    interior,
    type InteriorComposed,
    type InteriorHit,
    type InteriorLayoutDetail,
    type InteriorSearchReport,
    type InventoryRow,
  } from "$lib/api";

  interface Props {
    selected: string | null;
    detail: InteriorLayoutDetail | null;
    allOwned: InventoryRow[];
  }

  let { selected, detail, allOwned }: Props = $props();

  let moveRefs = $state<string[]>([]);
  let composeRefs = $state<string[]>([]);
  let step = $state(20);
  let solverBusy = $state(false);
  let solverError = $state<string | null>(null);
  let solverElapsed = $state(0);
  let searchResult = $state<InteriorSearchReport | null>(null);
  let composeResult = $state<InteriorComposed[] | null>(null);

  async function awaitJob<T>(id: number): Promise<T> {
    const started = Date.now();
    for (;;) {
      const { state } = await interior.job(id);
      if (state.state === "done") return state.outcome as T;
      if (state.state === "failed") throw new Error(state.reason);
      solverElapsed = Math.round((Date.now() - started) / 1000);
      await new Promise((r) => setTimeout(r, 1000));
    }
  }

  async function runSearch(): Promise<void> {
    if (selected === null || moveRefs.length === 0) return;
    solverBusy = true;
    solverError = null;
    searchResult = null;
    composeResult = null;
    try {
      const { job } = await interior.search(selected, moveRefs, step, 12);
      searchResult = await awaitJob<InteriorSearchReport>(job);
    } catch (caught) {
      solverError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      solverBusy = false;
    }
  }

  async function runCompose(): Promise<void> {
    if (composeRefs.length === 0) return;
    solverBusy = true;
    solverError = null;
    searchResult = null;
    composeResult = null;
    try {
      const { job } = await interior.compose(composeRefs, 25, 60, 5);
      composeResult = await awaitJob<InteriorComposed[]>(job);
    } catch (caught) {
      solverError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      solverBusy = false;
    }
  }

  function toggle(list: string[], id: string): string[] {
    return list.includes(id) ? list.filter((x) => x !== id) : [...list, id];
  }

  /** A hit with its place in the result: two hits can name the same positions. */
  type HitRow = InteriorHit & { rank: number };

  const hits: HitRow[] = $derived((searchResult?.hits ?? []).map((hit, index) => ({ ...hit, rank: index })));

  const places = (row: HitRow): string =>
    Object.entries(row.places)
      .map(([ref, [x, y]]) => `${ref} ${x},${y}`)
      .join(" · ");

  const hitColumns: Column<HitRow>[] = [
    { id: "places", label: "Places", wrap: true, cell: placesCell },
    { id: "reserve", label: "Tightest reserve", width: "9rem", align: "end", cell: reserveCell },
    { id: "wall", label: "Wall", width: "8rem", align: "end", cell: wallCell },
    { id: "walk", label: "Walk", width: "7rem", align: "end", cell: walkCell },
    { id: "soft", label: "Soft", width: "5rem", align: "end", cell: softCell },
  ];
</script>

{#snippet placesCell(row: HitRow)}
  {row.pareto ? "★ " : ""}{places(row)}
{/snippet}

{#snippet reserveCell(row: HitRow)}
  {row.tightest_reserve_cm ?? "—"} cm spare
{/snippet}

{#snippet wallCell(row: HitRow)}
  {row.wall_contact_cm} cm wall
{/snippet}

{#snippet walkCell(row: HitRow)}
  {row.bottleneck_cm} cm walk
{/snippet}

{#snippet softCell(row: HitRow)}
  {row.soft} soft
{/snippet}

<section class="card solver">
  <h2>Move pieces in {selected ?? "a layout"}</h2>
  {#if selected === null}
    <p class="empty">Pick a layout under Plans first.</p>
  {:else}
    <p class="note">
      Every position on the grid is checked, not the first that works. A ★ marks the Pareto
      front: under no weighting of the four goals is such a hit worse than another.
    </p>
    <div class="picks">
      {#each detail?.layout.item ?? [] as it (JSON.stringify(it))}
        {@const ref = (it as { ref: string }).ref}
        <label>
          <input
            type="checkbox"
            checked={moveRefs.includes(ref)}
            onchange={() => (moveRefs = toggle(moveRefs, ref))}
          />
          {ref}
        </label>
      {/each}
    </div>
    <label class="stepper">
      grid <input type="number" min="5" max="50" step="5" bind:value={step} /> cm
    </label>
    <button type="button" disabled={solverBusy || moveRefs.length === 0} onclick={runSearch}>
      {solverBusy ? `Searching… ${solverElapsed}s` : "Search"}
    </button>
  {/if}
</section>

<section class="card solver">
  <h2>Lay out the flat from nothing</h2>
  <p class="note">
    A beam search, and it says so: only the best partial arrangements survive each round, so
    the answer is computed and reproducible — not proven optimal.
  </p>
  <div class="picks">
    {#each allOwned as row (row.item.id)}
      <label>
        <input
          type="checkbox"
          checked={composeRefs.includes(row.item.id)}
          onchange={() => (composeRefs = toggle(composeRefs, row.item.id))}
        />
        {row.item.label}
      </label>
    {/each}
  </div>
  <button type="button" disabled={solverBusy || composeRefs.length === 0} onclick={runCompose}>
    {solverBusy ? `Placing… ${solverElapsed}s` : "Lay it out"}
  </button>
</section>

{#if solverError}<p class="error"><Icon name="alert" size={15} /> {solverError}</p>{/if}

{#if searchResult}
  <section class="card">
    <h2>
      {searchResult.hits.length} of {searchResult.fully_checked.toLocaleString()} checked
      <span class="note">in {(searchResult.elapsed_ms / 1000).toFixed(1)} s</span>
    </h2>
    <DataTable rows={hits} columns={hitColumns} key={(row) => String(row.rank)} rowHeader />
  </section>
{/if}

{#if composeResult}
  <section class="card">
    <h2>{composeResult.length} arrangements</h2>
    {#each composeResult as c, i (i)}
      <article class="composed">
        <h3>
          #{i + 1} {c.pareto ? "★" : ""}
          <span class="verdict" class:pass={c.pass}>{c.pass ? "passes" : "fails"}</span>
        </h3>
        <p class="mono">
          {c.tightest_reserve_cm ?? "—"} cm spare · {c.wall_contact_cm} cm wall ·
          {c.bottleneck_cm} cm walk · {c.free_m2.toFixed(2)} m² free
        </p>
        {#if c.hard.length > 0}<p class="note">hard: {c.hard.join(", ")}</p>{/if}
        <ul class="places mono">
          {#each c.places as [ref, p, rot] (ref)}
            <li>{ref} — x {p[0]}, y {p[1]}, {rot}°</li>
          {/each}
        </ul>
      </article>
    {/each}
  </section>
{/if}

<style>
  .card {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    margin-bottom: 1rem;
    padding: 1.25rem;
  }
  .solver h2 {
    font-size: var(--text-base);
    margin: 0 0 0.5rem;
  }
  .picks {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem 1rem;
    margin: 0.6rem 0;
  }
  .picks label,
  .stepper {
    align-items: center;
    color: var(--text-secondary);
    display: inline-flex;
    font-size: var(--text-sm);
    gap: 0.35rem;
  }
  .stepper input {
    width: 4.5rem;
  }
  .composed {
    border-top: 1px solid var(--card-border);
    padding: 0.6rem 0;
  }
  .composed h3 {
    font-size: var(--text-sm);
    margin: 0 0 0.3rem;
  }
  .places {
    color: var(--text-tertiary);
    font-size: var(--text-xs);
    list-style: none;
    margin: 0.35rem 0 0;
    padding: 0;
  }
  .verdict {
    border-radius: var(--radius-sm);
    color: var(--danger);
    font-size: var(--text-xs);
    font-weight: 600;
    margin-left: 0.5rem;
    padding: 0.15rem 0.45rem;
  }
  .verdict.pass {
    color: var(--success);
  }
  .note {
    color: var(--text-tertiary);
    font-size: var(--text-xs);
    margin: 0.25rem 0;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .error {
    color: var(--danger);
    font-size: var(--text-sm);
    margin: 0.5rem 0;
  }
  .empty {
    color: var(--text-tertiary);
    font-size: var(--text-sm);
    margin: 0.5rem 0;
  }
</style>
