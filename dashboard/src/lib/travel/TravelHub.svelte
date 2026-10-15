<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import MapSurface from "$lib/map/MapSurface.svelte";
  import { link } from "$lib/nav";
  import { tip } from "$lib/tip";
  import type { TransportMode, TripPlan } from "$lib/api";
  import { placeName } from "$lib/travel/place-image";
  import { TRIP_LAYERS } from "$lib/travel/trip-layers";
  import Collection from "$lib/ui/Collection.svelte";
  import Chip, { type Tone } from "$lib/ui/Chip.svelte";
  import type { Field } from "$lib/ui/collection";

  interface Props {
    plans: TripPlan[];
    upcomingPlans: TripPlan[];
    pastPlans: TripPlan[];
    highlightedPlanId: string | null;
    overviewMapSources: any;
    overviewMapFitKey: string;
    todayKey: string;
    modeOptions: Array<{ id: TransportMode; label: string }>;
    onOpenPlan: (plan: TripPlan) => void;
    onOpenPlanner: () => void;
    onHighlightPlan: (id: string | null) => void;
  }

  let {
    plans,
    upcomingPlans,
    pastPlans,
    highlightedPlanId,
    overviewMapSources,
    overviewMapFitKey,
    todayKey,
    modeOptions,
    onOpenPlan,
    onOpenPlanner,
    onHighlightPlan,
  }: Props = $props();

  let mapVisible = $state(true);

  function planPhase(plan: TripPlan): "In progress" | "Planned" | "Past" {
    if (plan.date_end < todayKey) return "Past";
    if (plan.date_start <= todayKey) return "In progress";
    return "Planned";
  }

  const PHASE_TONE: Record<string, Tone> = {
    "In progress": "warning",
    "Planned": "accent",
    "Past": "muted",
  };

  const shortDate = (date: string | null) =>
    date
      ? new Intl.DateTimeFormat("en-GB", { day: "2-digit", month: "short" }).format(
          new Date(`${date.slice(0, 10)}T12:00:00`),
        )
      : "open";

  function modeLabel(mode: TransportMode): string {
    return modeOptions.find((o) => o.id === mode)?.label ?? mode;
  }

  const fields: Field<TripPlan>[] = [
    {
      id: "title",
      label: "Trip",
      value: (p) => p.title,
      cell: titleCell,
    },
    {
      id: "phase",
      label: "Phase",
      kind: "select",
      width: "7.5rem",
      value: (p) => planPhase(p),
      tone: (v) => PHASE_TONE[v] ?? "neutral",
    },
    {
      id: "dates",
      label: "Dates",
      width: "11rem",
      value: (p) => `${shortDate(p.date_start)} – ${shortDate(p.date_end)}`,
      cell: datesCell,
    },
    {
      id: "modes",
      label: "Modes",
      width: "9rem",
      value: (p) => p.transport_modes.join(", "),
      cell: modesCell,
    },
    {
      id: "travelers",
      label: "Travelers",
      width: "8rem",
      value: (p) => p.travelers.join(", ") || "Solo",
    },
    {
      id: "date",
      label: "Start",
      kind: "date",
      width: "7.5rem",
      value: (p) => p.date_start.slice(0, 10),
    },
  ];
</script>

