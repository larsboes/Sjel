<script lang="ts">
  import { onMount } from "svelte";
  import { link } from "$lib/nav";
  import { tip } from "$lib/tip";
  import Icon from "$lib/Icon.svelte";
  import { contextLink, entryReaderLink, kindConfig } from "$lib/calendar/types";
  import type { CalendarContext, CalendarEntry } from "$lib/api";
  import { inspectorStore } from "$lib/inspector/inspector.svelte";

  let {
    contexts,
    entries,
  }: {
    contexts: CalendarContext[];
    entries: CalendarEntry[];
  } = $props();

  let now = $state(new Date());

  onMount(() => {
    const timer = setInterval(() => {
      now = new Date();
    }, 30_000);
    return () => clearInterval(timer);
  });

  /// Four rows before the Calendar page is the better surface. Past that this
  /// stops being a horizon and starts being a second calendar.
  const ENTRY_LIMIT = 4;

  function shortDate(value: string) {
    return new Date(`${value.slice(0, 10)}T12:00:00`).toLocaleDateString("en-GB", {
      day: "numeric",
      month: "short",
    });
  }

  function entryTime(entry: CalendarEntry) {
    return entry.all_day ? "all day" : entry.starts_at.slice(11, 16);
  }

  function getProximity(entry: CalendarEntry): { isNow: boolean; text: string } | null {
    if (entry.all_day) return null;
    const start = new Date(entry.starts_at).getTime();
    const end = new Date(entry.ends_at).getTime();
    const current = now.getTime();

    if (current >= start && current <= end) {
      return { isNow: true, text: "Now" };
    }
    const diffMin = Math.round((start - current) / 60_000);
    if (diffMin > 0 && diffMin <= 120) {
      return {
        isNow: false,
        text: diffMin < 60 ? `in ${diffMin}m` : `in ${Math.floor(diffMin / 60)}h ${diffMin % 60}m`,
      };
    }
    return null;
  }

  /// A context is a span, so it reads as one: "1–2 Sept", or a single date when
  /// it opens and closes on the same day. The kind label the old tiles carried
  /// (PREFERENCE / TRANSITION / NOTE) is Calendar-page vocabulary — at a glance
  /// the title already says what it is, and the taxonomy was pure ink.
  function span(context: CalendarContext) {
    const from = shortDate(context.valid_from);
    const until = shortDate(context.valid_until);
    return from === until ? from : `${from} – ${until}`;
  }
</script>

