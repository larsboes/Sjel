<script lang="ts">
  import { tip } from "$lib/tip";
  import {
    MINUTES_PER_DAY,
    allDaySpan,
    dayKey,
    kindConfig,
    packLanes,
    parseDayKey,
    timedSpan,
    type CalendarDay,
    type CalendarEntry,
  } from "./types";

  let {
    days,
    detail = false,
    onSelectDate,
    onSelectSlot,
    onSelectAllDay,
    onSelectEntry,
    onAddEntry,
  }: {
    days: CalendarDay[];
    /** Day view: fewer columns, so hours are taller and chips carry more. */
    detail?: boolean;
    onSelectDate?: (date: string) => void;
    onSelectSlot?: (date: string, hour: number) => void;
    onSelectAllDay?: (date: string) => void;
    onSelectEntry?: (entry: CalendarEntry, day: CalendarDay) => void;
    onAddEntry?: (date: string) => void;
  } = $props();

  const HOURS = Array.from({ length: 24 }, (_, hour) => hour);
  const WEEKDAY = new Intl.DateTimeFormat("en-GB", { weekday: "short" });

  let scroller = $state<HTMLDivElement>();
  let now = $state(new Date());

  let dates = $derived(days.map((day) => day.date));

  // All-day blocks span columns, so they are laid out once across the whole
  // range and stacked into rows where they overlap.
  let allDayRows = $derived(
    packLanes(
      dedupe(days.flatMap((day) => day.entries))
        .map((entry) => ({ entry, ...(allDaySpan(entry, dates) ?? { start: -1, end: -1 }) }))
        .filter((item) => item.end > item.start),
    ),
  );

  // Timed blocks belong to one column each; overlapping neighbours share the
  // column's width.
  let timedColumns = $derived(
    days.map((day) =>
      packLanes(
        day.entries
          .map((entry) => ({ entry, ...(timedSpan(entry, day.date) ?? { start: -1, end: -1 }) }))
          .filter((item) => item.end > item.start),
      ),
    ),
  );

  // Full days are rendered, but nobody plans at 03:00 — open on the first
  // entry, or on the working morning when the range is empty.
  let focusHour = $derived(
    Math.max(
      0,
      Math.min(...timedColumns.flat().map((item) => Math.floor(item.start / 60)), 8) - 1,
    ),
  );

  let nowDate = $derived(dayKey(now));
  let nowOffset = $derived(((now.getHours() * 60 + now.getMinutes()) / MINUTES_PER_DAY) * 100);

  $effect(() => {
    const timer = setInterval(() => (now = new Date()), 60_000);
    return () => clearInterval(timer);
  });

  $effect(() => {
    const hour = focusHour;
    if (scroller) scroller.scrollTop = (hour / 24) * scroller.scrollHeight;
  });

  /** An all-day block covering several visible days arrives once per day it
   * covers; the band draws it as one bar. */
  function dedupe(entries: CalendarEntry[]): CalendarEntry[] {
    const seen = new Set<string>();
    return entries.filter((entry) => {
      if (seen.has(entry.id)) return false;
      seen.add(entry.id);
      return true;
    });
  }

  function dayOf(date: string): CalendarDay {
    return days.find((day) => day.date === date) ?? days[0];
  }

  function hourLabel(hour: number): string {
    return `${String(hour).padStart(2, "0")}:00`;
  }

  function timeRange(entry: CalendarEntry): string {
    return `${entry.starts_at.slice(11, 16)}–${entry.ends_at.slice(11, 16)}`;
  }

  function entryLabel(entry: CalendarEntry): string {
    const when = entry.all_day ? "all day" : timeRange(entry);
    const rhythm = entry.rhythm_id ? ", from rhythm" : "";
    return `${entry.title}, ${when}${rhythm} — edit`;
  }

  /** The commitment as weight, in the vocabulary a time block has: how solid
   * its left edge is and how much colour its fill carries. The month grid says
   * the same thing with a dot; both read left to right as "less real" to
   * "actually happening". Cycling lives in the month grid and the form, so a
   * week view stays a reading surface. */
  function commitmentVars(commitment: string): string {
    switch (commitment) {
      case "committed":
        return "--edge-style: solid; --fill: 18%";
      case "planned":
        return "--edge-style: solid; --fill: 9%";
      default:
        return "--edge-style: dashed; --fill: 4%";
    }
  }

  function chipStyle(item: { entry: CalendarEntry; start: number; end: number; lane: number; lanes: number }): string {
    return [
      `--entry-color: ${kindConfig(item.entry.kind).color}`,
      commitmentVars(item.entry.commitment),
      `top: ${(item.start / MINUTES_PER_DAY) * 100}%`,
      `height: ${((item.end - item.start) / MINUTES_PER_DAY) * 100}%`,
      `left: ${(item.lane / item.lanes) * 100}%`,
      `width: ${100 / item.lanes}%`,
    ].join("; ");
  }

  function bandStyle(item: { entry: CalendarEntry; start: number; end: number; lane: number }): string {
    return [
      `--entry-color: ${kindConfig(item.entry.kind).color}`,
      commitmentVars(item.entry.commitment),
      `grid-column: ${item.start + 2} / span ${item.end - item.start}`,
      `grid-row: ${item.lane + 1}`,
    ].join("; ");
  }
