<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import MapSurface from "$lib/map/MapSurface.svelte";
  import { link } from "$lib/nav";
  import { tip } from "$lib/tip";
  import type { TripPlan } from "$lib/api";
  import { placeName } from "$lib/travel/place-image";
  import { TRIP_LAYERS } from "$lib/travel/trip-layers";

  interface Props {
    plans: TripPlan[];
    upcomingPlans: TripPlan[];
    pastPlans: TripPlan[];
    filteredPlans: TripPlan[];
    planFilter: "upcoming" | "past" | "all";
    highlightedPlanId: string | null;
    overviewMapSources: any;
    overviewMapFitKey: string;
    todayKey: string;
    onSelectFilter: (filter: "upcoming" | "past" | "all") => void;
    onOpenPlan: (plan: TripPlan) => void;
    onOpenPlanner: () => void;
    onHighlightPlan: (id: string | null) => void;
  }

  let {
    plans,
    upcomingPlans,
    pastPlans,
    filteredPlans,
    planFilter,
    highlightedPlanId,
    overviewMapSources,
    overviewMapFitKey,
    todayKey,
    onSelectFilter,
    onOpenPlan,
    onOpenPlanner,
    onHighlightPlan,
  }: Props = $props();

  function planPhase(plan: TripPlan): "In progress" | "Planned" | "Past" {
    if (plan.date_end < todayKey) return "Past";
    if (plan.date_start <= todayKey) return "In progress";
    return "Planned";
  }

  const shortDate = (date: string | null) =>
    date
      ? new Intl.DateTimeFormat("en-GB", { day: "2-digit", month: "short" }).format(
          new Date(`${date.slice(0, 10)}T12:00:00`),
        )
      : "open";
</script>

