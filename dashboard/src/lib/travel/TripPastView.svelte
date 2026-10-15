<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import MapSurface from "$lib/map/MapSurface.svelte";
  import { link } from "$lib/nav";
  import { tip } from "$lib/tip";
  import type { PlanItem, Retrospective, TripPlan } from "$lib/api";
  import { placeName } from "$lib/travel/place-image";
  import { TRIP_LAYERS } from "$lib/travel/trip-layers";
  import RetrospectiveForm from "$lib/travel/RetrospectiveForm.svelte";

  let {
    plan,
    items,
    retrospective,
    costProposal,
    tripMapSources,
    tripMapFitKey,
    onSavedRetrospective,
  }: {
    plan: TripPlan;
    items: PlanItem[];
    retrospective: Retrospective | null;
    costProposal: any;
    tripMapSources: any;
    tripMapFitKey: string;
    onSavedRetrospective: (row: Retrospective) => void;
  } = $props();

  const shortDate = (date: string | null) =>
    date
      ? new Intl.DateTimeFormat("en-GB", { day: "2-digit", month: "short" }).format(
          new Date(`${date.slice(0, 10)}T12:00:00`),
        )
      : "open";

  function itemLabel(item: PlanItem): string {
    const labels: Record<string, string> = {
      journey: "Rail connection",
      transport: "Transport",
      event: "Event",
      activity: "Activity",
      place: "Place",
      stay: "Accommodation",
      image: "Image",
      note: "Note",
      option_set: "Options",
    };
    return labels[item.item_type] ?? "Saved item";
  }

  const mapCity = $derived(
    plan.destinations.length > 0 ? placeName(plan.destinations[0]) : "",
  );
</script>

<section class="past-view">
  <div class="past-map-wrap">
    <div class="map-top-bar">
      <a
        class="map-link-btn"
        href={mapCity ? link(`/map?city=${encodeURIComponent(mapCity)}`) : link("/map")}
        use:tip={"Explore destination in Master Life Map"}
      >
        <Icon name="globe" size={12} />
        <span>Master Life Map</span>
      </a>
    </div>
    <MapSurface
      sources={tripMapSources}
      layers={TRIP_LAYERS}
      fitKey={tripMapFitKey}
      deferredLabel="Trip map"
    />
  </div>

  <div class="past-summary card">
    <span class="eyebrow">Trip history</span>
    <h3>{placeName(plan.origin)} → {plan.destinations.map(placeName).join(" → ")}</h3>
    <p class="dates mono">{shortDate(plan.date_start)} – {shortDate(plan.date_end)}</p>
    {#if plan.interests}
      <p class="past-intent">{plan.interests}</p>
    {/if}
  </div>

  <RetrospectiveForm
    planId={plan.id}
    currency={plan.currency}
    existing={retrospective}
    proposal={costProposal}
    onSaved={onSavedRetrospective}
  />

  <div class="past-timeline">
    <div class="section-heading">
      <h3>Saved itinerary</h3>
      <span class="mono count">{items.length} items</span>
    </div>
    {#if items.length === 0}
      <p class="empty-history">No itinerary has been saved for this trip yet.</p>
    {:else}
      <ol class="history-list">
        {#each items as item (item.id)}
          <li class="card">
            <time class="mono">{shortDate(item.day)}</time>
            <div class="item-copy">
              <strong>{item.title}</strong>
              <span class="subtitle">{itemLabel(item)}</span>
            </div>
          </li>
        {/each}
      </ol>
    {/if}
  </div>
</section>

<style>
  .past-view {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    margin-top: var(--space-3);
  }

  .past-map-wrap {
    position: relative;
    height: 18rem;
    border-radius: var(--radius-md);
    overflow: hidden;
    border: 1px solid var(--card-border);
  }

  .map-top-bar {
    position: absolute;
    top: var(--space-2);
    left: var(--space-2);
    z-index: 2;
  }

  .map-link-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.25rem 0.6rem;
    background: var(--surface);
    color: var(--text-primary);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    text-decoration: none;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12);
  }

  .map-link-btn:hover {
    background: var(--nav-hover);
  }

  .card {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    padding: var(--space-4);
  }

  .eyebrow {
    font-size: var(--text-2xs);
    color: var(--accent);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    display: block;
    margin-bottom: 0.25rem;
  }

  .past-summary h3 {
    margin: 0;
    font-size: var(--text-lg);
    color: var(--text-primary);
  }

  .dates {
    font-size: var(--text-xs);
    color: var(--text-secondary);
    margin: 0.35rem 0 0;
  }

  .past-intent {
    margin: 0.5rem 0 0;
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }

  .past-timeline {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .section-heading {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
  }

  .section-heading h3 {
    margin: 0;
    font-size: var(--text-base);
  }

  .count {
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .empty-history {
    font-size: var(--text-sm);
    color: var(--text-tertiary);
    padding: var(--space-4);
    text-align: center;
    background: var(--card-bg);
    border: 1px dashed var(--card-border);
    border-radius: var(--radius-md);
    margin: 0;
  }

  .history-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .history-list li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3);
  }

  .history-list time {
    font-size: var(--text-xs);
    color: var(--text-tertiary);
    flex-shrink: 0;
    width: 6rem;
  }

  .item-copy {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    min-width: 0;
  }

  .item-copy strong {
    font-size: var(--text-sm);
    color: var(--text-primary);
  }

  .subtitle {
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }
</style>
