<script lang="ts">
  import { onMount } from "svelte";
  import { eventItem } from "$lib/inspector/connections";
  import { inspectorStore } from "$lib/inspector/inspector.svelte";
  import { kindConfig } from "$lib/calendar/types";
  import { link } from "$lib/nav";
  import Icon from "$lib/Icon.svelte";
  import Chip, { type Tone } from "$lib/ui/Chip.svelte";
  import { tip } from "$lib/tip";
  import type { CalendarEntry, TripPlan, TripStage } from "$lib/api";
  import { localDateKey } from "./format";

  /// The trip you are on, read as today and tomorrow. While a trip runs, the
  /// question on opening Home is "what happens next and is it sorted", and the
  /// answer is spread over trips (the legs) and calendar (tickets, meetings).
  /// This puts both on one timeline per day.
  let { plan, entries }: { plan: TripPlan; entries: CalendarEntry[] } = $props();

  let now = $state(new Date());
  onMount(() => {
    const timer = setInterval(() => (now = new Date()), 60_000);
    return () => clearInterval(timer);
  });

  const todayKey = $derived(localDateKey(now));
  const tomorrowKey = $derived(
    localDateKey(new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1)),
  );

  const LEG_TONE: Record<TripStage["status"], Tone> = {
    booked: "success",
    completed: "muted",
    option_selected: "accent",
    planning: "warning",
    open: "warning",
  };

  const LEG_STATUS: Record<TripStage["status"], string> = {
    booked: "Booked",
    option_selected: "Chosen, not booked",
    planning: "Not booked",
    open: "Not booked",
    completed: "Done",
  };

  function dayNumber(key: string) {
    const ms = (k: string) => new Date(`${k}T12:00:00`).getTime();
    return Math.round((ms(key) - ms(plan.date_start)) / 86_400_000) + 1;
  }
  const totalDays = $derived(dayNumber(plan.date_end));

  function dayLabel(key: string) {
    return new Date(`${key}T12:00:00`).toLocaleDateString("en-GB", {
      weekday: "long",
      day: "numeric",
      month: "short",
    });
  }

  function legsOn(key: string) {
    return plan.stages
      .filter((s) => s.date === key)
      .sort((a, b) => a.sequence - b.sequence);
  }

  /// Calendar entries touching the day. Trip legs are left out because the
  /// plan renders them with their booking state, and the multi-day away block
  /// spanning the whole trip is this panel's header, not a row.
  function entriesOn(key: string) {
    return entries
      .filter((e) => {
        const start = e.starts_at.slice(0, 10);
        const end = e.ends_at.slice(0, 10);
        if (start > key || (end <= key && start !== key)) return false;
        if (e.source === "trips" && e.kind === "away") return false;
        if (e.kind === "away" && e.all_day && start < key) return false;
        return true;
      })
      .sort((a, b) => Number(b.all_day) - Number(a.all_day) || a.starts_at.localeCompare(b.starts_at));
  }

  function timing(entry: CalendarEntry): "past" | "now" | null {
    if (entry.all_day) return null;
    const t = now.getTime();
    if (new Date(entry.ends_at).getTime() < t) return "past";
    if (new Date(entry.starts_at).getTime() <= t) return "now";
    return null;
  }

  const days = $derived(
    [todayKey, tomorrowKey]
      .filter((key) => key <= plan.date_end)
      .map((key) => {
        const legs = legsOn(key);
        const items = entriesOn(key);
        // ponytail: a stage has a date but no time, so the order is a guess: you leave
        // in the morning, the day's timed events follow, later legs come after them.
        // Give TripStage a departure time to interleave them exactly.
        return {
          key,
          first: legs.slice(0, 1),
          later: legs.slice(1),
          timed: items.filter((e) => !e.all_day),
          allDay: items.filter((e) => e.all_day),
          empty: legs.length === 0 && items.length === 0,
        };
      }),
  );
</script>

<!-- A strip above Home's list, not a card (2026-10-09): one line per row, so a long title
     truncates instead of wrapping to three lines. Tomorrow folds away; the whole trip is one
     click on, in its context. -->