</script>

<div class="timegrid" class:detail style={`--cols: ${days.length}`}>
  <div class="row head">
    <div class="gutter"></div>
    {#each days as day (day.date)}
      <div class="col-head" class:today={day.isToday} class:other={!day.isCurrentMonth}>
        <span class="weekday">{WEEKDAY.format(parseDayKey(day.date))}</span>
        <button
          class="date"
          aria-label={`${day.date}: Tagesansicht`}
          onclick={() => onSelectDate?.(day.date)}
        >
          {day.day}
        </button>
      </div>
    {/each}
  </div>

  <div class="row band" style={`--rows: ${Math.max(allDayRows.length, 1)}`}>
    <div class="gutter">All day</div>
    {#each days as day, index (day.date)}
      <button
        class="band-slot"
        class:other={!day.isCurrentMonth}
        style={`grid-column: ${index + 2}`}
        aria-label={`${day.date}: create all-day entry`}
        onclick={() => onSelectAllDay?.(day.date)}
      ></button>
    {/each}
    {#each allDayRows as item (item.entry.id)}
      <button
        class="chip band-chip"
        class:rhythm={item.entry.rhythm_id !== null}
        style={bandStyle(item)}
        use:tip={item.entry.title}
        aria-label={entryLabel(item.entry)}
        onclick={() => onSelectEntry?.(item.entry, dayOf(dates[item.start]))}
      >
        <span class="chip-title">{item.entry.title}</span>
        {#if item.entry.location}<span class="chip-meta">{item.entry.location}</span>{/if}
      </button>
    {/each}
  </div>

  <div class="scroll" bind:this={scroller}>
    <div class="row body">
      <div class="gutter hours">
        {#each HOURS as hour (hour)}
          <div class="hour-label">{hourLabel(hour)}</div>
        {/each}
      </div>

      {#each days as day, index (day.date)}
        <div class="col" class:today={day.isToday} class:other={!day.isCurrentMonth}>
          {#each HOURS as hour (hour)}
            <button
              class="slot"
              aria-label={`${day.date} ${hourLabel(hour)}: create entry`}
              onclick={() => onSelectSlot?.(day.date, hour)}
            ></button>
          {/each}

          {#if day.date === nowDate}
            <div class="now" style={`top: ${nowOffset}%`} aria-hidden="true"></div>
          {/if}

          {#each timedColumns[index] as item (item.entry.id)}
            <button
              class="chip timed"
              class:rhythm={item.entry.rhythm_id !== null}
              style={chipStyle(item)}
              use:tip={item.entry.title}
              aria-label={entryLabel(item.entry)}
              onclick={() => onSelectEntry?.(item.entry, day)}
            >
              <span class="chip-time">{timeRange(item.entry)}</span>
              <span class="chip-title">{item.entry.title}</span>
              {#if detail && item.entry.location}
                <span class="chip-meta">{item.entry.location}</span>
              {/if}
            </button>
          {/each}
        </div>
      {/each}
    </div>
  </div>
</div>

<p class="paint-hint">Click an open hour to add an entry there.</p>

<button
  class="add-btn"
  aria-label="Create entry"
  onclick={() => onAddEntry?.(days.find((day) => day.isToday)?.date ?? days[0]?.date ?? "")}
>
  +
</button>

<style>
  .timegrid {
    /* Wide enough for the band's "ALL DAY" label, which is what actually sizes
       this — the hour labels need barely 40px. At 54px the band label rendered
       at the left: the gutter is right-aligned, so an overflow clips the head
       of the word rather than the tail, and it reads like a typo instead of a
       layout bug. */
    --gutter: 70px;
    --hour-h: 44px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background: var(--card-bg);
    user-select: none;
  }

  .timegrid.detail {
    --hour-h: 60px;
  }

  .row {
    display: grid;
    grid-template-columns: var(--gutter) repeat(var(--cols), minmax(0, 1fr));
  }

  .gutter {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    padding-right: 7px;
    color: var(--text-tertiary);
    font-size: 0.625rem;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .head {
    border-bottom: 1px solid var(--card-border);
    background: var(--surface);
  }

  .col-head {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 7px 0;
  }

  .col-head.other {
    opacity: 0.42;
  }

  .weekday {
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .date {
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
    background: var(--primary-soft);
    outline: 2px solid var(--primary);
    outline-offset: 1px;
  }

  .today .date {
    background: var(--primary);
    color: var(--text-inverse);
  }

  .band {
    position: relative;
    grid-template-rows: repeat(var(--rows), 22px);
    gap: 2px 0;
    padding: 4px 0;
    border-bottom: 1px solid var(--card-border);
  }

  .band .gutter {
    grid-column: 1;
    grid-row: 1 / -1;
    align-items: flex-start;
    padding-top: 5px;
  }

  .band-slot {
    grid-row: 1 / -1;
    border: 0;
    border-left: 1px solid var(--card-border);
    background: transparent;
    cursor: pointer;
  }

  .band-slot:hover {
    background: var(--primary-soft);
  }

  .band-slot.other {
    background: var(--surface);
  }

  .scroll {
    max-height: 62vh;
    overflow-y: auto;
    overscroll-behavior: contain;
  }

  .hours {
    display: block;
  }

  .hour-label {
    height: var(--hour-h);
    padding-right: 7px;
    color: var(--text-tertiary);
    font-size: 0.625rem;
    font-variant-numeric: tabular-nums;
    text-align: right;
    text-transform: none;
    transform: translateY(-0.4em);
  }

  .col {
    position: relative;
    border-left: 1px solid var(--card-border);
  }

  .col.other {
    background: var(--surface);
    opacity: 0.62;
  }

  .col.today {
    background: var(--primary-soft);
  }

  .slot {
    display: block;
    width: 100%;
    height: var(--hour-h);
    padding: 0;
    border: 0;
    border-top: 1px solid var(--card-border);
    background: transparent;
    cursor: pointer;
  }

  .slot:first-child {
    border-top: 0;
  }

  .slot:hover,
  .slot:focus-visible {
    background: var(--primary-soft);
  }

  /* The tint is the hover feel; the ring is the focus indicator. They were one rule with
     `outline: none` in it, which left a keyboard reader with an 8%-alpha wash and nothing
     else — 1.11:1 against the surface, where WCAG 1.4.11 asks for 3:1. Inset by 2px on
     purpose: this control sits inside a scrolling container, so a ring drawn outside its
     own box is clipped away by the ancestor's overflow. `--primary` on the tint measures
     4.81:1 light and 7.83:1 dark. Same idiom as `$lib/rail/RailSection.svelte`. */
  .slot:focus-visible {
    outline: 2px solid var(--primary);
    outline-offset: -2px;
  }

  .now {
    position: absolute;
    left: 0;
    right: 0;
    height: 2px;
    background: var(--danger);
    pointer-events: none;
    z-index: 2;
  }

  .chip {
    display: flex;
    flex-direction: column;
    gap: 1px;
    overflow: hidden;
    padding: 2px 5px;
    border: 0;
    border-left: 3px var(--edge-style, solid) var(--entry-color);
    border-radius: 4px;
    background: color-mix(in srgb, var(--entry-color) var(--fill, 18%), var(--card-bg));
    color: var(--text-primary);
    font: inherit;
    font-size: var(--text-2xs);
    text-align: left;
    cursor: pointer;
  }

  .chip:hover,
  .chip:focus-visible {
    outline: 1px solid var(--entry-color);
    outline-offset: -1px;
  }

  /* Materialized from a rhythm — the hatch says "this is what my rhythm says",
     not "this is a one-off I painted". */
  .chip.rhythm {
    border-left-style: dashed;
    background-image: repeating-linear-gradient(
      135deg,
      transparent 0 5px,
      color-mix(in srgb, var(--entry-color) 22%, transparent) 5px 10px
    );
  }

  .timed {
    position: absolute;
    z-index: 1;
    box-sizing: border-box;
    min-height: 15px;
  }

  .band-chip {
    position: relative;
    z-index: 1;
    flex-direction: row;
    align-items: center;
    gap: 6px;
    margin: 0 2px;
  }

  .chip-time {
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .chip-title {
    overflow: hidden;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chip-meta {
    overflow: hidden;
    color: var(--text-secondary);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .detail .chip {
    font-size: var(--text-xs);
  }

  .detail .timed {
    padding: 4px 8px;
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
    color: var(--text-inverse);
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
    .timegrid {
      --gutter: 42px;
      --hour-h: 38px;
    }

    .chip-time {
      display: none;
    }

    .detail .chip-time {
      display: inline;
    }
  }
</style>
