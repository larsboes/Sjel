<script lang="ts">
  // A day, a trip or a person, with everything joined to it in one list (2026-10-08).
  // `lib/context/context.ts` reads the sources; this page lays them out: properties first,
  // then the rows grouped by day. A row opens in the inspector, and from there as a page.
  import { page } from "$app/state";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import StateLine from "$lib/StateLine.svelte";
  import { link } from "$lib/nav";
  import { tip } from "$lib/tip";
  import Section from "$lib/ui/Section.svelte";
  import Collection from "$lib/ui/Collection.svelte";
  import type { Field } from "$lib/ui/collection";
  import { inspectorStore } from "$lib/inspector/inspector.svelte";
  import { goto } from "$app/navigation";
  import {
    dayLabel,
    loadDay,
    loadPerson,
    loadTrip,
    money,
    shiftDay,
    type ContextRow,
    type ContextView,
  } from "$lib/context/context";

  const params = $derived(page.url.searchParams);
  const today = new Date().toLocaleDateString("sv-SE");
  // No parameter means today: the day is the context Home's ranked list already lives in.
  const day = $derived(params.get("trip") || params.get("person") ? null : (params.get("day") ?? today));

  let view = $state<ContextView | null>(null);
  let error = $state<string | null>(null);

  $effect(() => {
    const trip = params.get("trip");
    const person = params.get("person");
    const d = day;
    let live = true;
    view = null;
    error = null;
    const read = trip ? loadTrip(trip) : person ? loadPerson(person) : loadDay(d ?? today);
    read.then(
      (v) => live && (view = v),
      (e) => live && (error = e instanceof Error ? e.message : String(e)),
    );
    return () => {
      live = false;
    };
  });

  const KIND: Record<ContextRow["kind"], string> = { event: "Event", plan: "Plan", spend: "Spend", trip: "Trip" };
  const TONE = { event: "accent", plan: "neutral", spend: "muted", trip: "success" } as const;

  const fields: Field<ContextRow>[] = [
    { id: "time", label: "Time", width: "4rem", value: (r) => r.time, cell: timeCell },
    { id: "title", label: "What", value: (r) => r.title, cell: titleCell },
    { id: "kind", label: "Kind", kind: "select", width: "5.5rem", value: (r) => r.kind, display: (k) => KIND[k as ContextRow["kind"]] ?? k, tone: (k) => TONE[k as ContextRow["kind"]] ?? "neutral" },
    { id: "day", label: "Day", kind: "select", column: false, value: (r) => r.day ?? "", display: (d) => (d ? dayLabel(d) : "No day") },
    { id: "date", label: "Date", kind: "date", column: false, value: (r) => r.day },
    { id: "meta", label: "Detail", width: "12rem", value: (r) => r.meta, cell: metaCell },
    { id: "amount", label: "Amount", kind: "number", width: "7rem", value: (r) => r.amount, cell: amountCell },
  ];

  function open(row: ContextRow) {
    if (row.item) inspectorStore.open(row.item);
    else if (row.href) void goto(row.href);
  }

  /** `[` and `]` step a day context back and forward. */
  function onkey(e: KeyboardEvent) {
    if (!day || e.metaKey || e.ctrlKey || (e.target instanceof Element && e.target.closest("input, textarea, select"))) return;
    if (e.key === "[") void goto(link(`/context?day=${shiftDay(day, -1)}`));
    if (e.key === "]") void goto(link(`/context?day=${shiftDay(day, 1)}`));
  }
</script>

<svelte:window onkeydown={onkey} />

{#snippet timeCell(row: ContextRow)}<span class="mono soft">{row.time ?? ""}</span>{/snippet}
{#snippet titleCell(row: ContextRow)}<span class="title" use:tip={row.title}>{row.title}</span>{/snippet}
{#snippet metaCell(row: ContextRow)}<span class="soft">{row.meta}</span>{/snippet}
{#snippet amountCell(row: ContextRow)}{row.amount !== null && row.currency ? money(row.amount, row.currency) : ""}{/snippet}

{#if error}
  <StateLine state="error" message={error} />
{:else if !view}
  <StateLine state="loading" message="Reading what this touches…" />
{:else}
  <PageHeader badge={view.eyebrow} title={view.title}>
    {#snippet actions()}
      {#if day}
        <nav class="steps" aria-label="Day">
          <a href={link(`/context?day=${shiftDay(day, -1)}`)} use:tip={"Previous day ( [ )"}><Icon name="arrow-left" size={13} /></a>
          {#if day !== today}<a class="today" href={link("/context")}>Today</a>{/if}
          <a href={link(`/context?day=${shiftDay(day, 1)}`)} use:tip={"Next day ( ] )"}><Icon name="arrow-right" size={13} /></a>
        </nav>
      {/if}
    {/snippet}
  </PageHeader>

  {#if view.properties.length}
    <dl class="props">
      {#each view.properties as p, i (p.label + i)}
        <div>
          <dt>{p.label}</dt>
          <dd>{#if p.href}<a href={p.href}>{p.value}</a>{:else}{p.value}{/if}</dd>
        </div>
      {/each}
    </dl>
  {/if}

  {#if view.missing.length}
    <p class="missing"><Icon name="alert" size={12} /> Not answering, so their rows are absent: {view.missing.join(", ")}</p>
  {/if}

  <Section title="Everything joined" count={view.rows.length}>
    <Collection
      id="ctx"
      rows={view.rows}
      {fields}
      key={(r) => r.key}
      title={(r) => r.title}
      defaults={{ group: view.kind === "day" ? null : "day" }}
      onOpen={open}
      inactive={(r) => r.inactive}
      empty={view.kind === "day" ? "Nothing is recorded for this day." : "Nothing else references this yet."}
    />
  </Section>
{/if}

<style>
  .props {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-6);
    margin: 0 0 var(--space-4);
  }

  .props div {
    display: grid;
    gap: 0.1rem;
  }

  dt {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  dd {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-primary);
  }

  dd a {
    color: inherit;
    text-decoration: underline;
    text-decoration-color: var(--rule);
    text-underline-offset: 0.2em;
  }

  dd a:hover {
    text-decoration-color: currentColor;
  }

  .missing {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0 0 var(--space-3);
    font-size: var(--text-2xs);
    color: var(--warning-ink);
  }

  .steps {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  .steps a {
    display: inline-grid;
    place-items: center;
    min-width: 1.6rem;
    height: 1.6rem;
    padding: 0 0.4rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    font-size: var(--text-2xs);
    color: var(--text-secondary);
  }

  .steps a:hover {
    color: var(--text-primary);
    border-color: var(--card-border-hover);
  }

  .steps a:focus-visible {
    outline: 2px solid var(--focus-ring);
  }

  .mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }

  .soft {
    color: var(--text-tertiary);
  }

  .title {
    color: var(--text-primary);
  }
</style>
