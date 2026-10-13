<script lang="ts">
  // Everything captured and not yet sorted: from `sjel capture`, the iOS share sheet or the
  // field below. Rules suggest where an item goes; nothing moves until it is pressed.
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import { tip } from "$lib/tip";
  import PageHeader from "$lib/PageHeader.svelte";
  import Collection from "$lib/ui/Collection.svelte";
  import SidePeek from "$lib/ui/SidePeek.svelte";
  import Property from "$lib/ui/Property.svelte";
  import Chip from "$lib/ui/Chip.svelte";
  import type { Field } from "$lib/ui/collection";
  import { TAGS, captures, label, route, routeFor, type Capture, type Route } from "$lib/capture";

  let rows = $state<Capture[]>([]);
  let error = $state<string | null>(null);
  let draft = $state("");
  let peek = $state<string | null>(null);
  let busy = $state(false);

  const open = $derived(rows.find((r) => r.id === peek) ?? null);
  const routable = $derived(rows.filter((r) => r.status === "new" && routeFor(r)));

  const TAG_TONE = { buy: "warning", look: "accent", go: "success", learn: "neutral" } as const;
  const tagOptions = [{ value: "", label: "None" }, ...TAGS.map((t) => ({ value: t, label: t }))];
  const statusOptions = ["new", "done", "dismissed"].map((s) => ({ value: s, label: s }));

  const fields: Field<Capture>[] = [
    { id: "item", label: "Item", value: label, cell: itemCell },
    {
      id: "tag", label: "Tag", kind: "select", width: "6rem",
      value: (r) => r.tag ?? "untagged",
      tone: (v) => (v in TAG_TONE ? TAG_TONE[v as keyof typeof TAG_TONE] : "muted"),
    },
    { id: "status", label: "Status", kind: "select", width: "6rem", value: (r) => r.status, tone: (v) => (v === "new" ? "accent" : "muted") },
    { id: "created", label: "Captured", kind: "date", width: "7rem", value: (r) => r.created_at.slice(0, 10) },
  ];

  async function load() {
    try {
      rows = await captures.list();
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function replace(next: Capture) {
    rows = rows.map((r) => (r.id === next.id ? next : r));
  }

  async function act(fn: () => Promise<Capture>) {
    busy = true;
    try {
      replace(await fn());
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function add(event: SubmitEvent) {
    event.preventDefault();
    const words = draft.trim().split(/\s+/).filter(Boolean);
    if (!words.length) return;
    const url = words.find((w) => /^https?:\/\//.test(w));
    const text = words.filter((w) => w !== url).join(" ");
    try {
      rows = [await captures.add({ url, text: text || undefined }), ...rows];
      draft = "";
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function applyAll() {
    for (const r of routable) await act(() => route(r, routeFor(r)!));
  }

  const ROUTE_LABEL: Record<Route, string> = { wishlist: "To wishlist", feed: "To feed" };

  onMount(load);
</script>

{#snippet itemCell(r: Capture)}
  <span class="item">
    {label(r)}
    {#if r.url}<a href={r.url} target="_blank" rel="noreferrer" onclick={(e) => e.stopPropagation()}><Icon name="external" size={12} /></a>{/if}
  </span>
{/snippet}

{#snippet rowActions(r: Capture)}
  {#if r.status === "new"}
    {@const to = routeFor(r)}
    {#if to}<button class="icon" disabled={busy} aria-label={ROUTE_LABEL[to]} use:tip={ROUTE_LABEL[to]} onclick={() => act(() => route(r, to))}><Icon name="arrow-right" size={13} /></button>{/if}
    <button class="icon" disabled={busy} aria-label="Done" use:tip={"Done"} onclick={() => act(() => captures.patch(r.id, { status: "done" }))}><Icon name="check" size={13} /></button>
    <button class="icon" disabled={busy} aria-label="Dismiss" use:tip={"Dismiss"} onclick={() => act(() => captures.patch(r.id, { status: "dismissed" }))}><Icon name="close" size={13} /></button>
  {/if}
{/snippet}

<PageHeader badge="Inbox" title="Captured, not sorted yet">
  {#snippet actions()}
    {#if routable.length}
      <button class="act" disabled={busy} onclick={applyAll}>Apply {routable.length} suggestions</button>
    {/if}
  {/snippet}
</PageHeader>

<form class="capture" onsubmit={add}>
  <Icon name="plus" size={14} />
  <input bind:value={draft} placeholder="A link or a thought. #buy #look #go #learn sort it." aria-label="Capture" />
</form>

{#if error}<p class="err"><Icon name="alert" size={13} /> {error}</p>{/if}

<Collection
  id="inbox"
  {rows}
  {fields}
  key={(r) => r.id}
  title={label}
  defaults={{ filter: { status: ["new"] } }}
  onOpen={(r) => (peek = r.id)}
  inactive={(r) => r.status !== "new"}
  selected={peek}
  actions={rowActions}
  empty="Nothing waiting."
/>

{#if open}
  <SidePeek title={label(open)} eyebrow={open.tag ? `#${open.tag}` : "Untagged"} onClose={() => (peek = null)}>
    <Property label="Text" kind="textarea" value={open.text} onCommit={(v) => act(() => captures.patch(open.id, { text: v ?? "" }))} />
    <Property label="Tag" kind="select" value={open.tag ?? ""} options={tagOptions} onCommit={(v) => act(() => captures.patch(open.id, { tag: (v ?? "") as Capture["tag"] }))} />
    <Property label="Status" kind="select" value={open.status} options={statusOptions} onCommit={(v) => v && act(() => captures.patch(open.id, { status: v as Capture["status"] }))} />
    {#if open.url}<Property label="Link" value={open.url} readonly />{/if}
    {#if open.routed_to}<Property label="Went to" value={open.routed_to} readonly />{/if}
    <Property label="From" value={open.client} readonly />
    {#if routeFor(open)}
      <Chip label={`Suggested: ${ROUTE_LABEL[routeFor(open)!].toLowerCase()}`} tone="accent" />
    {/if}
  </SidePeek>
{/if}

<style>
  .capture {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 0.7rem;
    margin: 0 0 1rem;
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
  }
  .capture input {
    flex: 1;
    border: 0;
    background: transparent;
    font: inherit;
    color: var(--text-primary);
    outline: none;
  }
  .item { display: inline-flex; align-items: center; gap: 0.35rem; }
  .item a { color: var(--text-tertiary); }
  .act {
    font-size: var(--text-2xs);
    padding: 0.15rem 0.45rem;
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text-secondary);
    cursor: pointer;
  }
  :global(td.row-actions:has(.icon)) { white-space: nowrap; }
  .icon { border: 0; background: transparent; color: var(--text-tertiary); cursor: pointer; padding: 0.15rem; }
  .icon:hover, .act:hover { color: var(--text-primary); }
  .err { display: flex; align-items: center; gap: 0.4rem; font-size: 0.8rem; color: var(--warning-ink); margin: 0 0 0.8rem; }
</style>