{#if plans.length > 0}
  <section class="journey-index" aria-label="Trip overview">
    <div>
      <span class="index-value">{upcomingPlans.length}</span>
      <span class="index-label">upcoming</span>
    </div>
    <div>
      <span class="index-value">{pastPlans.length}</span>
      <span class="index-label">past</span>
    </div>
    {#if upcomingPlans[0]}
      <button type="button" onclick={() => void onOpenPlan(upcomingPlans[0])}>
        <span class="index-label">up next</span>
        <strong>{upcomingPlans[0].title}</strong>
        <small>{shortDate(upcomingPlans[0].date_start)}</small>
      </button>
    {:else}
      <div class="next-empty">
        <span class="index-label">up next</span>
        <strong>Still open</strong>
      </div>
    {/if}
  </section>

  <section class="travel-board">
    <header class="board-toolbar">
      <div>
        <h2>Trips on the map</h2>
        <p>The markers and list show the same saved plans.</p>
      </div>
      <div class="board-actions">
        <div class="plan-filters" aria-label="Filter trips">
          <button
            type="button"
            class:active={planFilter === "upcoming"}
            onclick={() => onSelectFilter("upcoming")}
          >Upcoming <span>{upcomingPlans.length}</span></button>
          <button
            type="button"
            class:active={planFilter === "past"}
            onclick={() => onSelectFilter("past")}
          >Past <span>{pastPlans.length}</span></button>
          <button
            type="button"
            class:active={planFilter === "all"}
            onclick={() => onSelectFilter("all")}
          >All <span>{plans.length}</span></button>
        </div>
        <button class="btn btn-primary" type="button" onclick={onOpenPlanner}>
          <Icon name="plus" size={14} /> New trip
        </button>
      </div>
    </header>

    <div class="board-layout">
      <div class="overview-map">
        <div class="map-top-bar">
          <a class="map-link-btn" href={link("/map")} use:tip={"Explore in Master Life Map"}>
            <Icon name="globe" size={12} />
            <span>Master Life Map</span>
          </a>
        </div>
        <MapSurface
          sources={overviewMapSources}
          layers={TRIP_LAYERS}
          fitKey={overviewMapFitKey}
          interactive={["trip-points"]}
          onFeatureClick={(_, feature) => {
            const groupId = feature.properties?.groupId;
            if (typeof groupId === "string") onHighlightPlan(groupId);
          }}
          deferredLabel="Trips"
          eager
        />
        <div class="map-legend" aria-label="Map legend">
          <span><i class="upcoming"></i> upcoming</span>
          <span><i class="past"></i> past</span>
          <span><i class="selected"></i> selected</span>
        </div>
      </div>

      <div class="trip-options" aria-live="polite">
        {#if filteredPlans.length === 0}
          <p class="empty-history">
            {planFilter === "past"
              ? "Completed trips appear here automatically."
              : "There are no trips for this filter yet."}
          </p>
        {:else}
          <ol>
            {#each filteredPlans as plan (plan.id)}
              <li>
                <button
                  type="button"
                  class:highlighted={highlightedPlanId === plan.id}
                  onmouseenter={() => onHighlightPlan(plan.id)}
                  onfocus={() => onHighlightPlan(plan.id)}
                  onclick={() => void onOpenPlan(plan)}
                >
                  <span class="option-date">
                    <strong>{shortDate(plan.date_start)}</strong>
                    <small>{shortDate(plan.date_end)}</small>
                  </span>
                  <span class="option-route">
                    <span class:past={planPhase(plan) === "Past"} class="phase">
                      {planPhase(plan)}
                    </span>
                    <strong>{placeName(plan.origin)} → {plan.destinations.map(placeName).join(" → ")}</strong>
                    <small>
                      {plan.destinations.length} {plan.destinations.length === 1 ? "destination" : "destinations"}
                      · {plan.status === "saved" ? "Itinerary saved" : "Draft"}
                    </small>
                  </span>
                  <Icon name="arrow-right" size={15} />
                </button>
              </li>
            {/each}
          </ol>
        {/if}
      </div>
    </div>
  </section>
{:else}
  <section class="board-empty">
    <div>
      <h2>No trips planned yet</h2>
      <p>Start one from scratch, plan around a candidate in the rail, or import from Obsidian.</p>
    </div>
    <button class="btn btn-primary" type="button" onclick={onOpenPlanner}>
      <Icon name="plus" size={14} /> New trip
    </button>
  </section>
{/if}

<style>
  .journey-index {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    margin-bottom: 0.75rem;
    border-block: 1px solid var(--card-border);
  }

  .journey-index > div,
  .journey-index > button {
    min-width: 0;
    padding: 0.8rem;
    border: 0;
    border-right: 1px solid var(--card-border);
    background: transparent;
    color: var(--text-primary);
    font: inherit;
    text-align: left;
  }

  .journey-index > button,
  .journey-index .next-empty {
    grid-column: 1 / -1;
    border-top: 1px solid var(--card-border);
    border-right: 0;
  }

  .journey-index > button {
    cursor: pointer;
  }

  .journey-index > button:hover strong {
    color: var(--primary);
  }

  .index-value,
  .index-label,
  .journey-index strong,
  .journey-index small {
    display: block;
  }

  .index-value {
    font-family: var(--font-mono);
    font-size: 1.15rem;
  }

  .index-label,
  .journey-index small {
    color: var(--text-tertiary);
    font-size: 0.625rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .travel-board {
    margin-bottom: 1rem;
    overflow: hidden;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background: var(--card-bg);
    box-shadow: var(--card-shadow);
  }

  .board-toolbar {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 0.9rem 1rem;
    border-bottom: 1px solid var(--card-border);
  }

  .board-toolbar h2 {
    margin: 0;
    font-size: 0.875rem;
  }

  .board-toolbar p {
    margin: 0.1rem 0 0;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .board-actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .plan-filters {
    display: flex;
    align-items: center;
    gap: 0.2rem;
  }

  .plan-filters button {
    padding: 0.35rem 0.5rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    font-size: var(--text-2xs);
    cursor: pointer;
  }

  .plan-filters button:hover,
  .plan-filters button.active {
    background: var(--primary-soft);
    color: var(--primary);
  }

  .plan-filters span {
    margin-left: 0.2rem;
    font-family: var(--font-mono);
    font-size: 0.5625rem;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.35rem 0.65rem;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    cursor: pointer;
  }

  .btn-primary {
    background: var(--primary);
    color: white;
  }

  .board-layout {
    display: grid;
    min-width: 0;
  }

  .overview-map {
    position: relative;
    min-width: 0;
    padding: 0.75rem;
    border-bottom: 1px solid var(--card-border);
  }

  .map-top-bar {
    position: absolute;
    top: 1.25rem;
    right: 1.25rem;
    z-index: 2;
  }

  .map-link-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.3rem 0.6rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text-primary);
    font-size: var(--text-2xs);
    font-weight: 600;
    text-decoration: none;
    box-shadow: 0 1px 4px rgb(0 0 0 / 12%);
    transition: background-color var(--motion-fast) ease, border-color var(--motion-fast) ease;
  }

  .map-link-btn:hover {
    background: var(--card-bg);
    border-color: var(--primary);
    color: var(--primary);
  }

  .map-legend {
    position: absolute;
    left: 1.4rem;
    bottom: 1.4rem;
    z-index: 2;
    display: flex;
    gap: 0.6rem;
    padding: 0.35rem 0.5rem;
    border: 1px solid rgb(255 255 255 / 65%);
    border-radius: var(--radius-sm);
    background: rgb(255 255 255 / 90%);
    color: #3f3f46;
    font-size: 0.5625rem;
    box-shadow: 0 1px 4px rgb(0 0 0 / 15%);
  }

  .map-legend span {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
  }

  .map-legend i {
    width: 0.45rem;
    height: 0.45rem;
    border-radius: 50%;
    background: #0891b2;
  }

  .map-legend i.past {
    background: #71717a;
  }

  .map-legend i.selected {
    background: #d97706;
  }

  .trip-options {
    min-width: 0;
    max-height: 28rem;
    overflow-y: auto;
  }

  .trip-options ol {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .trip-options li + li {
    border-top: 1px solid var(--card-border);
  }

  .trip-options button {
    display: grid;
    grid-template-columns: 4.25rem minmax(0, 1fr) auto;
    gap: 0.7rem;
    align-items: center;
    width: 100%;
    padding: 0.85rem;
    border: 0;
    border-left: 2px solid transparent;
    background: transparent;
    color: var(--text-primary);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .trip-options button:hover,
  .trip-options button.highlighted {
    border-left-color: var(--primary);
    background: var(--primary-soft);
  }

  .option-date strong,
  .option-date small,
  .option-route strong,
  .option-route small {
    display: block;
  }

  .option-date strong {
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
  }

  .option-date small,
  .option-route small {
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .option-route {
    min-width: 0;
  }

  .option-route strong {
    overflow: hidden;
    margin: 0.15rem 0;
    font-size: var(--text-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .phase {
    color: var(--primary);
    font-size: 0.5625rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .phase.past {
    color: var(--text-tertiary);
  }

  .empty-history {
    padding: 1rem;
    color: var(--text-tertiary);
    font-size: var(--text-xs);
    text-align: center;
  }

  .board-empty {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    margin-bottom: 1rem;
    padding: 1.5rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background: var(--card-bg);
    box-shadow: var(--card-shadow);
  }

  .board-empty h2 {
    margin: 0;
    font-size: 0.95rem;
  }

  .board-empty p {
    margin: 0.2rem 0 0;
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  @media (width >= 40rem) {
    .board-toolbar {
      flex-direction: row;
      align-items: center;
      justify-content: space-between;
    }

    .journey-index {
      grid-template-columns: 8rem 8rem 1fr;
    }

    .journey-index > button,
    .journey-index .next-empty {
      grid-column: auto;
      border-top: 0;
    }
  }

  @media (width >= 54rem) {
    .board-layout {
      grid-template-columns: minmax(0, 1.35fr) minmax(18rem, 0.85fr);
    }

    .overview-map {
      border-right: 1px solid var(--card-border);
      border-bottom: 0;
    }

    .trip-options {
      max-height: 23.5rem;
    }
  }
</style>