{#if contexts.length > 0 || entries.length > 0}
  <section class="horizon">
    {#if entries.length > 0}
      <ol class="entries">
        {#each entries.slice(0, ENTRY_LIMIT) as entry (entry.id)}
          {@const proximity = getProximity(entry)}
          <li>
            <button
              type="button"
              class="entry-row-action"
              onclick={() => inspectorStore.inspectEvent({
                id: entry.id,
                title: entry.title,
                startsAt: entry.starts_at,
                endsAt: entry.ends_at,
                allDay: entry.all_day,
                location: entry.location ?? undefined,
                commitment: entry.commitment,
              })}
              use:tip={"Inspect event details"}
            >
              <time>{shortDate(entry.starts_at)}</time>
              <i
                style={`--entry-color: ${kindConfig(entry.kind).color}`}
                class:planned={entry.commitment !== "committed"}
              ></i>
              <span class="entry-main">
                <strong>{entry.title}</strong>
                {#if proximity}
                  <span class="live-badge" class:now={proximity.isNow}>
                    <span class="pulse-dot"></span>
                    {proximity.text}
                  </span>
                {/if}
              </span>
              <small>
                <span class="when">{entryTime(entry)}</span>
                {#if entry.location}<span class="where">{entry.location}</span>{/if}
              </small>
            </button>
          </li>
        {/each}
      </ol>
    {/if}

    {#if contexts.length > 0}
      <p class="contexts">
        {#each contexts as context, index (context.id)}
          <a href={contextLink(context)} use:tip={context.details || context.title}>
            {context.title}<span>{span(context)}</span>
          </a>
        {/each}
        <a class="edit" href={link("/calendar")}>Calendar</a>
      </p>
    {/if}
  </section>
{/if}

<style>
  /* No card, no tiles. The old shape was a bordered card holding bordered
     tiles holding taxonomy labels — three levels of chrome around four facts.
     A hairline rule and whitespace carry the same separation. */
  .horizon {
    margin-bottom: 1.35rem;
  }

  .entries {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .entries li + li {
    border-top: 1px solid var(--card-border);
  }

  .entries .entry-row-action {
    display: grid;
    grid-template-columns: 3.5rem auto minmax(0, 1fr) auto;
    align-items: baseline;
    gap: 0.6rem;
    padding: 0.45rem 0.25rem;
    width: 100%;
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
    border-radius: var(--radius-sm);
    transition: background-color var(--motion-fast) ease;
  }

  .entries .entry-row-action:hover {
    background: var(--surface);
  }

  .entries .entry-row-action:hover strong {
    color: var(--primary);
  }

  .entries time {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
    font-variant-numeric: tabular-nums;
  }

  /* Same vocabulary the month grid uses: filled means committed, outlined
     means planned. One glyph doing the work the old "COMMITTED" / "PLANNED"
     labels did in a whole column of uppercase text. */
  .entries i {
    align-self: center;
    width: 0.5rem;
    height: 0.5rem;
    border: 1.5px solid var(--entry-color);
    border-radius: 50%;
    background: var(--entry-color);
  }

  .entries i.planned {
    background: transparent;
  }

  .entries strong {
    overflow: hidden;
    font-size: var(--text-sm);
    font-weight: 550;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .entry-main {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    min-width: 0;
  }

  .live-badge {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.05rem 0.4rem;
    border-radius: var(--radius-sm);
    font-size: var(--text-2xs);
    font-weight: 600;
    color: var(--primary);
    background-color: var(--primary-soft);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .live-badge.now {
    color: var(--success);
    background-color: var(--success-soft);
  }

  .pulse-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background-color: currentColor;
    animation: pulse-glow 2s cubic-bezier(0.4, 0, 0.6, 1) infinite;
  }

  @keyframes pulse-glow {
    0%, 100% {
      opacity: 1;
      transform: scale(1);
    }
    50% {
      opacity: 0.4;
      transform: scale(0.85);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .pulse-dot {
      animation: none;
    }
  }

  /* Two facts, separated by space rather than by a middle dot. "all day · Telekom,
     Bonn" made the reader parse a punctuation mark to find the boundary the layout can
     state outright — and the same dot was doing that job in 166 places across the app.
     The time is the fixed-width half, so it gets the tabular figures.

     A venue line runs to a full street address, so it truncates rather than
     stretching the title column it sits beside. */
  .entries small {
    display: flex;
    gap: var(--space-4);
    overflow: hidden;
    max-width: 22rem;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    text-align: right;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Contexts are ambient, not scheduled — so one wrapped line of quiet text
     under the schedule, never a row of cards competing with it. */
  /* The gap separates them. A middle dot between each pair made the reader parse
     punctuation to find a boundary a wider gap states outright. */
  .contexts {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-3) var(--space-6);
    margin: 0.7rem 0 0;
    padding-top: 0.55rem;
    border-top: 1px solid var(--card-border);
    font-size: var(--text-2xs);
  }

  .contexts a {
    color: var(--text-secondary);
  }

  .contexts a:hover {
    color: var(--primary);
  }

  .contexts a span {
    margin-left: 0.3rem;
    color: var(--text-tertiary);
  }

  .contexts .edit {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    margin-left: auto;
    color: var(--text-tertiary);
  }

  @media (width < 38rem) {
    .entries .entry-row-action {
      grid-template-columns: 3.25rem auto minmax(0, 1fr);
      row-gap: 0.1rem;
      min-height: 3rem;
      padding-block: 0.55rem;
    }

    .entries small {
      grid-column: 3;
      text-align: left;
    }

    .contexts {
      font-size: var(--text-xs);
    }

    .contexts a {
      min-height: 1.75rem;
    }
  }
</style>
