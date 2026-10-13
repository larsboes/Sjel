<script lang="ts">
  // The wishlist as one collection: table or board, grouped by category, edited in a side peek.
  // Every edit goes through `onPatch`, which is the page's offline-aware `interior.patchItem`.
  import Icon from "$lib/Icon.svelte";
  import { tip } from "$lib/tip";
  import type { InteriorItem } from "$lib/api";
  import Chip from "$lib/ui/Chip.svelte";
  import Collection from "$lib/ui/Collection.svelte";
  import SidePeek from "$lib/ui/SidePeek.svelte";
  import Property from "$lib/ui/Property.svelte";
  import type { Field } from "$lib/ui/collection";
  import { centsFrom, euro } from "./money";

  let {
    items,
    pending,
    conflicts,
    busy = false,
    onPatch,
    onBought,
    onFullEditor,
  }: {
    items: InteriorItem[];
    pending: Set<string>;
    conflicts: Set<string>;
    busy?: boolean;
    onPatch: (item: InteriorItem, patch: Partial<InteriorItem>) => Promise<void>;
    onBought: (id: string) => void;
    onFullEditor: (id: string) => void;
  } = $props();

  let peek = $state<string | null>(null);
  let error = $state<string | null>(null);
  const open = $derived(items.find((i) => i.id === peek) ?? null);

  const price = (i: InteriorItem) => i.preis_cent ?? i.kosten_min_cent;

  const fields: Field<InteriorItem>[] = [
    { id: "label", label: "Item", value: (i) => i.label, cell: labelCell },
    { id: "category", label: "Category", kind: "select", width: "8rem", value: (i) => i.category ?? "none", display: (v) => (v === "none" ? "No category" : v), tone: (v) => (v === "none" ? "muted" : "neutral") },
    { id: "priority", label: "Priority", kind: "select", width: "8rem", value: (i) => i.prioritaet ?? "none", display: (v) => (v === "none" ? "—" : v), tone: (v) => (v === "pflicht" ? "warning" : v === "none" ? "muted" : "neutral") },
    { id: "price", label: "Price", kind: "number", width: "9rem", align: "end", value: price, cell: priceCell },
    { id: "kind", label: "Kind", kind: "select", column: false, value: (i) => i.kind, display: (v) => (v === "slot" ? "Flat need" : v === "gear" ? "Gear" : "Thing") },
  ];

  async function patch(item: InteriorItem, change: Partial<InteriorItem>) {
    error = null;
    try {
      await onPatch(item, change);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    }
  }

  function commitPrice(item: InteriorItem, text: string | null) {
    const cents = centsFrom(text);
    if (cents === undefined) error = `"${text}" is not a price. Write it like 39,90.`;
    else void patch(item, { preis_cent: cents });
  }
</script>

{#snippet labelCell(i: InteriorItem)}
  <span class="label">
    {i.label}
    {#if i.link}<a href={i.link} target="_blank" rel="noreferrer" aria-label="Product page" onclick={(e) => e.stopPropagation()}><Icon name="external" size={12} /></a>{/if}
    {#if pending.has(i.id)}<span use:tip={"Saved on this device; not on the canonical node yet."}><Chip label="not synced" tone="warning" /></span>{/if}
    {#if conflicts.has(i.id)}<span use:tip={"The canonical node changed this item too. Review it in the sync line at the top."}><Chip label="conflict" tone="danger" /></span>{/if}
  </span>
{/snippet}

{#snippet priceCell(i: InteriorItem)}
  {#if i.preis_cent !== null}
    <span class="mono">{euro(i.preis_cent)}</span>
  {:else if i.kosten_min_cent !== null}
    <span class="mono">{euro(i.kosten_min_cent)}–{euro(i.kosten_max_cent ?? i.kosten_min_cent)}</span>
  {:else}
    <Chip label="no price" tone="muted" />
  {/if}
{/snippet}

{#snippet rowActions(i: InteriorItem)}
  <button class="icon" disabled={busy} aria-label="Mark bought" use:tip={"Mark bought"} onclick={() => onBought(i.id)}><Icon name="check" size={13} /></button>
{/snippet}

{#snippet errorLine()}{#if error}<p class="err"><Icon name="alert" size={13} /> {error}</p>{/if}{/snippet}

{#if !open}{@render errorLine()}{/if}

<Collection
  id="wish"
  rows={items}
  {fields}
  key={(i) => i.id}
  title={(i) => i.label}
  defaults={{ group: "category" }}
  onOpen={(i) => (peek = i.id)}
  selected={peek}
  actions={rowActions}
  empty="Nothing wanted."
/>

{#if open}
  {@const i = open}
  <SidePeek title={i.label} eyebrow={i.kind === "slot" ? "Flat need" : "Wanted"} onClose={() => { peek = null; error = null; }}>
    {@render errorLine()}
    <Property label="Label" value={i.label} onCommit={(v) => v && patch(i, { label: v })} />
    <Property label="Price" value={i.preis_cent === null ? null : euro(i.preis_cent)} placeholder="39,90" onCommit={(v) => commitPrice(i, v)} />
    <Property label="Category" value={i.category} placeholder="kleidung" onCommit={(v) => patch(i, { category: v })} />
    <Property label="Priority" value={i.prioritaet} placeholder="pflicht, empfehlung, kandidat" onCommit={(v) => patch(i, { prioritaet: v })} />
    <Property label="Size" value={i.groesse} onCommit={(v) => patch(i, { groesse: v })} />
    <Property label="Colour" value={i.farbe} onCommit={(v) => patch(i, { farbe: v })} />
    <Property label="Link" value={i.link} placeholder="https://…" onCommit={(v) => patch(i, { link: v })} />
    <Property label="Note" kind="textarea" value={i.hinweis} onCommit={(v) => patch(i, { hinweis: v })} />
    {#if i.ziel}<Property label="Goal" value={i.ziel} readonly />{/if}
    {#snippet footer()}
      <button class="act" disabled={busy} onclick={() => { onBought(i.id); peek = null; }}>Mark bought</button>
      <button class="act" onclick={() => { onFullEditor(i.id); peek = null; }}>Sizes, picture, history…</button>
    {/snippet}
  </SidePeek>
{/if}

<style>
  .label { display: inline-flex; align-items: center; gap: 0.35rem; flex-wrap: wrap; }
  .label a { color: var(--text-tertiary); }
  .mono { font-family: var(--font-mono); }
  .icon { border: 0; background: transparent; color: var(--text-tertiary); cursor: pointer; padding: 0.15rem; }
  .icon:hover, .act:hover { color: var(--text-primary); }
  .act {
    font-size: var(--text-xs);
    padding: 0.25rem 0.55rem;
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text-secondary);
    cursor: pointer;
  }
  .err { display: flex; align-items: center; gap: 0.4rem; font-size: 0.8rem; color: var(--warning-ink); margin: 0 0 0.8rem; }
</style>