{#snippet titleCell(p: TripPlan)}
  <span class="trip-cell">
    <strong>{p.title}</strong>
    <span class="route-sub">{placeName(p.origin)} → {p.destinations.map(placeName).join(" → ")}</span>
  </span>
{/snippet}

{#snippet datesCell(p: TripPlan)}
  <span class="dates-cell mono">
    {shortDate(p.date_start)} – {shortDate(p.date_end)}
  </span>
{/snippet}

{#snippet modesCell(p: TripPlan)}
  <span class="modes-cell">
    {#each p.transport_modes as mode (mode)}
      <Chip label={modeLabel(mode)} tone={mode === "train" ? "success" : mode === "flight" ? "accent" : "neutral"} />
    {/each}
  </span>
{/snippet}

{#snippet planActions(p: TripPlan)}
  <div class="row-actions">
    <button
      type="button"
      class="icon-act"
      aria-label="Open trip"
      use:tip={"Open trip details"}
      onclick={(e) => {
        e.stopPropagation();
        onOpenPlan(p);
      }}
    >
      <Icon name="arrow-right" size={13} />
    </button>
  </div>
{/snippet}

<section class="hub-container">
  <div class="hub-metrics">
    <div class="metric">
      <span class="m-label">Upcoming</span>
      <strong class="m-val mono accent">{upcomingPlans.length}</strong>
    </div>
    <div class="metric">
      <span class="m-label">Past</span>
      <strong class="m-val mono muted">{pastPlans.length}</strong>
    </div>
    {#if upcomingPlans[0]}
      <button
        type="button"
        class="next-card"
        onclick={() => onOpenPlan(upcomingPlans[0])}
        use:tip={"Jump to next trip"}
      >
        <span class="m-label">Up next</span>
        <strong>{upcomingPlans[0].title}</strong>
        <small class="mono">{shortDate(upcomingPlans[0].date_start)}</small>
      </button>
    {/if}
    <div class="hub-controls">
      <button
        type="button"
        class="btn btn-secondary"
        onclick={() => (mapVisible = !mapVisible)}
        use:tip={"Toggle interactive route map"}
      >
        <Icon name="globe" size={13} />
        {mapVisible ? "Hide map" : "Show map"}
      </button>
      <button
        type="button"
        class="btn btn-primary"
        onclick={onOpenPlanner}
      >
        <Icon name="plus" size={13} />
        New trip
      </button>
    </div>
  </div>

  {#if mapVisible && plans.length > 0}
    <div class="map-wrap">
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
    </div>
  {/if}

  <Collection
    id="trips"
    rows={plans}
    {fields}
    key={(p) => p.id}
    title={(p) => p.title}
    defaults={{ view: "table", group: "phase", filter: { phase: ["In progress", "Planned"] } }}
    onOpen={(p) => onOpenPlan(p)}
    inactive={(p) => planPhase(p) === "Past"}
    selected={highlightedPlanId}
    actions={planActions}
    empty="No trips match these filters."
  />
</section>

<style>
  .hub-container {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .hub-metrics {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-2) 0;
    border-bottom: 1px solid var(--card-border);
    flex-wrap: wrap;
  }

  .metric {
    display: flex;
    flex-direction: column;
    padding-right: var(--space-4);
    border-right: 1px solid var(--card-border);
  }

  .m-label {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .m-val {
    font-size: var(--text-lg);
    line-height: 1.1;
  }

  .m-val.accent {
    color: var(--accent);
  }

  .m-val.muted {
    color: var(--text-tertiary);
  }

  .next-card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    padding: var(--space-1) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    cursor: pointer;
    text-align: left;
  }

  .next-card:hover {
    background: var(--nav-hover);
    border-color: var(--accent);
  }

  .next-card strong {
    font-size: var(--text-sm);
    color: var(--text-primary);
  }

  .next-card small {
    font-size: var(--text-2xs);
    color: var(--text-secondary);
  }

  .hub-controls {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .map-wrap {
    position: relative;
    height: 16rem;
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

  .trip-cell {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  .route-sub {
    font-size: var(--text-2xs);
    color: var(--text-secondary);
  }

  .dates-cell {
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .modes-cell {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
  }

  .row-actions {
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }

  .icon-act {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    border-radius: var(--radius-sm);
    background: var(--surface);
    border: 1px solid var(--card-border);
    color: var(--text-secondary);
    cursor: pointer;
  }

  .icon-act:hover {
    color: var(--accent);
    background: var(--nav-hover);
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.35rem;
    padding: 0.4rem 0.75rem;
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    font-weight: 500;
    cursor: pointer;
    border: none;
  }

  .btn-primary {
    background: var(--accent);
    color: white;
  }

  .btn-primary:hover {
    opacity: 0.9;
  }

  .btn-secondary {
    background: var(--surface);
    border: 1px solid var(--card-border);
    color: var(--text-primary);
  }

  .btn-secondary:hover {
    background: var(--nav-hover);
  }
</style>
