<script lang="ts">
  // A trip read stage by stage (2026-10-07): one band per leg with its days, its train and
  // its stay, and the items in a table. The same items also show as a board (one column per
  // day) or the timeline. A row opens in a side peek, where every property edits in place.
  import type { Snippet } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import { tip } from "$lib/tip";
  import { trips, type PlanItem, type TripPlan, type TripStage, type TransportMode } from "$lib/api";
  import Section from "$lib/ui/Section.svelte";
  import Chip from "$lib/ui/Chip.svelte";
  import DataTable, { type Column } from "$lib/ui/DataTable.svelte";
  import ViewSwitch from "$lib/ui/ViewSwitch.svelte";
  import SidePeek from "$lib/ui/SidePeek.svelte";
  import Property from "$lib/ui/Property.svelte";
  import { bandsFor, itemBlock, itemCost, itemInactive, itemStatus, itemTime, stageTone } from "./stages";

  let {
    plan,
    items,
    modes,
    stageLabel,
    onItemChanged,
    onRemoveItem,
    onUpdateStage,
    onToggleMode,
    timeline,
  }: {
    plan: TripPlan;
    items: PlanItem[];
    modes: { id: TransportMode; label: string }[];
    stageLabel: (status: TripStage["status"]) => string;
    onItemChanged: (item: PlanItem) => void;
    onRemoveItem: (item: PlanItem) => void;
    onUpdateStage: (stageId: string, patch: Partial<TripStage>) => void;
    onToggleMode: (stageId: string, mode: TransportMode) => void;
    timeline: Snippet;
  } = $props();

  type View = "stages" | "board" | "timeline";
  let view = $state<View>("stages");
  let peekItem = $state<string | null>(null);
  let peekStage = $state<string | null>(null);
  let error = $state<string | null>(null);

  const trip = $derived(bandsFor(plan, items));
  const openItem = $derived(items.find((i) => i.id === peekItem) ?? null);
  const openStage = $derived(plan.stages.find((s) => s.id === peekStage) ?? null);
  const allDays = $derived(trip.bands.flatMap((b) => b.days).filter((d, i, a) => a.indexOf(d) === i));

  const dayLabel = (day: string) =>
    new Date(`${day}T12:00:00`).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" });
  const rangeLabel = (days: string[]) =>
    days.length === 1 ? dayLabel(days[0]) : `${dayLabel(days[0])} – ${dayLabel(days[days.length - 1])}`;
  const payload = (item: PlanItem) =>
    (item.payload && typeof item.payload === "object" ? item.payload : {}) as Record<string, unknown>;
  const text = (v: unknown) => (typeof v === "string" ? v : v == null ? null : String(v));

  const TYPE: Record<string, string> = {
    journey: "Rail", transport: "Transport", event: "Event", activity: "Activity", place: "Place",
    stay: "Stay", image: "Image", note: "Note", option_set: "Options", booking: "Booking", outcome: "Outcome",
  };
  const BLOCKS = [
    { value: "", label: "—" }, { value: "morning", label: "Morning" }, { value: "day", label: "All day" },
    { value: "afternoon", label: "Afternoon" }, { value: "evening", label: "Evening" },
  ];
  const STATUSES = [
    { value: "proposed", label: "Proposed" }, { value: "planned", label: "Planned" },
    { value: "booked", label: "Booked" }, { value: "done", label: "Done" }, { value: "dropped", label: "Dropped" },
  ];
  const STAGE_STATUSES: TripStage["status"][] = ["open", "planning", "option_selected", "booked", "completed"];

  const VIEWS = [
    { id: "stages" as const, label: "Stages", icon: "layout" as const },
    { id: "board" as const, label: "Board", icon: "boxes" as const },
    { id: "timeline" as const, label: "Timeline", icon: "calendar" as const },
  ];

  const columns: Column<PlanItem>[] = [
    { id: "time", label: "Time", width: "5rem", cell: timeCell },
    { id: "title", label: "What", cell: titleCell },
    { id: "type", label: "Type", width: "5.5rem", cell: typeCell },
    { id: "status", label: "Status", width: "6rem", cell: statusCell },
    { id: "cost", label: "Cost", width: "5.5rem", align: "end", cell: costCell },
  ];

  async function edit(item: PlanItem, change: { title?: string; payload?: Record<string, unknown>; day?: string | null }) {
    error = null;
    try {
      const updated =
        "day" in change
          ? await trips.setItemDay(plan.id, item.id, change.day ?? null)
          : await trips.editItem(plan.id, item.id, change);
      onItemChanged(updated);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  const open = (item: PlanItem) => {
    peekStage = null;
    peekItem = item.id;
  };

  /** The leg's own train and the night's bed, for the band's header line. */
  const facts = (bandItems: PlanItem[]) => ({
    travel: bandItems.filter((i) => i.item_type === "booking" || i.item_type === "transport" || i.item_type === "journey"),
    stay: bandItems.filter((i) => i.item_type === "stay"),
  });
</script>

{#snippet timeCell(item: PlanItem)}
  {@const time = itemTime(item)}
  {#if time}<span class="mono">{time}</span>{:else}<span class="soft">{BLOCKS.find((b) => b.value === itemBlock(item))?.label ?? ""}</span>{/if}
{/snippet}
{#snippet titleCell(item: PlanItem)}
  {@const why = text(payload(item).why)}
  {#if why}<span class="title" use:tip={why}>{item.title}</span>{:else}<span class="title">{item.title}</span>{/if}
{/snippet}
{#snippet typeCell(item: PlanItem)}<span class="soft">{TYPE[item.item_type]}</span>{/snippet}
{#snippet statusCell(item: PlanItem)}
  {@const s = itemStatus(item)}<Chip label={s.label} tone={s.tone} />
{/snippet}
{#snippet costCell(item: PlanItem)}{itemCost(item) ?? ""}{/snippet}
{#snippet rowActions(item: PlanItem)}
  <button type="button" class="icon-btn" onclick={() => onRemoveItem(item)} use:tip={`Remove "${item.title}"`}>
    <Icon name="close" size={12} />
  </button>
{/snippet}

<Section title="Itinerary" count={items.length} meta={`${trip.bands.length} stages`}>
  {#snippet actions()}<ViewSwitch views={VIEWS} bind:value={view} label="Itinerary view" />{/snippet}

  {#if error}<p class="error">{error}</p>{/if}

  {#if view === "stages"}
    <div class="bands">
      {#each trip.bands as band, index (band.stage.id)}
        {@const f = facts(band.items)}
        <div class="band">
          <Section
            title={band.stage.destination.name}
            meta={`${rangeLabel(band.days)} · from ${band.stage.origin.name}`}
            count={band.items.length}
            collapsible
          >
            {#snippet lead()}
              <span class="num mono">{String(index + 1).padStart(2, "0")}</span>
            {/snippet}
            {#snippet actions()}
              <Chip label={stageLabel(band.stage.status)} tone={stageTone(band.stage.status)} />
              <button type="button" class="link" onclick={() => { peekItem = null; peekStage = band.stage.id; }}>Leg</button>
            {/snippet}
            {#if f.travel.length || f.stay.length}
              <div class="facts">
                {#each f.travel as t (t.id)}
                  <button type="button" class="fact" onclick={() => open(t)}><Icon name="train" size={12} />{t.title}</button>
                {/each}
                {#each f.stay as s (s.id)}
                  <button type="button" class="fact" onclick={() => open(s)}><Icon name="home" size={12} />{s.title}</button>
                {/each}
              </div>
            {/if}
            <DataTable
              rows={band.items}
              {columns}
              key={(i) => i.id}
              group={(i) => (i.day ? dayLabel(i.day) : "No day")}
              onOpen={open}
              inactive={itemInactive}
              selected={peekItem}
              actions={rowActions}
              empty="Nothing planned for this stage yet."
            />
          </Section>
        </div>
      {/each}

      {#if trip.outside.length}
        <div class="band warn">
          <Section title="Before the trip starts" count={trip.outside.length} collapsible>
            {#snippet lead()}<span use:tip={"Dated before the first stage. Move them to a trip day, or remove them."}><Icon name="alert" size={13} /></span>{/snippet}
            <DataTable rows={trip.outside} {columns} key={(i) => i.id} group={(i) => dayLabel(i.day!)} onOpen={open} selected={peekItem} actions={rowActions} />
          </Section>
        </div>
      {/if}
      {#if trip.undated.length}
        <div class="band">
          <Section title="No day yet" count={trip.undated.length} collapsible open={false}>
            <DataTable rows={trip.undated} {columns} key={(i) => i.id} onOpen={open} selected={peekItem} actions={rowActions} />
          </Section>
        </div>
      {/if}
    </div>
  {:else if view === "board"}
    <div class="board">
      {#each allDays as day (day)}
        {@const dayItems = trip.bands.flatMap((b) => b.items).filter((i) => i.day === day)}
        <div class="column">
          <header>
            <strong>{dayLabel(day)}</strong>
            <span class="mono soft">{dayItems.length}</span>
          </header>
          {#each dayItems as item (item.id)}
            {@const s = itemStatus(item)}
            <button type="button" class="card" class:selected={peekItem === item.id} class:inactive={itemInactive(item)} onclick={() => open(item)}>
              <span class="card-top">{@render timeCell(item)}<Chip label={s.label} tone={s.tone} /></span>
              <span class="card-title">{item.title}</span>
            </button>
          {/each}
        </div>
      {/each}
    </div>
  {:else}
    {@render timeline()}
  {/if}
</Section>

{#if openItem}
  {@const p = payload(openItem)}
  {@const item = openItem}
  <SidePeek title={item.title} eyebrow={`${TYPE[item.item_type]}${item.day ? ` · ${dayLabel(item.day)}` : ""}`} onClose={() => (peekItem = null)}>
    <Property label="Title" value={item.title} onCommit={(v) => v && edit(item, { title: v })} />
    <Property
      label="Day"
      kind="select"
      value={item.day}
      options={[{ value: "", label: "No day" }, ...allDays.map((d) => ({ value: d, label: dayLabel(d) }))]}
      onCommit={(v) => edit(item, { day: v })}
    />
    {#if item.item_type !== "booking"}
      <Property label="Time" kind="time" value={text(p.time)} hint="A clock time sorts the item within its day." onCommit={(v) => edit(item, { payload: { time: v } })} />
      <Property label="Part of day" kind="select" value={text(p.block)} options={BLOCKS} onCommit={(v) => edit(item, { payload: { block: v } })} />
      <Property label="Status" kind="select" value={text(p.status)} options={STATUSES} onCommit={(v) => edit(item, { payload: { status: v } })} />
      <Property label="Why" kind="textarea" value={text(p.why ?? p.text)} onCommit={(v) => edit(item, { payload: { [p.text !== undefined && p.why === undefined ? "text" : "why"]: v } })} />
    {:else}
      <Property label="Departs" value={text(p.departure)?.replace("T", " ") ?? null} readonly />
      <Property label="Arrives" value={text(p.arrival)?.replace("T", " ") ?? null} readonly />
    {/if}
    {#each ["provider", "order_ref", "fare", "room", "check_in", "check_out", "notes", "venue"] as field (field)}
      {#if text(p[field])}<Property label={field.replace("_", " ")} value={text(p[field])} readonly />{/if}
    {/each}
    {#if itemCost(item)}<Property label="Cost" value={itemCost(item)} readonly />{/if}
    {#snippet footer()}
      <button type="button" class="link danger" onclick={() => { onRemoveItem(item); peekItem = null; }}>Remove from trip</button>
    {/snippet}
  </SidePeek>
{/if}

{#if openStage}
  {@const stage = openStage}
  <SidePeek title={`${stage.origin.name} → ${stage.destination.name}`} eyebrow="Leg" onClose={() => (peekStage = null)}>
    <Property label="Day" kind="select" value={stage.date} options={allDays.concat(plan.date_end).filter((d, i, a) => a.indexOf(d) === i).map((d) => ({ value: d, label: dayLabel(d) }))} onCommit={(v) => v && onUpdateStage(stage.id, { date: v })} />
    <Property label="Status" kind="select" value={stage.status} options={STAGE_STATUSES.map((s) => ({ value: s, label: stageLabel(s) }))} onCommit={(v) => v && onUpdateStage(stage.id, { status: v as TripStage["status"] })} />
    <Property
      label="Travellers"
      value={stage.travelers.join(", ") || null}
      placeholder="Everyone on the trip"
      onCommit={(v) => onUpdateStage(stage.id, { travelers: (v ?? "").split(",").map((t) => t.trim()).filter(Boolean) })}
    />
    <div class="modes" role="group" aria-label="Transport modes">
      {#each modes as mode (mode.id)}
        <button type="button" class:active={stage.transport_modes.includes(mode.id)} aria-pressed={stage.transport_modes.includes(mode.id)} onclick={() => onToggleMode(stage.id, mode.id)}>
          {mode.label}
        </button>
      {/each}
    </div>
  </SidePeek>
{/if}

<style>
  .bands {
    display: grid;
    gap: var(--space-4);
  }

  .band {
    padding-left: var(--space-3);
    border-left: 2px solid var(--card-border);
  }

  .band.warn {
    border-left-color: var(--warning);
  }

  .num {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
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

  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .fact {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    max-width: 100%;
    padding: 0.2rem 0.5rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--card-bg);
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    cursor: pointer;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .fact:hover {
    border-color: var(--card-border-hover);
    color: var(--text-primary);
  }

  .board {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: minmax(13rem, 1fr);
    gap: var(--space-2);
    overflow-x: auto;
    padding-bottom: var(--space-2);
  }

  .column {
    display: grid;
    align-content: start;
    gap: var(--space-1);
    min-width: 0;
  }

  .column header {
    display: flex;
    justify-content: space-between;
    font-size: var(--text-2xs);
    padding: 0 0.15rem var(--space-1);
    border-bottom: 1px solid var(--rule);
  }

  .card {
    display: grid;
    gap: 0.3rem;
    padding: 0.45rem 0.5rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--card-bg);
    text-align: left;
    color: var(--text-primary);
    font: inherit;
    cursor: pointer;
  }

  .card:hover,
  .card.selected {
    border-color: var(--card-border-hover);
    background: var(--nav-hover);
  }

  .card.inactive {
    opacity: 0.55;
  }

  .card-top {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: var(--text-2xs);
  }

  .card-title {
    font-size: var(--text-xs);
    line-height: var(--leading-tight);
  }

  .icon-btn {
    display: inline-grid;
    place-items: center;
    width: 1.4rem;
    height: 1.4rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-tertiary);
    cursor: pointer;
  }

  .icon-btn:hover {
    background: var(--danger-soft);
    color: var(--danger);
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
    color: var(--danger);
  }

  .modes {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    padding-left: calc(7rem + var(--space-2));
  }

  .modes button {
    padding: 0.15rem 0.5rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    cursor: pointer;
  }

  .modes button.active {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent);
  }

  .error {
    margin: 0;
    color: var(--danger);
    font-size: var(--text-2xs);
  }
</style>
