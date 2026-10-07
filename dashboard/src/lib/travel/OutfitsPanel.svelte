<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import { bridgedSrc } from "$lib/bridged-url";
  import { interior, trips, type InteriorItem, type Outfit, type OutfitInput } from "$lib/api";

  let { planId, dateStart, dateEnd }: { planId: string; dateStart: string; dateEnd: string } =
    $props();

  let outfits = $state<Outfit[]>([]);
  let wardrobe = $state<InteriorItem[]>([]);
  let error = $state<string | null>(null);
  let loaded = $state(false);
  /** Index of the outfit being edited, and whether its piece picker is open. */
  let editing = $state<number | null>(null);
  let picking = $state(false);

  // What is worn: clothes and shoes. Bags and chargers are packed, never worn.
  const WEARABLE = new Set(["kleidung", "schuhe"]);

  const byId = $derived(new Map(wardrobe.map((item) => [item.id, item])));

  const days = $derived.by(() => {
    const out: string[] = [];
    const end = new Date(`${dateEnd}T12:00:00`);
    for (let d = new Date(`${dateStart}T12:00:00`); d <= end && out.length < 60; d.setDate(d.getDate() + 1)) {
      out.push(d.toISOString().slice(0, 10));
    }
    return out;
  });

  const dayLabel = (day: string | null) =>
    day
      ? new Date(`${day}T12:00:00`).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" })
      : "Any day";

  async function load(id: string) {
    error = null;
    try {
      const [res, rows] = await Promise.all([
        trips.outfits(id),
        interior.inventory().catch(() => []),
      ]);
      outfits = res.outfits;
      wardrobe = rows
        .filter((row) => row.state === "owned" && WEARABLE.has(row.item.category ?? ""))
        .map((row) => row.item);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loaded = true;
    }
  }

  $effect(() => {
    void load(planId);
  });

  async function save(next: OutfitInput[]) {
    const before = outfits;
    outfits = next.map((o) => ({ ...o, not_on_pack_list: [] }));
    try {
      await trips.putOutfits(
        planId,
        next.map(({ name, day, pieces, note, proposed }) => ({ name, day, pieces, note, proposed })),
      );
      // Re-read: which pieces are missing from the pack list is the server's answer.
      outfits = (await trips.outfits(planId)).outfits;
    } catch (e) {
      outfits = before;
      error = e instanceof Error ? e.message : String(e);
    }
  }

  const change = (index: number, patch: Partial<OutfitInput>) =>
    void save(outfits.map((o, i) => (i === index ? { ...o, ...patch } : o)));

  function addOutfit() {
    const day = days.find((d) => !outfits.some((o) => o.day === d)) ?? null;
    void save([...outfits, { name: "New outfit", day, pieces: [], note: null, not_on_pack_list: [] }]);
    editing = outfits.length;
    picking = true;
  }

  function removeOutfit(index: number) {
    editing = null;
    void save(outfits.filter((_, i) => i !== index));
  }

  const candidates = (outfit: OutfitInput) =>
    wardrobe
      .filter((item) => !outfit.pieces.includes(item.id))
      .sort((a, b) => (a.category ?? "").localeCompare(b.category ?? "") || a.label.localeCompare(b.label));
</script>

