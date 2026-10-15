<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import { link } from "$lib/nav";
  import { tip } from "$lib/tip";
  import type {
    CalendarEntry,
    Entity,
    Journey,
    LocatedPerson,
    PlanItem,
    PlaceRef,
    ScoredResult,
    TransportMode,
    TripPlan,
  } from "$lib/api";
  import { placeName, type PlaceImage } from "$lib/travel/place-image";
  import { TRIP_LAYERS } from "$lib/travel/trip-layers";
  import type { ClimateBatch, ClimateResult, PlanCost } from "$lib/travel/api";
  import type { NearbyPlace } from "$lib/travel/nearby-places";
  import TripItinerary from "$lib/travel/TripItinerary.svelte";
  import ItineraryTimeline from "$lib/travel/ItineraryTimeline.svelte";
  import PlanEditor from "$lib/travel/PlanEditor.svelte";
  import CostCard from "$lib/travel/CostCard.svelte";
  import WhoIsAround from "$lib/travel/WhoIsAround.svelte";
  import PackingPanel from "$lib/travel/PackingPanel.svelte";
  import OutfitsPanel from "$lib/travel/OutfitsPanel.svelte";
  import ClimateStrip from "$lib/travel/ClimateStrip.svelte";
  import MapSurface from "$lib/map/MapSurface.svelte";
  import TripResultColumns from "$lib/travel/TripResultColumns.svelte";
  import Chip from "$lib/ui/Chip.svelte";

  interface Props {
    plan: TripPlan;
    items: PlanItem[];
    modeOptions: Array<{ id: TransportMode; label: string }>;
    planCostRollup: PlanCost | null;
    planCalendarEntries: CalendarEntry[];
    knownPeople: Entity[];
    peopleNotice: string | null;
    legCoordinates: Record<string, [number, number] | null>;
    locatedByDay: Record<string, LocatedPerson[]>;
    results: any[];
    selectedId: string;
    climate: ClimateBatch | null;
    tripMapSources: any;
    tripMapFitKey: string;
    journeySort?: "departure" | "duration" | "price" | "ranked";
    journeyPriority?: string;
    journeyPhrase?: string;
    expandedJourneyId?: string | null;
    orderedJourneys: (journeys: Journey[]) => Journey[];
    savingPlan: boolean;
    deletingPlan: boolean;
    onBack: () => void;
    onSavePlanEdits: (patch: any) => Promise<void>;
    onDeletePlan: () => Promise<void>;
    onUpdateStage: (id: string, patch: any) => Promise<void>;
    onToggleStageMode: (stageId: string, mode: TransportMode) => void;
    onItemChanged: (item: PlanItem) => void;
    onRemoveItem: (item: PlanItem) => Promise<void>;
    onUpdateItemDay: (item: PlanItem, day: string | null) => Promise<void>;
    onSaveCalendarEvent: (entry: CalendarEntry) => Promise<void>;
    onSaveJourney: (journey: Journey, place: PlaceRef) => Promise<void>;
    onSaveEvent: (event: ScoredResult) => Promise<void> | void;
    onSavePlace: (activity: NearbyPlace) => Promise<void> | void;
    onReloadWhoIsAround: () => Promise<void>;
    onUseCover: (image: PlaceImage) => Promise<void>;
  }

  let {
    plan,
    items = $bindable(),
    modeOptions,
    planCostRollup,
    planCalendarEntries,
    knownPeople,
    peopleNotice,
    legCoordinates,
    locatedByDay,
    results,
    selectedId = $bindable(),
    climate,
    tripMapSources,
    tripMapFitKey,
    journeySort = $bindable("ranked"),
    journeyPriority = $bindable(""),
    journeyPhrase = $bindable(""),
    expandedJourneyId = $bindable(null),
    orderedJourneys,
    savingPlan,
    deletingPlan,
    onBack,
    onSavePlanEdits,
    onDeletePlan,
    onUpdateStage,
    onToggleStageMode,
    onItemChanged,
    onRemoveItem,
    onUpdateItemDay,
    onSaveCalendarEvent,
    onSaveJourney,
    onSaveEvent,
    onSavePlace,
    onReloadWhoIsAround,
    onUseCover,
  }: Props = $props();

  let activeTab = $state<"itinerary" | "discovery" | "people" | "logistics">("itinerary");
  let editingPlan = $state(false);
  let mapOpen = $state(false);

  const selected = $derived(results.find((r) => r.place.id === selectedId) ?? results[0]);

  const shortDate = (date: string | null) =>
    date
      ? new Intl.DateTimeFormat("en-GB", { day: "2-digit", month: "short" }).format(
          new Date(`${date.slice(0, 10)}T12:00:00`),
        )
      : "open";

  function stageStatusLabel(status: string): string {
    if (status === "booked") return "Booked";
    if (status === "holding") return "Holding";
    if (status === "option") return "Option";
    return "Draft";
  }

  function modeLabel(mode: TransportMode): string {
    return modeOptions.find((o) => o.id === mode)?.label ?? mode;
  }

  function itemSaved(kind: string, key: string): boolean {
    return items.some((it) => it.item_type === kind && it.external_id === key);
  }

  function climateResultFor(place: PlaceRef): ClimateResult | null {
    if (!climate) return null;
    if (typeof place.latitude !== "number" || typeof place.longitude !== "number") return null;
    const key = `${place.latitude},${place.longitude}`;
    return climate.results.find((r) => r.key === key) ?? null;
  }

  function calendarTime(entry: CalendarEntry): string {
    if (entry.all_day) return shortDate(entry.starts_at);
    return `${shortDate(entry.starts_at)} · ${entry.starts_at.slice(11, 16)}–${entry.ends_at.slice(11, 16)}`;
  }

  function calendarUrl(entry: CalendarEntry): string | null {
    if (entry.id.startsWith("google:")) {
      return `https://calendar.google.com/calendar/u/0/r/eventedit/${entry.id.slice(7)}`;
    }
    return null;
  }

  function commitmentLabel(commitment: string): string {
    if (commitment === "must") return "Confirmed";
    if (commitment === "should") return "Planned";
    return "Tentative";
  }

  const MODE_TONE: Record<string, "success" | "accent" | "warning" | "neutral"> = {
    train: "success",
    flight: "accent",
    car: "warning",
    bus: "warning",
    ferry: "accent",
    bike: "success",
    walk: "neutral",
  };
