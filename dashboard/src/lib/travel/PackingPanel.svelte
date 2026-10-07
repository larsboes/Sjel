<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import { tip } from "$lib/tip";
  import { bridgedSrc } from "$lib/bridged-url";
  import {
    interior,
    trips,
    type InteriorItem,
    type PackItem,
    type PackView,
  } from "$lib/api";

  let { planId }: { planId: string } = $props();

  let view = $state<PackView | null>(null);
  let owned = $state<InteriorItem[]>([]);
  let error = $state<string | null>(null);
  let busy = $state(false);

  // Shoes first because they are worn, not packed; the rest follows the bag.
  const ORDER = ["schuhe", "kleidung", "reise", "elektronik", "hygiene"];

  const byId = $derived(new Map(owned.map((item) => [item.id, item])));
  const list = $derived(view?.lists[0] ?? null);
  const packedCount = $derived(list ? list.items.filter((row) => row.packed).length : 0);
  const groups = $derived.by(() => {
    const out = new Map<string, PackItem[]>();
    for (const row of list?.items ?? []) {
      const key = row.category ?? byId.get(row.item_ref)?.category ?? "sonstiges";
      out.set(key, [...(out.get(key) ?? []), row]);
    }
    const rank = (key: string) => (ORDER.indexOf(key) + 1 || ORDER.length + 1);
    return [...out.entries()].sort(([a], [b]) => rank(a) - rank(b));
  });
  // What the wardrobe holds and the list does not yet name, grouped like the list.
  const addable = $derived.by(() => {
    const taken = new Set(list?.items.map((row) => row.item_ref));
    return owned
      .filter((item) => item.category && !taken.has(item.id))
      .sort((a, b) => (a.category ?? "").localeCompare(b.category ?? "") || a.label.localeCompare(b.label));
  });
  const addableCategories = $derived([...new Set(addable.map((item) => item.category ?? ""))]);

  async function load(id: string) {
    error = null;
    try {
      const [pack, rows] = await Promise.all([
        trips.pack(id),
        // Inventory only adds pictures and the add menu; a list still reads without it.
        interior.inventory().catch(() => []),
      ]);
      view = pack;
      owned = rows.filter((row) => row.state === "owned").map((row) => row.item);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  $effect(() => {
    void load(planId);
  });

  async function save(next: PackItem[]) {
    if (!list || !view) return;
    const before = view;
    view = { ...view, lists: [{ ...list, items: next }, ...view.lists.slice(1)] };
    try {
      await trips.putPackItems(
        planId,
        list.id,
        next.map(({ item_ref, packed, note }) => ({ item_ref, packed, note })),
      );
    } catch (e) {
      view = before;
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function toggle(row: PackItem) {
    if (!list) return;
    void save(list.items.map((r) => (r.item_ref === row.item_ref ? { ...r, packed: !r.packed } : r)));
  }

  function remove(row: PackItem) {
    if (!list) return;
    void save(list.items.filter((r) => r.item_ref !== row.item_ref));
  }

  function add(id: string) {
    const item = byId.get(id);
    if (!list || !item) return;
    void save([
      ...list.items,
      {
        item_ref: id,
        label: item.label,
        packed: false,
        note: null,
        resolved: true,
        category: item.category,
        weight_g: null,
      },
    ]);
  }

  async function createList() {
    busy = true;
    try {
      await trips.createPackList(planId, "Packing");
      await load(planId);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  function meta(row: PackItem): string {
    const item = byId.get(row.item_ref);
    return [item?.farbe, item?.groesse].filter(Boolean).join(" · ");
  }
</script>

{#if error}
  <p class="pack-error">{error}</p>
{/if}

{#if view && !list}
  <div class="pack-empty">
    <p>No packing list for this trip yet.</p>
    <button type="button" disabled={busy} onclick={() => void createList()}>Create packing list</button>
  </div>
{:else if list}
  <div class="pack-head">
    <strong>{list.name}</strong>
    <span class="pack-count">{packedCount} / {list.items.length} packed</span>
  </div>
  <div class="pack-progress" aria-hidden="true">
    <span style:width={`${list.items.length ? (packedCount / list.items.length) * 100 : 0}%`}></span>
  </div>

  {#each groups as [category, rows] (category)}
    <section class="pack-group">
      <h4>{category}</h4>
      <ul>
        {#each rows as row (row.item_ref)}
          {@const item = byId.get(row.item_ref)}
          <li class:packed={row.packed}>
            <label>
              <input type="checkbox" checked={row.packed} onchange={() => toggle(row)} />
              {#if item?.bild}
                <img use:bridgedSrc={interior.mediaUrl(item.bild)} alt="" loading="lazy" />
              {:else}
                <span class="pack-thumb"><Icon name="boxes" size={14} /></span>
              {/if}
              <span class="pack-text">
                <span class="pack-label">{row.label ?? item?.label ?? row.item_ref}</span>
                {#if meta(row) || row.note}
                  <small>{[meta(row), row.note].filter(Boolean).join(" — ")}</small>
                {/if}
              </span>
            </label>
            {#if item?.link}
              <a class="pack-icon" href={item.link} target="_blank" rel="noreferrer" aria-label={`Product page for ${row.label}`}>
                <Icon name="external" size={12} />
              </a>
            {/if}
            <button
              type="button"
              class="pack-icon"
              onclick={() => remove(row)}
              aria-label={`Remove ${row.label} from the list`}
              use:tip={"Remove from list"}
            >
              <Icon name="close" size={12} />
            </button>
          </li>
        {/each}
      </ul>
    </section>
  {/each}

  {#if addable.length > 0}
    <select
      class="pack-add"
      aria-label="Add from inventory"
      value=""
      onchange={(e) => {
        add(e.currentTarget.value);
        e.currentTarget.value = "";
      }}
    >
      <option value="">Add from inventory…</option>
      {#each addableCategories as category (category)}
        <optgroup label={category}>
          {#each addable.filter((item) => item.category === category) as item (item.id)}
            <option value={item.id}>{item.label}{item.farbe ? ` (${item.farbe})` : ""}</option>
          {/each}
        </optgroup>
      {/each}
    </select>
  {/if}
{/if}

<style>
  .pack-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 0.5rem;
  }

  .pack-head strong {
    line-height: 1.3;
  }

  .pack-count {
    font-size: var(--text-2xs);
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .pack-progress {
    height: 3px;
    margin: 0.4rem 0 0.6rem;
    border-radius: 2px;
    background: var(--card-border);
    overflow: hidden;
  }

  .pack-progress span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width var(--motion-fast) ease;
  }

  .pack-group h4 {
    margin: 0.6rem 0 0.25rem;
    font-size: var(--text-2xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-secondary);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(15rem, 1fr));
    gap: 0.25rem 0.75rem;
  }

  li {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    min-width: 0;
  }

  li label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex: 1;
    min-width: 0;
    padding: 0.2rem 0;
    cursor: pointer;
  }

  li.packed .pack-text {
    opacity: 0.5;
  }

  li.packed .pack-label {
    text-decoration: line-through;
  }

  img,
  .pack-thumb {
    flex: none;
    width: 2.25rem;
    height: 2.25rem;
    border-radius: var(--radius-sm);
    object-fit: cover;
    background: var(--surface);
  }

  .pack-thumb {
    display: grid;
    place-items: center;
    color: var(--text-secondary);
  }

  .pack-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .pack-label {
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pack-text small {
    font-size: var(--text-2xs);
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pack-icon {
    flex: none;
    display: grid;
    place-items: center;
    padding: 0.25rem;
    border: 0;
    background: none;
    color: var(--text-secondary);
    cursor: pointer;
  }

  .pack-icon:hover {
    color: var(--text-primary);
  }

  .pack-add {
    margin-top: 0.75rem;
    padding: 0.3rem 0.5rem;
    border-radius: var(--radius-sm);
    border: 1px solid var(--card-border);
    background: var(--surface);
    color: var(--text-secondary);
    font-size: var(--text-2xs);
  }

  .pack-empty {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .pack-error {
    color: var(--danger, #ef4444);
    font-size: var(--text-2xs);
  }
</style>
