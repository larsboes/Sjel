<script lang="ts">
  import { tip } from "$lib/tip";
  import {
    commitmentConfig,
    isRecommended,
    kindConfig,
    nextCommitment,
    type CalendarDay,
    type CalendarEntry,
    type Commitment,
  } from "./types";
  import type { TripPlan } from "$lib/api";
  import { inspectorStore } from "$lib/inspector/inspector.svelte";

  let {
    days,
    trips = [],
    onSelectDay,
    onSelectEntry,
    onSelectRange,
    onAddEntry,
    onCycleCommitment,
    freeDays = new Set<string>(),
  }: {
    days: CalendarDay[];
    trips?: TripPlan[];
    onSelectDay?: (day: CalendarDay) => void;
    onSelectEntry?: (entry: CalendarEntry, day: CalendarDay) => void;
    onSelectRange?: (startDate: string, endDate: string) => void;
    onAddEntry?: (date: string) => void;
    onCycleCommitment?: (entry: CalendarEntry, next: Commitment) => void;
    /** Days the calendar capability itself calls free. Passed in rather than
     * derived here: the verdict is the capability's, not the grid's. */
    freeDays?: ReadonlySet<string>;
  } = $props();

  function tripFor(date: string): TripPlan | undefined {
    return trips.find(
      (t) => t.date_start && t.date_end && date >= t.date_start && date <= t.date_end,
    );
  }

  const DAY_HEADERS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

  let dragStart = $state<string | null>(null);
  let dragEnd = $state<string | null>(null);

  function orderedRange(): [string, string] | null {
    if (!dragStart || !dragEnd) return null;
    return dragStart <= dragEnd ? [dragStart, dragEnd] : [dragEnd, dragStart];
  }

  function isInDragRange(date: string): boolean {
    const range = orderedRange();
    return range ? date >= range[0] && date <= range[1] : false;
  }

  function startDrag(day: CalendarDay, event: PointerEvent) {
    if (event.button !== 0) return;
    event.preventDefault();
    dragStart = day.date;
    dragEnd = day.date;
  }

  function extendDrag(day: CalendarDay) {
    if (dragStart) dragEnd = day.date;
  }

  function finishDrag() {
    const range = orderedRange();
    dragStart = null;
    dragEnd = null;
    if (!range) return;

    if (range[0] === range[1]) {
      const day = days.find((candidate) => candidate.date === range[0]);
      if (day) onSelectDay?.(day);
      return;
    }
    onSelectRange?.(range[0], range[1]);
  }

  function cancelDrag() {
    dragStart = null;
    dragEnd = null;
  }

  function timeLabel(entry: CalendarEntry): string | null {
    return entry.all_day ? null : entry.starts_at.slice(11, 16);
  }
</script>

<svelte:window onpointerup={finishDrag} onpointercancel={cancelDrag} />

