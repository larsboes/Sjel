<script lang="ts">
  import { link } from "$lib/nav";
  import Icon from "$lib/Icon.svelte";
  import { tip } from "$lib/tip";
  import type { FeedEntry } from "$lib/api";

  export interface TravelPlanContext {
    id: string;
    label: string;
    date_start: string | null;
    date_end: string | null;
  }

  interface Lane {
    id: string;
    label: string;
    dateStart: string | null;
    dateEnd: string | null;
    entries: FeedEntry[];
    average: number;
  }

  let {
    entries,
    plans = [],
    onSelectTrip,
  }: {
    entries: FeedEntry[];
    plans?: TravelPlanContext[];
    onSelectTrip?: (tripId: string) => void;
  } = $props();

  function travelFactor(entry: FeedEntry) {
    return entry.evaluation?.factors.find((f) => f.key === "travel") ?? null;
  }

  function shortRange(start: string | null, end: string | null): string {
    const format = (value: string | null) =>
      value
        ? new Intl.DateTimeFormat("en-GB", { day: "2-digit", month: "short" }).format(
            new Date(`${value}T12:00:00`),
          )
        : "open";
    return `${format(start)} – ${format(end)}`;
  }

  const travelLanes = $derived.by<Lane[]>(() => {
    const lanes = new Map<string, {
      id: string;
      label: string;
      dateStart: string | null;
      dateEnd: string | null;
      entries: FeedEntry[];
    }>();

    for (const plan of plans) {
      lanes.set(plan.id, {
        id: plan.id,
        label: plan.label,
        dateStart: plan.date_start,
        dateEnd: plan.date_end,
        entries: [],
      });
    }

    for (const entry of entries) {
      const factor = travelFactor(entry);
      const context = factor?.context;
      if (!context || factor.score <= 0) continue;
      const lane = lanes.get(context.id);
      if (lane) {
        lane.entries.push(entry);
      } else {
        lanes.set(context.id, {
          id: context.id,
          label: context.label,
          dateStart: context.date_start,
          dateEnd: context.date_end,
          entries: [entry],
        });
      }
    }

    return [...lanes.values()]
      .map((lane) => ({
        ...lane,
        entries: [...lane.entries].sort((a, b) => {
          const scoreB = travelFactor(b)?.score ?? 0;
          const scoreA = travelFactor(a)?.score ?? 0;
          return scoreB - scoreA;
        }),
        average:
          lane.entries.length === 0
            ? 0
            : lane.entries.reduce((sum, e) => sum + (travelFactor(e)?.score ?? 0), 0) /
              lane.entries.length,
      }))
      .filter((lane) => lane.entries.length > 0)
      .sort((a, b) => (a.dateStart ?? "9999").localeCompare(b.dateStart ?? "9999"));
  });

  const totalMatches = $derived(
    travelLanes.reduce((sum, lane) => sum + lane.entries.length, 0),
  );
</script>

{#if travelLanes.length > 0}
  <details class="travel-widget">
    <summary>
      <div class="summary-left">
        <Icon name="compass" size={14} />
        <span class="summary-title">Travel Relevance</span>
        <span class="badge mono">{travelLanes.length} trips · {totalMatches} matches</span>
      </div>
      <a href={link("/travel")} class="travel-link" onclick={(e) => e.stopPropagation()}>
        Open Travel <Icon name="arrow-right" size={12} />
      </a>
    </summary>

    <div class="timeline-grid">
      {#each travelLanes as lane (lane.id)}
        <div class="lane-card">
          <header class="lane-header">
            <span class="dates mono">{shortRange(lane.dateStart, lane.dateEnd)}</span>
            <button
              type="button"
              class="trip-btn"
              onclick={() => onSelectTrip?.(lane.id)}
              use:tip={"Filter collection by this trip"}
            >
              <strong>{lane.label}</strong>
              <Icon name="arrow-right" size={11} />
            </button>
            <span class="lane-stat">
              {lane.entries.length} items · <span class="mono">{Math.round(lane.average * 100)}%</span> fit
            </span>
          </header>

          <ol class="lane-matches">
            {#each lane.entries.slice(0, 3) as entry (entry.id)}
              {@const factor = travelFactor(entry)}
              <li>
                <a href={link(`/feed/${entry.id}`)} class="entry-title">
                  {entry.title ?? entry.url}
                </a>
                <span class="score mono">{Math.round((factor?.score ?? 0) * 100)}</span>
              </li>
            {/each}
          </ol>
        </div>
      {/each}
    </div>
  </details>
{/if}

<style>
  .travel-widget {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    margin: var(--space-2) 0;
    overflow: hidden;
  }

  summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-3);
    cursor: pointer;
    font-size: var(--text-xs);
    color: var(--text-secondary);
    user-select: none;
    list-style: none;
  }

  summary::-webkit-details-marker {
    display: none;
  }

  summary:hover {
    background: var(--nav-hover);
  }

  .summary-left {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .summary-title {
    font-weight: 500;
    color: var(--text-primary);
  }

  .badge {
    background: var(--accent-soft);
    color: var(--accent);
    padding: 0.1rem 0.4rem;
    border-radius: var(--radius-full);
    font-size: var(--text-2xs);
  }

  .travel-link {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    color: var(--accent);
    text-decoration: none;
    font-size: var(--text-2xs);
  }

  .travel-link:hover {
    text-decoration: underline;
  }

  .timeline-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(18rem, 1fr));
    gap: var(--space-3);
    padding: var(--space-3);
    border-top: 1px solid var(--card-border);
    background: var(--surface);
  }

  .lane-card {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .lane-header {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  .dates {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .trip-btn {
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    padding: 0;
    border: none;
    background: none;
    cursor: pointer;
    text-align: left;
    color: var(--text-primary);
    font-size: var(--text-sm);
  }

  .trip-btn:hover {
    color: var(--accent);
  }

  .lane-stat {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .lane-matches {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    border-top: 1px solid var(--card-border);
    padding-top: var(--space-2);
  }

  .lane-matches li {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
    font-size: var(--text-xs);
  }

  .entry-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-secondary);
    text-decoration: none;
  }

  .entry-title:hover {
    color: var(--text-primary);
    text-decoration: underline;
  }

  .score {
    font-size: var(--text-2xs);
    color: var(--accent);
    flex-shrink: 0;
  }
</style>
