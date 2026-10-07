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
  /** The card whose details are open, and the group whose add-picker is open. */
  let openRef = $state<string | null>(null);
  let addingTo = $state<string | null>(null);

  // Bag order: what carries the rest, what must not be forgotten, then what is worn and packed.
  const GROUPS: [string, string][] = [
    ["gepaeck", "Bag"],
    ["dokumente", "Documents"],
    ["elektronik", "Electronics"],
    ["schuhe", "Shoes"],
    ["kleidung", "Clothing"],
    ["hygiene", "Toiletries"],
    ["reise", "Travel"],
    ["freizeit", "Fun"],
  ];
  const groupLabel = (key: string) => GROUPS.find(([k]) => k === key)?.[1] ?? key;
  const rank = (key: string) => {
    const i = GROUPS.findIndex(([k]) => k === key);
    return i === -1 ? GROUPS.length : i;
  };

  // A few traits carry meaning at a glance; the rest share one neutral tone.
  const TONE: Record<string, string> = {
    rain: "rain",
    warm: "warm",
    style: "style",
    comfort: "comfort",
    "odor-resistant": "fresh",
    "quick-dry": "fresh",
    essential: "essential",
    worn: "essential",
  };

  const byId = $derived(new Map(owned.map((item) => [item.id, item])));
  const list = $derived(view?.lists[0] ?? null);
  const packedCount = $derived(list ? list.items.filter((row) => row.packed).length : 0);

  function groupOf(row: { category: string | null; item_ref: string }): string {
    return row.category ?? byId.get(row.item_ref)?.category ?? "sonstiges";
  }

  const groups = $derived.by(() => {
    const out = new Map<string, PackItem[]>();
    for (const row of list?.items ?? []) {
      const key = groupOf(row);
      out.set(key, [...(out.get(key) ?? []), row]);
    }
    return [...out.entries()].sort(([a], [b]) => rank(a) - rank(b));
  });

  const onList = $derived(new Set(list?.items.map((row) => row.item_ref)));
  const candidates = (group: string) =>
    owned
      .filter((item) => (item.category ?? "sonstiges") === group && !onList.has(item.id))
      .sort((a, b) => a.label.localeCompare(b.label));
  /** Groups the wardrobe holds that the list does not use yet. */
  const otherGroups = $derived(
    [...new Set(owned.map((item) => item.category).filter((c): c is string => !!c))]
      .filter((g) => !groups.some(([key]) => key === g) && candidates(g).length > 0)
      .sort((a, b) => rank(a) - rank(b)),
  );

  /** Stated traits plus the two the booleans already say, without repeats. */
  function traitsOf(item: InteriorItem | undefined): string[] {
    if (!item) return [];
    const out = [...(item.traits ?? [])];
    if (item.waterproof && !out.includes("rain")) out.push("rain");
    if (item.quick_dry && !out.includes("quick-dry")) out.push("quick-dry");
    return out;
  }

  async function load(id: string) {
    error = null;
    try {
      const [pack, rows] = await Promise.all([
        trips.pack(id),
        // Inventory adds pictures, traits and the add-picker; the list still reads without it.
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

  const update = (ref: string, change: Partial<PackItem>) =>
    list && void save(list.items.map((r) => (r.item_ref === ref ? { ...r, ...change } : r)));

  function remove(ref: string) {
    if (!list) return;
    openRef = null;
    void save(list.items.filter((r) => r.item_ref !== ref));
  }

  function add(item: InteriorItem) {
    if (!list) return;
    void save([
      ...list.items,
      {
        item_ref: item.id,
        label: item.label,
        packed: false,
        note: null,
        resolved: true,
        category: item.category,
        weight_g: item.weight_g ?? null,
      },
    ]);
  }

  function packGroup(rows: PackItem[], packed: boolean) {
    if (!list) return;
    const refs = new Set(rows.map((r) => r.item_ref));
    void save(list.items.map((r) => (refs.has(r.item_ref) ? { ...r, packed } : r)));
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

  const kg = (grams: number) => `${(grams / 1000).toFixed(1)} kg`;
</script>

{#if error}
  <p class="pack-error">{error}</p>
{/if}

{#snippet picker(group: string)}
  {@const options = candidates(group)}
  <div class="picker">
    {#if options.length === 0}
      <p class="muted">Everything in {groupLabel(group)} is on the list.</p>
    {/if}
    {#each options as item (item.id)}
      <button type="button" class="pick" onclick={() => add(item)}>
        {#if item.bild}
          <img use:bridgedSrc={interior.mediaUrl(item.bild)} alt="" loading="lazy" />
        {:else}
          <span class="thumb"><Icon name="boxes" size={14} /></span>
        {/if}
        <span class="pick-label">{item.label}</span>
        {#if item.farbe}<small>{item.farbe}</small>{/if}
        <Icon name="plus" size={12} />
      </button>
    {/each}
  </div>
{/snippet}

{#if view && !list}
  <div class="pack-empty">
    <p>No packing list for this trip yet.</p>
    <button type="button" disabled={busy} onclick={() => void createList()}>Create packing list</button>
  </div>
{:else if list}
  <div class="pack-head">
    <strong>{list.name}</strong>
    <span class="pack-count">
      {packedCount} / {list.items.length} packed
      {#if list.total_weight_g}· {kg(list.total_weight_g)}{#if list.weights_missing}{" "}+ {list.weights_missing} without weight{/if}{/if}
    </span>
  </div>
  <div class="pack-progress" aria-hidden="true">
    <span style:width={`${list.items.length ? (packedCount / list.items.length) * 100 : 0}%`}></span>
  </div>

  {#each groups as [group, rows] (group)}
    {@const done = rows.filter((r) => r.packed).length}
    <details class="group" open>
      <summary>
        <Icon name="chevron" size={13} />
        <span class="group-name">{groupLabel(group)}</span>
        <span class="group-count" class:complete={done === rows.length}>{done}/{rows.length}</span>
        <span class="group-actions">
          <button
            type="button"
            class="link"
            onclick={(e) => {
              e.preventDefault();
              packGroup(rows, done !== rows.length);
            }}
          >
            {done === rows.length ? "Unpack all" : "Pack all"}
          </button>
        </span>
      </summary>

      <ul class="cards">
        {#each rows as row (row.item_ref)}
          {@const item = byId.get(row.item_ref)}
          {@const traits = traitsOf(item)}
          {@const open = openRef === row.item_ref}
          <li class="card" class:packed={row.packed} class:open>
            <div class="card-main">
              <input
                type="checkbox"
                checked={row.packed}
                onchange={() => update(row.item_ref, { packed: !row.packed })}
                aria-label={`Packed: ${row.label}`}
              />
              <button
                type="button"
                class="card-body"
                aria-expanded={open}
                onclick={() => (openRef = open ? null : row.item_ref)}
              >
                {#if item?.bild}
                  <img use:bridgedSrc={interior.mediaUrl(item.bild)} alt="" loading="lazy" />
                {:else}
                  <span class="thumb"><Icon name="boxes" size={16} /></span>
                {/if}
                <span class="card-text">
                  <span class="card-label">{row.label ?? item?.label ?? row.item_ref}</span>
                  {#if traits.length}
                    <span class="traits">
                      {#each traits as t (t)}<span class="trait tone-{TONE[t] ?? 'plain'}">{t}</span>{/each}
                    </span>
                  {/if}
                  {#if row.note && !open}<span class="why">{row.note}</span>{/if}
                </span>
              </button>
            </div>

            {#if open}
              <div class="details">
                <dl>
                  {#if item?.farbe}<dt>Colour</dt><dd>{item.farbe}</dd>{/if}
                  {#if item?.groesse}<dt>Size</dt><dd>{item.groesse}</dd>{/if}
                  {#if item?.saison?.length}<dt>Season</dt><dd>{item.saison.join(", ")}</dd>{/if}
                  {#if item?.weight_g}<dt>Weight</dt><dd>{item.weight_g} g</dd>{/if}
                  {#if item?.quelle}<dt>Brand</dt><dd>{item.quelle}</dd>{/if}
                  {#if item?.hinweis}<dt>Note</dt><dd>{item.hinweis}</dd>{/if}
                </dl>
                <label class="why-edit">
                  <span>Why it comes along</span>
                  <textarea
                    rows="2"
                    value={row.note ?? ""}
                    onchange={(e) => update(row.item_ref, { note: e.currentTarget.value.trim() || null })}
                  ></textarea>
                </label>
                <div class="detail-actions">
                  {#if item?.link}
                    <a href={item.link} target="_blank" rel="noreferrer">
                      Product page <Icon name="external" size={11} />
                    </a>
                  {/if}
                  <button type="button" class="link danger" onclick={() => remove(row.item_ref)} use:tip={"Remove from this list"}>
                    Remove from list
                  </button>
                </div>
              </div>
            {/if}
          </li>
        {/each}
      </ul>

      <button type="button" class="add" onclick={() => (addingTo = addingTo === group ? null : group)}>
        <Icon name={addingTo === group ? "close" : "plus"} size={12} />
        {addingTo === group ? "Done" : `Add ${groupLabel(group).toLowerCase()}`}
      </button>
      {#if addingTo === group}{@render picker(group)}{/if}
    </details>
  {/each}

  {#if otherGroups.length}
    <div class="more-groups">
      <span class="muted">Add from</span>
      {#each otherGroups as g (g)}
        <button type="button" class="add" class:active={addingTo === g} onclick={() => (addingTo = addingTo === g ? null : g)}>
          <Icon name="plus" size={12} /> {groupLabel(g)}
        </button>
      {/each}
    </div>
    {#if addingTo && otherGroups.includes(addingTo)}{@render picker(addingTo)}{/if}
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

  .pack-count,
  .muted {
    font-size: var(--text-2xs);
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .pack-progress {
    height: 3px;
    margin: 0.4rem 0 0.5rem;
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

  .group {
    border-top: 1px solid var(--card-border);
    padding: 0.35rem 0 0.5rem;
  }

  summary {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.35rem 0;
    cursor: pointer;
    list-style: none;
  }

  summary::-webkit-details-marker {
    display: none;
  }

  summary :global(svg:first-child) {
    transition: transform var(--motion-fast) ease;
    color: var(--text-secondary);
  }

  details[open] > summary :global(svg:first-child) {
    transform: rotate(90deg);
  }

  .group-name {
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .group-count {
    font-size: var(--text-2xs);
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .group-count.complete {
    color: var(--accent);
  }

  .group-actions {
    margin-left: auto;
  }

  .cards {
    list-style: none;
    margin: 0.25rem 0 0.5rem;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(17rem, 1fr));
    gap: 0.5rem;
    align-items: start;
  }

  .card {
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    min-width: 0;
    transition: border-color var(--motion-fast) ease;
  }

  .card.open {
    border-color: var(--accent);
    grid-column: span 2;
  }

  @media (max-width: 40rem) {
    .card.open {
      grid-column: auto;
    }
  }

  .card.packed .card-text {
    opacity: 0.5;
  }

  .card.packed .card-label {
    text-decoration: line-through;
  }

  .card-main {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    padding: 0.5rem;
  }

  .card-main input {
    margin-top: 0.2rem;
    flex: none;
  }

  .card-body {
    display: flex;
    gap: 0.6rem;
    flex: 1;
    min-width: 0;
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    text-align: left;
    cursor: pointer;
  }

  img,
  .thumb {
    flex: none;
    width: 3.25rem;
    height: 3.25rem;
    border-radius: var(--radius-sm);
    object-fit: cover;
    background: var(--card-bg);
  }

  .thumb {
    display: grid;
    place-items: center;
    color: var(--text-secondary);
  }

  .card-text {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    min-width: 0;
  }

  .card-label {
    font-size: var(--text-sm);
    line-height: 1.25;
  }

  .traits {
    display: flex;
    flex-wrap: wrap;
    gap: 0.2rem;
  }

  .trait {
    font-size: var(--text-2xs);
    padding: 0.05rem 0.4rem;
    border-radius: 999px;
    border: 1px solid var(--card-border);
    color: var(--text-secondary);
  }

  /* Tints mix into the theme's own surface, so they read in light and dark alike. */
  .tone-rain { color: #60a5fa; border-color: color-mix(in srgb, #60a5fa 45%, transparent); }
  .tone-warm { color: #f59e0b; border-color: color-mix(in srgb, #f59e0b 45%, transparent); }
  .tone-style { color: #c084fc; border-color: color-mix(in srgb, #c084fc 45%, transparent); }
  .tone-comfort { color: #34d399; border-color: color-mix(in srgb, #34d399 45%, transparent); }
  .tone-fresh { color: #2dd4bf; border-color: color-mix(in srgb, #2dd4bf 45%, transparent); }
  .tone-essential { color: var(--text-primary); }

  .why {
    font-size: var(--text-2xs);
    color: var(--text-secondary);
    line-height: 1.35;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .details {
    padding: 0 0.75rem 0.75rem 2.4rem;
    display: grid;
    gap: 0.6rem;
  }

  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 0.15rem 0.75rem;
    margin: 0;
    font-size: var(--text-2xs);
  }

  dt {
    color: var(--text-secondary);
  }

  dd {
    margin: 0;
  }

  .why-edit {
    display: grid;
    gap: 0.25rem;
    font-size: var(--text-2xs);
    color: var(--text-secondary);
  }

  .why-edit textarea {
    font: inherit;
    font-size: var(--text-xs, 0.8rem);
    color: var(--text-primary);
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    padding: 0.35rem 0.5rem;
    resize: vertical;
  }

  .detail-actions {
    display: flex;
    gap: 1rem;
    align-items: center;
    font-size: var(--text-2xs);
  }

  .detail-actions a {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    color: var(--accent);
  }

  .link {
    border: 0;
    background: none;
    padding: 0;
    font-size: var(--text-2xs);
    color: var(--text-secondary);
    cursor: pointer;
  }

  .link:hover {
    color: var(--text-primary);
  }

  .link.danger:hover {
    color: var(--danger, #ef4444);
  }

  .add {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.25rem 0.6rem;
    border-radius: 999px;
    border: 1px dashed var(--card-border);
    background: none;
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    cursor: pointer;
  }

  .add:hover,
  .add.active {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .picker {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(13rem, 1fr));
    gap: 0.35rem;
    margin-top: 0.5rem;
  }

  .pick {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.35rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--card-bg);
    color: var(--text-primary);
    text-align: left;
    cursor: pointer;
    min-width: 0;
  }

  .pick:hover {
    border-color: var(--accent);
  }

  .pick img,
  .pick .thumb {
    width: 2.25rem;
    height: 2.25rem;
  }

  .pick-label {
    flex: 1;
    min-width: 0;
    font-size: var(--text-xs, 0.8rem);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pick small {
    font-size: var(--text-2xs);
    color: var(--text-secondary);
  }

  .more-groups {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem;
    border-top: 1px solid var(--card-border);
    padding-top: 0.6rem;
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