<div class="grid">
  {#each DAY_HEADERS as header}
    <div class="header">{header}</div>
  {/each}

  {#each days as day (day.date)}
    <div
      class="cell"
      class:other={!day.isCurrentMonth}
      class:today={day.isToday}
      class:has-entries={day.entries.length > 0}
      class:painting={isInDragRange(day.date)}
      role="group"
      aria-label={day.date}
      onpointerdown={(event) => startDrag(day, event)}
      onpointerenter={() => extendDrag(day)}
      onpointermove={() => extendDrag(day)}
    >
      <button
        class="date"
        aria-label={`${day.date}: create entry`}
        onclick={() => onSelectDay?.(day)}
        onpointerdown={(event) => event.stopPropagation()}
      >
        {day.day}
      </button>

      {#if tripFor(day.date)}
        {@const trip = tripFor(day.date)!}
        <button
          type="button"
          class="trip-ribbon"
          onclick={(e) => {
            e.stopPropagation();
            inspectorStore.inspectTrip({
              id: trip.id,
              title: trip.title,
              destination: trip.destinations?.[0]?.name ?? trip.title,
              dates: `${trip.date_start} – ${trip.date_end}`,
            });
          }}
          use:tip={`Trip: ${trip.title} (${trip.destinations?.[0]?.name ?? ""})`}
          onpointerdown={(event) => event.stopPropagation()}
        >
          <span class="trip-ribbon-dot"></span>
          <span class="trip-ribbon-text">{trip.destinations?.[0]?.name ?? trip.title}</span>
        </button>
      {/if}

      <div class="entries">
        {#each day.entries.slice(0, 2) as entry (entry.id)}
          <!-- Two buttons, not one: the dot changes how binding the entry is,
               the chip opens it. Nesting them would be invalid markup and
               would cost the commitment its own keyboard target. -->
          <div class="entry-row" style={`--entry-color: ${kindConfig(entry.kind).color}`}>
            <button
              class="entry-dot commitment-{entry.commitment}"
              use:tip={`${commitmentConfig(entry.commitment).label} — ${commitmentConfig(entry.commitment).hint}`}
              aria-label={`${entry.title}: ${commitmentConfig(entry.commitment).label}, click to change`}
              onclick={() => onCycleCommitment?.(entry, nextCommitment(entry.commitment))}
              onpointerdown={(event) => event.stopPropagation()}
            ></button>
            <button
              class="entry"
              class:proposal={entry.commitment === "possible"}
              use:tip={entry.title}
              aria-label={`Inspect ${entry.title}`}
              onclick={() => onSelectEntry?.(entry, day)}
              onpointerdown={(event) => event.stopPropagation()}
            >
              {#if timeLabel(entry)}
                <span class="entry-time">{timeLabel(entry)}</span>
              {/if}
              <span class="entry-title">{entry.title}</span>
              {#if isRecommended(entry, freeDays)}
                <span class="recommended" use:tip={"Still open, and the day is free"}>★</span>
              {/if}
            </button>
          </div>
        {/each}
        {#if day.entries.length > 2}
          <span class="more">+{day.entries.length - 2} more</span>
        {/if}
      </div>
    </div>
  {/each}
</div>

<p class="paint-hint">Drag across several days to add a date range.</p>

<button
  class="add-btn"
  aria-label="Create entry"
  onclick={() => onAddEntry?.(days.find((day) => day.isToday)?.date ?? days[0]?.date ?? "")}
>
  +
</button>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    gap: 2px;
    padding: 4px;
    background: var(--card-border);
    border-radius: 8px;
    user-select: none;
    touch-action: pan-y;
  }

  .header {
    text-align: center;
    font-size: var(--text-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 8px 0;
    color: var(--text-secondary);
    background: var(--card-bg);
  }

  .cell {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 5px;
    padding: 6px;
    min-height: 92px;
    background: var(--card-bg);
    transition: background var(--motion-fast), box-shadow var(--motion-fast);
    position: relative;
  }

  .cell:hover,
  .cell.painting {
    background: var(--surface);
  }

  .cell.painting {
    box-shadow: inset 0 0 0 2px var(--primary);
  }

  .cell.other {
    opacity: 0.42;
  }

  .date {
    align-self: flex-start;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--text-primary);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
  }

  .date:hover,
  .date:focus-visible {
    background: var(--surface);
    outline: 2px solid var(--primary);
    outline-offset: 1px;
  }

  .today .date {
    background: var(--primary);
    color: #fff;
  }

  .trip-ribbon {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.2rem 0.45rem;
    margin-bottom: 0.25rem;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
    color: var(--primary);
    border: 1px solid transparent;
    font: inherit;
    font-size: var(--text-2xs);
    font-weight: 600;
    cursor: pointer;
    text-align: left;
    width: 100%;
    overflow: hidden;
    white-space: nowrap;
    transition: background-color var(--motion-fast) ease, color var(--motion-fast) ease;
  }

  .trip-ribbon:hover {
    background: var(--primary);
    color: var(--text-inverse);
  }

  .trip-ribbon-dot {
    width: 5px;
    height: 5px;
    border-radius: var(--radius-full);
    background-color: currentColor;
    flex-shrink: 0;
  }

  .trip-ribbon-text {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .entries {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 3px;
  }

  .entry {
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 4px;
    padding: 3px 5px;
    border: 0;
    border-radius: 5px;
    background: color-mix(in srgb, var(--entry-color) 13%, transparent);
    color: var(--text-primary);
    font: inherit;
    font-size: var(--text-2xs);
    text-align: left;
    cursor: pointer;
  }

  .entry:hover,
  .entry:focus-visible {
    outline: 1px solid var(--entry-color);
    outline-offset: 0;
  }

  /* A proposal is a real row in this store, so the grid draws it — but it is
     something a source suggested, not something the operator agreed to, and a
     filled chip says the opposite. Outlined and unfilled, matching the outline
     dot beside it: on the radar, not on the calendar. */
  .entry.proposal {
    background: transparent;
    box-shadow: inset 0 0 0 1px
      color-mix(in srgb, var(--entry-color) 45%, transparent);
    color: var(--text-secondary);
  }

  .entry-row {
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 4px;
  }

  /* The commitment, as how filled the dot is: outline = on the radar,
     half = decided but unbooked, solid = actually happening. Reading the
     column tells you what your week really costs before any label does. */
  .entry-dot {
    width: 9px;
    height: 9px;
    flex: 0 0 auto;
    padding: 0;
    border: 1.5px solid var(--entry-color);
    border-radius: 50%;
    background: transparent;
    cursor: pointer;
  }

  .entry-dot.commitment-planned {
    background: linear-gradient(
      to right,
      var(--entry-color) 0 50%,
      transparent 50% 100%
    );
  }

  .entry-dot.commitment-committed {
    background: var(--entry-color);
  }

  .entry-dot:focus-visible {
    outline: 2px solid var(--entry-color);
    outline-offset: 2px;
  }

  /* Computed, never stored — see isRecommended in types.ts. */
  .recommended {
    flex: 0 0 auto;
    color: var(--entry-color);
    font-size: 0.7em;
    line-height: 1;
  }

  .entry-time {
    flex: 0 0 auto;
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .entry-title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .more {
    padding-left: 5px;
    font-size: 0.625rem;
    color: var(--text-secondary);
  }

  .paint-hint {
    margin: 7px 4px 0;
    color: var(--text-secondary);
    font-size: var(--text-2xs);
  }

  .add-btn {
    position: fixed;
    bottom: 24px;

    /* The page owns this inset, because only the page knows whether a rail is beside
     * the grid. Fixed to the viewport, the button would otherwise sit on top of it. */
    right: var(--grid-fab-inset, 24px);
    z-index: 10;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 48px;
    height: 48px;
    border: none;
    border-radius: 50%;
    background: var(--primary);
    color: #fff;
    font-size: 1.5rem;
    cursor: pointer;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
    transition: transform var(--motion-fast), box-shadow var(--motion-fast);
  }

  .add-btn:hover {
    transform: scale(1.06);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.35);
  }

  @media (max-width: 700px) {
    .cell {
      min-height: 70px;
      padding: 4px;
    }

    .entry {
      padding-inline: 3px;
    }

    .entry-time,
    .entry-title {
      display: none;
    }
  }
</style>