{#snippet thumb(item: InteriorItem | undefined, ref: string, size: string)}
  {#if item?.bild}
    <img use:bridgedSrc={interior.mediaUrl(item.bild)} alt={item.label} title={item.label} loading="lazy" style:width={size} style:height={size} />
  {:else}
    <span class="thumb" title={item?.label ?? ref} style:width={size} style:height={size}><Icon name="boxes" size={14} /></span>
  {/if}
{/snippet}

{#if error}<p class="error">{error}</p>{/if}

{#if loaded && outfits.length === 0}
  <p class="muted">No outfits yet. Plan what you wear each day, and the pack list is checked against it.</p>
{/if}

<ul class="outfits">
  {#each outfits as outfit, index (index)}
    {@const isEditing = editing === index}
    <li class="outfit" class:editing={isEditing}>
      <div class="head">
        {#if isEditing}
          <input
            class="name-input"
            value={outfit.name}
            aria-label="Outfit name"
            onchange={(e) => change(index, { name: e.currentTarget.value.trim() || outfit.name })}
          />
          <select
            value={outfit.day ?? ""}
            aria-label="Day"
            onchange={(e) => change(index, { day: e.currentTarget.value || null })}
          >
            <option value="">Any day</option>
            {#each days as d (d)}<option value={d}>{dayLabel(d)}</option>{/each}
          </select>
        {:else}
          <span class="day">{dayLabel(outfit.day)}</span>
          <strong>{outfit.name}</strong>
          {#if outfit.proposed}<span class="proposed">Proposed</span>{/if}
        {/if}
        <button
          type="button"
          class="link"
          onclick={() => {
            editing = isEditing ? null : index;
            picking = false;
          }}
        >
          {isEditing ? "Done" : "Edit"}
        </button>
      </div>

      <ul class="pieces">
        {#each outfit.pieces as ref (ref)}
          {@const item = byId.get(ref)}
          <li class="piece" class:missing={outfit.not_on_pack_list.includes(ref)}>
            {@render thumb(item, ref, "3.5rem")}
            <span class="piece-label">{item?.label ?? ref}</span>
            {#if isEditing}
              <button
                type="button"
                class="drop"
                aria-label={`Remove ${item?.label ?? ref}`}
                onclick={() => change(index, { pieces: outfit.pieces.filter((p) => p !== ref) })}
              >
                <Icon name="close" size={10} />
              </button>
            {/if}
          </li>
        {/each}
        {#if isEditing}
          <li>
            <button type="button" class="add-piece" onclick={() => (picking = !picking)} aria-expanded={picking}>
              <Icon name={picking ? "close" : "plus"} size={14} />
            </button>
          </li>
        {/if}
      </ul>

      {#if outfit.proposed && !isEditing}
        <div class="decide">
          <button type="button" class="link accept" onclick={() => change(index, { proposed: false })}>Accept</button>
          <button type="button" class="link danger" onclick={() => removeOutfit(index)}>Discard</button>
        </div>
      {/if}

      {#if outfit.not_on_pack_list.length}
        <p class="warn">
          Not on the pack list:
          {outfit.not_on_pack_list.map((ref) => byId.get(ref)?.label ?? ref).join(", ")}
        </p>
      {/if}

      {#if isEditing}
        <textarea
          rows="2"
          placeholder="Why this, for this day"
          value={outfit.note ?? ""}
          onchange={(e) => change(index, { note: e.currentTarget.value.trim() || null })}
        ></textarea>
        {#if picking}
          <div class="picker">
            {#each candidates(outfit) as item (item.id)}
              <button type="button" class="pick" onclick={() => change(index, { pieces: [...outfit.pieces, item.id] })}>
                {@render thumb(item, item.id, "2.25rem")}
                <span>{item.label}</span>
              </button>
            {/each}
          </div>
        {/if}
        <button type="button" class="link danger" onclick={() => removeOutfit(index)}>Delete outfit</button>
      {:else if outfit.note}
        <p class="note">{outfit.note}</p>
      {/if}
    </li>
  {/each}
</ul>

<button type="button" class="add" onclick={addOutfit}><Icon name="plus" size={12} /> New outfit</button>

<style>
  .outfits {
    list-style: none;
    margin: 0 0 0.75rem;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(22rem, 1fr));
    gap: 0.6rem;
  }

  .outfit {
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    padding: 0.6rem 0.7rem;
    display: grid;
    gap: 0.5rem;
    align-content: start;
  }

  .outfit.editing {
    border-color: var(--accent);
  }

  .proposed {
    font-size: var(--text-2xs);
    color: var(--text-secondary);
    border: 1px dashed var(--card-border);
    border-radius: 999px;
    padding: 0 0.4rem;
  }

  .decide {
    display: flex;
    gap: 0.8rem;
  }

  .link.accept:hover {
    color: var(--accent);
  }

  .head {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
  }

  .head strong {
    font-size: var(--text-sm);
    flex: 1;
    min-width: 0;
  }

  .day {
    font-size: var(--text-2xs);
    color: var(--accent);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  .name-input,
  select,
  textarea {
    font: inherit;
    font-size: var(--text-xs, 0.8rem);
    color: var(--text-primary);
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    padding: 0.25rem 0.45rem;
  }

  .name-input {
    flex: 1;
    min-width: 0;
  }

  textarea {
    resize: vertical;
  }

  .pieces {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }

  .piece {
    position: relative;
    width: 3.5rem;
    display: grid;
    gap: 0.15rem;
  }

  .piece.missing img,
  .piece.missing .thumb {
    outline: 2px solid #f59e0b;
    outline-offset: 1px;
  }

  .piece-label {
    font-size: 0.6rem;
    line-height: 1.15;
    color: var(--text-secondary);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  img,
  .thumb {
    border-radius: var(--radius-sm);
    object-fit: cover;
    background: var(--card-bg);
    flex: none;
  }

  .thumb {
    display: grid;
    place-items: center;
    color: var(--text-secondary);
  }

  .drop {
    position: absolute;
    top: -0.3rem;
    right: -0.3rem;
    width: 1.1rem;
    height: 1.1rem;
    display: grid;
    place-items: center;
    border-radius: 999px;
    border: 1px solid var(--card-border);
    background: var(--card-bg);
    color: var(--text-secondary);
    cursor: pointer;
    padding: 0;
  }

  .add-piece {
    width: 3.5rem;
    height: 3.5rem;
    display: grid;
    place-items: center;
    border: 1px dashed var(--card-border);
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-secondary);
    cursor: pointer;
  }

  .add-piece:hover {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .note,
  .muted {
    margin: 0;
    font-size: var(--text-2xs);
    color: var(--text-secondary);
    line-height: 1.35;
  }

  .warn {
    margin: 0;
    font-size: var(--text-2xs);
    color: #f59e0b;
  }

  .picker {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(11rem, 1fr));
    gap: 0.3rem;
    max-height: 16rem;
    overflow: auto;
  }

  .pick {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    padding: 0.25rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--card-bg);
    color: var(--text-primary);
    font-size: var(--text-2xs);
    text-align: left;
    cursor: pointer;
    min-width: 0;
  }

  .pick span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pick:hover {
    border-color: var(--accent);
  }

  .link {
    border: 0;
    background: none;
    padding: 0;
    font-size: var(--text-2xs);
    color: var(--text-secondary);
    cursor: pointer;
    justify-self: start;
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

  .add:hover {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .error {
    color: var(--danger, #ef4444);
    font-size: var(--text-2xs);
  }
</style>