<section class="trip-day" aria-label="Trip today and tomorrow">
  <header>
    <h2>{plan.title} <span>day {dayNumber(todayKey)} of {totalDays}</span></h2>
    <a href={link(`/context?trip=${encodeURIComponent(plan.id)}`)}>Everything around it</a>
    <a href={link(`/travel?plan=${encodeURIComponent(plan.id)}`)}>Itinerary <Icon name="arrow-right" size={12} /></a>
  </header>

  {#each days as day, index (day.key)}
    {#if index === 0}
      <h3>Today <span>{dayLabel(day.key)}</span></h3>
      {@render dayRows(day)}
    {:else if day.empty}
      <p class="quiet">Tomorrow, {dayLabel(day.key)}: nothing planned.</p>
    {:else}
      <details>
        <summary><h3>Tomorrow <span>{dayLabel(day.key)}</span></h3><span class="count">{day.first.length + day.timed.length + day.later.length + day.allDay.length}</span></summary>
        {@render dayRows(day)}
      </details>
    {/if}
  {/each}
</section>

{#snippet dayRows(day: (typeof days)[number])}
  {#if day.empty}
    <p class="quiet">Nothing planned.</p>
  {:else}
    <ol>
      {#each day.first as leg (leg.id)}{@render legRow(leg)}{/each}
      {#each day.timed as entry (entry.id)}{@render entryRow(entry)}{/each}
      {#each day.later as leg (leg.id)}{@render legRow(leg)}{/each}
      {#each day.allDay as entry (entry.id)}{@render entryRow(entry)}{/each}
    </ol>
  {/if}
{/snippet}

{#snippet legRow(leg: TripStage)}
  {@const extra = [leg.transport_modes[0] && leg.transport_modes[0] !== "train" ? `by ${leg.transport_modes[0]}` : "", leg.branch_note ?? ""].filter(Boolean).join(" · ")}
  <li class="row">
    <span class="when"><Icon name="train" size={13} /></span>
    <strong>{leg.origin.name} → {leg.destination.name}</strong>
    <small use:tip={extra || undefined}>{extra}</small>
    <Chip label={LEG_STATUS[leg.status]} tone={LEG_TONE[leg.status]} />
  </li>
{/snippet}

{#snippet entryRow(entry: CalendarEntry)}
  {@const s = timing(entry)}
  <li class:past={s === "past"}>
    <button type="button" class="row" onclick={() => inspectorStore.open(eventItem(entry))}>
      <span class="when">{entry.all_day ? "all day" : entry.starts_at.slice(11, 16)}</span>
      <strong use:tip={entry.title}>
        <i style={`--entry-color: ${kindConfig(entry.kind).color}`} class:planned={entry.commitment !== "committed"}></i>
        <span class="title">{entry.title}</span>
      </strong>
      <small use:tip={entry.location ?? undefined}>{entry.location ?? ""}</small>
      {#if s === "now"}<Chip label="Now" tone="success" />{:else}<span></span>{/if}
    </button>
  </li>
{/snippet}

<style>
  .trip-day {
    padding-bottom: var(--space-4);
    margin-bottom: var(--space-5);
    border-bottom: 1px solid var(--rule);
  }

  header {
    display: flex;
    align-items: baseline;
    gap: var(--space-4);
    margin-bottom: var(--space-3);
  }

  h2 {
    margin: 0 auto 0 0;
    white-space: nowrap;
    font-size: var(--text-md);
    font-weight: 600;
  }

  h2 span,
  h3 span {
    margin-left: 0.35rem;
    color: var(--text-tertiary);
    font-size: var(--text-xs);
    font-weight: 400;
  }

  header a {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    color: var(--text-secondary);
    font-size: var(--text-xs);
  }

  header a:hover {
    color: var(--primary);
  }

  header a:focus-visible,
  summary:focus-visible,
  .row:focus-visible {
    outline: 2px solid var(--focus-ring);
  }

  h3 {
    margin: 0 0 var(--space-1);
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-secondary);
  }

  ol {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .row {
    display: grid;
    grid-template-columns: 3.25rem minmax(0, 1fr) minmax(0, 14rem) 5.5rem;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    min-height: 2rem;
    padding: 0 var(--space-1);
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    font: inherit;
    text-align: left;
    color: inherit;
  }

  button.row {
    cursor: pointer;
  }

  button.row:hover {
    background: var(--nav-hover);
  }

  .row > :global(.chip) {
    justify-self: end;
  }

  li.past {
    color: var(--text-tertiary);
  }

  .when {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  strong {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    min-width: 0;
    font-size: var(--text-sm);
    font-weight: 550;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .title {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  i {
    flex-shrink: 0;
    width: 0.5rem;
    height: 0.5rem;
    border: 1.5px solid var(--entry-color);
    border-radius: 50%;
    background: var(--entry-color);
  }

  i.planned {
    background: transparent;
  }

  small {
    min-width: 0;
    overflow: hidden;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  details {
    margin-top: var(--space-3);
  }

  summary {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    cursor: pointer;
    list-style: none;
  }

  summary::-webkit-details-marker {
    display: none;
  }

  summary h3 {
    margin: 0;
  }

  .count {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
  }

  .quiet {
    margin: var(--space-3) 0 0;
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  @media (width < 38rem) {
    .row {
      grid-template-columns: 3rem minmax(0, 1fr) auto;
    }

    small {
      display: none;
    }
  }
</style>