</script>

{#if editingPlan}
  <PlanEditor
    {plan}
    saving={savingPlan}
    deleting={deletingPlan}
    onSave={async (patch) => {
      await onSavePlanEdits(patch);
      editingPlan = false;
    }}
    onCancel={() => (editingPlan = false)}
    onDelete={onDeletePlan}
  />
{/if}

<div class:editing-hidden={editingPlan} class="workspace-wrap">
  <section class="trip-head">
    {#if plan.cover_image_url}
      <img class="trip-cover" src={plan.cover_image_url} alt="" />
    {/if}
    <div class="head-left">
      <button class="back-btn" type="button" onclick={onBack}>
        <Icon name="arrow-left" size={13} /> All trips
      </button>
      <h2>{plan.title}</h2>
      <p class="route-summary">
        {placeName(plan.origin)} → {plan.destinations.map(placeName).join(" → ")}
        · <span class="mono">{shortDate(plan.date_start)} – {shortDate(plan.date_end)}</span>
      </p>
      <div class="trip-chips">
        {#each plan.transport_modes as mode (mode)}
          <Chip label={modeLabel(mode)} tone={MODE_TONE[mode] ?? "neutral"} />
        {/each}
        {#if plan.travelers.length > 0}
          <Chip label={plan.travelers.join(", ")} tone="neutral" />
        {/if}
      </div>
    </div>
    <div class="trip-head-actions">
      <button type="button" class="btn btn-secondary" onclick={() => (editingPlan = !editingPlan)}>
        <Icon name="pencil" size={13} />
        {editingPlan ? "Close editor" : "Edit trip"}
      </button>
      <a class="connection-link" href={link("/travel/connections")}>
        Find connection <Icon name="arrow-right" size={13} />
      </a>
    </div>
  </section>

  <!-- Progressive Disclosure Navigation Tabs -->
  <div class="domain-tabs" role="tablist" aria-label="Trip planning areas">
    <button
      type="button"
      role="tab"
      aria-selected={activeTab === "itinerary"}
      class:active={activeTab === "itinerary"}
      onclick={() => (activeTab = "itinerary")}
    >
      <Icon name="calendar" size={14} />
      Itinerary & Timeline
      <span class="pill mono">{items.length}</span>
    </button>
    <button
      type="button"
      role="tab"
      aria-selected={activeTab === "discovery"}
      class:active={activeTab === "discovery"}
      onclick={() => (activeTab = "discovery")}
    >
      <Icon name="compass" size={14} />
      Discovery & Connections
      {#if selected}
        <span class="pill mono">{selected.journeys.length + selected.events.length}</span>
      {/if}
    </button>
    <button
      type="button"
      role="tab"
      aria-selected={activeTab === "people"}
      class:active={activeTab === "people"}
      onclick={() => (activeTab = "people")}
    >
      <Icon name="users" size={14} />
      Who's Around
      <span class="pill mono">{knownPeople.length}</span>
    </button>
    <button
      type="button"
      role="tab"
      aria-selected={activeTab === "logistics"}
      class:active={activeTab === "logistics"}
      onclick={() => (activeTab = "logistics")}
    >
      <Icon name="boxes" size={14} />
      Logistics & Budget
      {#if planCostRollup && planCostRollup.planned_cents != null}
        <span class="pill mono">{planCostRollup.currency ?? ""} {Math.round(planCostRollup.planned_cents / 100)}</span>
      {/if}
    </button>
  </div>

  <!-- Tab 1: Itinerary & Stages -->
  {#if activeTab === "itinerary"}
    <div class="tab-panel">
      <TripItinerary
        {plan}
        {items}
        modes={modeOptions}
        stageLabel={stageStatusLabel}
        {onItemChanged}
        onRemoveItem={(item) => void onRemoveItem(item)}
        onUpdateStage={(id, patch) => void onUpdateStage(id, patch)}
        onToggleMode={onToggleStageMode}
      >
        {#snippet timeline()}
          <ItineraryTimeline
            {plan}
            {items}
            calendarEntries={planCalendarEntries}
            onRemoveItem={onRemoveItem}
            onUpdateItemDay={onUpdateItemDay}
          />
        {/snippet}
      </TripItinerary>
    </div>
  {/if}

  <!-- Tab 2: Discovery & Live Connections -->
  {#if activeTab === "discovery"}
    <div class="tab-panel discovery-tab">
      {#if results.length > 1}
        <nav class="destination-strip" aria-label="Trip destinations">
          {#each results as result (result.place.id)}
            <button
              type="button"
              class:active={result.place.id === selectedId}
              onclick={() => (selectedId = result.place.id)}
            >
              <span class="destination-mark"><Icon name="map-pin" size={15} /></span>
              <span class="destination-copy">
                <strong>{result.city}</strong>
                {#if result.loading}
                  <small>Checking sources…</small>
                {:else}
                  <small>
                    {result.anchors.length} anchors ·
                    {result.journeys.length} connections · {result.events.length} discoveries
                  </small>
                {/if}
              </span>
            </button>
          {/each}
        </nav>
      {/if}

      {#if selected}
        <header class="destination-head">
          <div>
            <span class="eyebrow">On location in</span>
            <h2>{selected.city}</h2>
          </div>
          <div class="destination-actions">
            {#if selected.image}
              <button
                type="button"
                class="btn btn-secondary"
                disabled={plan.cover_image_url === selected.image.url}
                onclick={() => void onUseCover(selected.image!)}
              >
                {plan.cover_image_url === selected.image.url ? "Current cover" : "Use as cover"}
              </button>
            {/if}
            <button
              type="button"
              class="btn btn-secondary"
              aria-expanded={mapOpen}
              onclick={() => (mapOpen = !mapOpen)}
            >
              <Icon name="map-pin" size={13} />
              {mapOpen ? "Close map" : "Show map"}
            </button>
          </div>
        </header>

        {#if climate}
          {@const found = climateResultFor(selected.place)}
          {#if found}
            <ClimateStrip
              result={found}
              rule={climate.best_months_rule}
              attribution={climate.attribution}
            />
          {/if}
        {/if}

        {#if selected.anchors.length > 0}
          <section class="calendar-anchors card" aria-labelledby="calendar-anchor-heading">
            <header class="anchors-head">
              <div>
                <span class="eyebrow">From your calendar</span>
                <h3 id="calendar-anchor-heading">Anchors for this trip</h3>
              </div>
              <a href={link("/calendar")} class="cal-link">Calendar <Icon name="arrow-right" size={11} /></a>
            </header>
            <ol class="anchors-list">
              {#each selected.anchors as entry (entry.id)}
                {@const entryUrl = calendarUrl(entry)}
                <li>
                  <time class="mono">{calendarTime(entry)}</time>
                  <div class="anchor-copy">
                    {#if entryUrl}
                      <a href={entryUrl} target="_blank" rel="noreferrer">{entry.title}</a>
                    {:else}
                      <strong>{entry.title}</strong>
                    {/if}
                    <span>{entry.location ?? selected.city}</span>
                  </div>
                  <Chip
                    label={commitmentLabel(entry.commitment)}
                    tone={entry.commitment === "must" ? "accent" : "neutral"}
                  />
                  <button
                    type="button"
                    class="btn btn-sm"
                    disabled={itemSaved("event", `calendar:${entry.id}`)}
                    onclick={() => void onSaveCalendarEvent(entry)}
                  >
                    <Icon
                      name={itemSaved("event", `calendar:${entry.id}`) ? "check" : "plus"}
                      size={12}
                    />
                    {itemSaved("event", `calendar:${entry.id}`) ? "In itinerary" : "Add"}
                  </button>
                </li>
              {/each}
            </ol>
          </section>
        {/if}

        {#if mapOpen}
          <div class="detail-map">
            <MapSurface
              sources={tripMapSources}
              layers={TRIP_LAYERS}
              fitKey={tripMapFitKey}
              deferredLabel="Trip map"
            />
          </div>
        {/if}

        {#if selected.loading}
          <div class="loading-block">
            <Icon name="loader" size={18} /> Combining connections and events…
          </div>
        {:else}
          <TripResultColumns
            {selected}
            activePlan={plan}
            bind:journeySort
            bind:journeyPriority
            bind:journeyPhrase
            bind:expandedJourneyId
            {itemSaved}
            {orderedJourneys}
            {onSaveJourney}
            {onSaveEvent}
            {onSavePlace}
          />
        {/if}
      {/if}
    </div>
  {/if}

  <!-- Tab 3: Who's Around (Friends & Companions) -->
  {#if activeTab === "people"}
    <div class="tab-panel">
      <section class="who-around card" aria-labelledby="who-around-heading">
        <header class="section-head">
          <div>
            <span class="eyebrow">Friend Graph</span>
            <h3 id="who-around-heading">Who's around along your route</h3>
          </div>
          <span class="mono count">{knownPeople.length} people nearby</span>
        </header>
        <WhoIsAround
          stages={plan.stages}
          {items}
          coordinates={legCoordinates}
          {locatedByDay}
          people={knownPeople}
          notice={peopleNotice}
          onChanged={onReloadWhoIsAround}
        />
      </section>
    </div>
  {/if}

  <!-- Tab 4: Logistics, Packing & Outfits -->
  {#if activeTab === "logistics"}
    <div class="tab-panel logistics-grid">
      {#if planCostRollup}
        <div class="cost-wrap">
          <CostCard cost={planCostRollup} />
        </div>
      {/if}

      <section class="logistics-card card" aria-labelledby="packing-heading">
        <h3 id="packing-heading">Packing Checklist</h3>
        <PackingPanel planId={plan.id} />
      </section>

      <section class="logistics-card card" aria-labelledby="outfits-heading">
        <h3 id="outfits-heading">Outfits & Wardrobe</h3>
        <OutfitsPanel planId={plan.id} dateStart={plan.date_start} dateEnd={plan.date_end} />
      </section>
    </div>
  {/if}
</div>

<style>
  .workspace-wrap {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .editing-hidden {
    display: none;
  }

  .trip-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-4);
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    position: relative;
    overflow: hidden;
  }

  .trip-cover {
    position: absolute;
    top: 0;
    right: 0;
    width: 45%;
    height: 100%;
    object-fit: cover;
    opacity: 0.15;
    mask-image: linear-gradient(to right, transparent, black);
    pointer-events: none;
  }

  .head-left {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    z-index: 1;
  }

  .back-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    background: none;
    border: none;
    color: var(--text-tertiary);
    font-size: var(--text-xs);
    cursor: pointer;
    padding: 0;
    margin-bottom: 0.25rem;
  }

  .back-btn:hover {
    color: var(--text-primary);
  }

  .trip-head h2 {
    margin: 0;
    font-size: var(--text-xl);
    color: var(--text-primary);
  }

  .route-summary {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }

  .trip-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    margin-top: 0.25rem;
  }

  .trip-head-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    z-index: 1;
  }

  .connection-link {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.4rem 0.75rem;
    background: var(--surface);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    text-decoration: none;
    font-size: var(--text-xs);
    font-weight: 500;
  }

  .connection-link:hover {
    background: var(--nav-hover);
  }

  .domain-tabs {
    display: flex;
    gap: var(--space-2);
    border-bottom: 1px solid var(--card-border);
    padding-bottom: var(--space-1);
    overflow-x: auto;
  }

  .domain-tabs button {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    background: none;
    border: none;
    padding: var(--space-2) var(--space-3);
    font-size: var(--text-sm);
    color: var(--text-secondary);
    cursor: pointer;
    border-bottom: 2px solid transparent;
    white-space: nowrap;
  }

  .domain-tabs button:hover {
    color: var(--text-primary);
  }

  .domain-tabs button.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
    font-weight: 500;
  }

  .pill {
    background: var(--surface);
    border: 1px solid var(--card-border);
    padding: 0.1rem 0.4rem;
    border-radius: var(--radius-full);
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .domain-tabs button.active .pill {
    background: var(--accent-soft);
    color: var(--accent);
    border-color: transparent;
  }

  .tab-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .card {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    padding: var(--space-4);
  }

  .section-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin-bottom: var(--space-3);
  }

  .eyebrow {
    font-size: var(--text-2xs);
    color: var(--accent);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    display: block;
  }

  .section-head h3 {
    margin: 0.2rem 0 0;
    font-size: var(--text-base);
  }

  .count {
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .destination-strip {
    display: flex;
    gap: var(--space-2);
    overflow-x: auto;
    padding-bottom: var(--space-2);
  }

  .destination-strip button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    cursor: pointer;
    text-align: left;
  }

  .destination-strip button.active {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .destination-copy {
    display: flex;
    flex-direction: column;
  }

  .destination-copy strong {
    font-size: var(--text-sm);
    color: var(--text-primary);
  }

  .destination-copy small {
    font-size: var(--text-2xs);
    color: var(--text-secondary);
  }

  .destination-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
  }

  .destination-head h2 {
    margin: 0.2rem 0 0;
    font-size: var(--text-lg);
  }

  .destination-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .calendar-anchors {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .anchors-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
  }

  .cal-link {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    font-size: var(--text-xs);
    color: var(--accent);
    text-decoration: none;
  }

  .anchors-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .anchors-list li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) 0;
    border-top: 1px solid var(--card-border);
  }

  .anchor-copy {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }

  .anchor-copy strong,
  .anchor-copy a {
    font-size: var(--text-sm);
    color: var(--text-primary);
    text-decoration: none;
  }

  .anchor-copy a:hover {
    color: var(--accent);
    text-decoration: underline;
  }

  .anchor-copy span {
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .detail-map {
    height: 18rem;
    border-radius: var(--radius-md);
    overflow: hidden;
    border: 1px solid var(--card-border);
  }

  .loading-block {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-6);
    color: var(--text-tertiary);
    font-size: var(--text-sm);
  }

  .logistics-grid {
    display: grid;
    grid-template-columns: 1fr;
    gap: var(--space-4);
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

  .btn-sm {
    padding: 0.25rem 0.5rem;
    font-size: var(--text-2xs);
    background: var(--surface);
    border: 1px solid var(--card-border);
    color: var(--text-primary);
  }

  .btn-sm:hover:not(:disabled) {
    background: var(--nav-hover);
  }

  .btn-secondary {
    background: var(--surface);
    border: 1px solid var(--card-border);
    color: var(--text-primary);
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--nav-hover);
  }
</style>
