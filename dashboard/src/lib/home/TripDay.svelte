<script lang="ts">
  import { onMount } from "svelte";
  import { eventItem } from "$lib/inspector/connections";
  import { inspectorStore } from "$lib/inspector/inspector.svelte";
  import { kindConfig } from "$lib/calendar/types";
  import { link } from "$lib/nav";
  import Icon from "$lib/Icon.svelte";
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

<section class="trip-day" aria-label="Trip today and tomorrow">
  <header>
    <div>
      <span class="kicker">On a trip · day {dayNumber(todayKey)} of {totalDays}</span>
      <h2>{plan.title}</h2>
    </div>
    <a href={link(`/travel?plan=${encodeURIComponent(plan.id)}`)}>Open trip <Icon name="arrow-right" size={12} /></a>
  </header>

  <div class="days">
    {#each days as day, index (day.key)}
      <div class="day">
        <h3>{index === 0 ? "Today" : "Tomorrow"} <span>{dayLabel(day.key)}</span></h3>
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
      </div>
    {/each}
  </div>
</section>

{#snippet legRow(leg: TripStage)}
  <li class="leg">
    <span class="when"><Icon name="train" size={13} /></span>
    <strong>
      {leg.origin.name} → {leg.destination.name}
      {#if leg.transport_modes[0] && leg.transport_modes[0] !== "train"}<span class="mode">by {leg.transport_modes[0]}</span>{/if}
      {#if leg.branch_note}<span class="mode">{leg.branch_note}</span>{/if}
    </strong>
    <span class="status" class:open={leg.status !== "booked" && leg.status !== "completed"}>
      {LEG_STATUS[leg.status]}
    </span>
  </li>
{/snippet}

{#snippet entryRow(entry: CalendarEntry)}
  {@const s = timing(entry)}
  <li class:past={s === "past"}>
    <button type="button" onclick={() => inspectorStore.open(eventItem(entry))}>
      <span class="when">{entry.all_day ? "" : entry.starts_at.slice(11, 16)}</span>
      <strong>
        <i style={`--entry-color: ${kindConfig(entry.kind).color}`} class:planned={entry.commitment !== "committed"}></i>
        {entry.title}
        {#if s === "now"}<span class="now">Now</span>{/if}
      </strong>
      {#if entry.location}<small>{entry.location}</small>{/if}
    </button>
  </li>
{/snippet}

<style>
  .trip-day {
    margin-bottom: var(--space-4);
    padding: var(--space-4) var(--space-5);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-lg);
    background: var(--card-bg);
    box-shadow: var(--card-shadow);
  }

  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
    margin-bottom: var(--space-3);
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

  .kicker {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  h2 {
    margin: 0.1rem 0 0;
    font-size: var(--text-md);
    font-weight: 600;
  }

  .days {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(18rem, 1fr));
    gap: var(--space-5);
  }

  h3 {
    margin: 0 0 0.35rem;
    font-size: var(--text-sm);
    font-weight: 600;
  }

  h3 span {
    margin-left: 0.35rem;
    color: var(--text-tertiary);
    font-weight: 400;
  }

  ol {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li + li {
    border-top: 1px solid var(--card-border);
  }

  li.leg,
  li button {
    display: grid;
    grid-template-columns: 2.75rem minmax(0, 1fr) auto;
    align-items: baseline;
    gap: 0.5rem;
    width: 100%;
    padding: 0.45rem 0.25rem;
    border: none;
    background: transparent;
    text-align: left;
  }

  li button {
    cursor: pointer;
    border-radius: var(--radius-sm);
  }

  li button:hover {
    background: var(--surface);
  }

  li.past {
    opacity: 0.5;
  }

  .when {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
    font-variant-numeric: tabular-nums;
  }

  .leg .when {
    align-self: center;
    color: var(--primary);
  }

  strong {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    min-width: 0;
    font-size: var(--text-sm);
    font-weight: 550;
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
    overflow: hidden;
    max-width: 12rem;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .status {
    white-space: nowrap;
    color: var(--success);
    font-size: var(--text-2xs);
    font-weight: 600;
  }

  .status.open {
    color: var(--warning);
  }

  .now {
    padding: 0.05rem 0.4rem;
    border-radius: var(--radius-sm);
    background: var(--success-soft);
    color: var(--success);
    font-size: var(--text-2xs);
    font-weight: 600;
  }

  .mode {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    font-weight: 400;
  }

  .quiet {
    margin: 0.35rem 0 0;
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }
</style>
