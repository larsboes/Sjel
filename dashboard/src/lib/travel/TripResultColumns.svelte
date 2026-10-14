<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import { link } from "$lib/nav";
  import { tip } from "$lib/tip";
  import {
    JOURNEY_PRIORITIES,
    type Journey,
    type PlaceRef,
    type ScoredResult,
    type TripPlan,
  } from "$lib/api";
  import JourneyOption from "$lib/travel/JourneyOption.svelte";
  import type { NearbyPlace } from "$lib/travel/nearby-places";

  export interface DestinationResult {
    place: PlaceRef;
    city: string;
    image: unknown;
    journeys: Journey[];
    anchors: unknown[];
    events: ScoredResult[];
    activities: NearbyPlace[];
    loading: boolean;
    notices: string[];
  }

  interface Props {
    selected: DestinationResult;
    activePlan: TripPlan;
    journeySort?: "ranked" | "price" | "duration" | "departure";
    journeyPriority?: string;
    journeyPhrase?: string;
    expandedJourneyId?: string | null;
    itemSaved: (type: "transport" | "event" | "activity", id: string) => boolean;
    orderedJourneys: (journeys: Journey[]) => Journey[];
    onSaveJourney: (journey: Journey, place: PlaceRef) => void;
    onSaveEvent: (event: ScoredResult) => void;
    onSavePlace: (activity: NearbyPlace) => void;
  }

  let {
    selected,
    activePlan,
    journeySort = $bindable("ranked"),
    journeyPriority = $bindable(""),
    journeyPhrase = $bindable(""),
    expandedJourneyId = $bindable(null),
    itemSaved,
    orderedJourneys,
    onSaveJourney,
    onSaveEvent,
    onSavePlace,
  }: Props = $props();

  const shortDate = (date: string | null) =>
    date
      ? new Intl.DateTimeFormat("en-GB", { day: "2-digit", month: "short" }).format(
          new Date(`${date.slice(0, 10)}T12:00:00`),
        )
      : "open";

  function dateRangeLabel(from: string, to: string): string {
    return from === to ? shortDate(from) : `${shortDate(from)} – ${shortDate(to)}`;
  }
</script>

{#if selected.notices.length > 0}
  <p class="source-notice">{selected.notices.join(" · ")}</p>
{/if}

<div class="results-grid">
  <section class="result-column">
    <div class="section-heading">
      <h3>Outbound journey</h3>
      <div class="journey-sort" aria-label="Sort connections">
        <button
          class:active={journeySort === "ranked"}
          type="button"
          onclick={() => (journeySort = "ranked")}
        >
          Ranked
        </button>
        <button
          class:active={journeySort === "price"}
          type="button"
          onclick={() => (journeySort = "price")}
        >
          Price
        </button>
        <button
          class:active={journeySort === "duration"}
          type="button"
          onclick={() => (journeySort = "duration")}
        >
          Duration
        </button>
        <button
          class:active={journeySort === "departure"}
          type="button"
          onclick={() => (journeySort = "departure")}
        >
          Departure
        </button>
      </div>
    </div>
    <div class="priority-bar">
      <span class="priority-label">Rank by</span>
      <div class="sort-options">
        <button
          class:active={!journeyPriority && !journeyPhrase.trim()}
          type="button"
          use:tip={"the weights stored on your profile"}
          onclick={() => {
            journeyPriority = "";
            journeyPhrase = "";
          }}
        >
          My usual
        </button>
        {#each JOURNEY_PRIORITIES as preset (preset.id)}
          <button
            class:active={journeyPriority === preset.id && !journeyPhrase.trim()}
            type="button"
            use:tip={preset.hint}
            onclick={() => {
              journeyPriority = preset.id;
              journeyPhrase = "";
            }}
          >
            {preset.label}
          </button>
        {/each}
      </div>
      <input
        class="input priority-phrase"
        bind:value={journeyPhrase}
        aria-label="Rank journeys by a sentence"
        placeholder="or say it: cheapest, direct, schnell und zuverlässig"
      />
    </div>

    {#if selected.journeys.length === 0}
      <p class="empty">
        No suitable connection received.
        <a href={link("/travel/connections")}>Open connection search</a>
      </p>
    {:else}
      <ol class="journey-list">
        {#each orderedJourneys(selected.journeys) as journey (journey.id)}
          <JourneyOption
            {journey}
            expanded={expandedJourneyId === journey.id}
            saved={itemSaved("transport", journey.id)}
            onToggle={() =>
              (expandedJourneyId =
                expandedJourneyId === journey.id ? null : journey.id)}
            onSave={() => void onSaveJourney(journey, selected.place)}
          />
        {/each}
      </ol>
    {/if}
  </section>

  <section class="result-column">
    <div class="section-heading">
      <h3>Other events</h3>
      <span>{dateRangeLabel(activePlan.date_start, activePlan.date_end)}</span>
    </div>
    {#if selected.events.length === 0}
      <p class="empty">Nothing else was found during the trip period.</p>
    {:else}
      <ol class="event-list">
        {#each selected.events as event (event.url)}
          <li>
            <time>{shortDate(event.date)}</time>
            <div>
              <a href={event.url} target="_blank" rel="noreferrer">{event.title}</a>
              <span>{event.city ?? event.location ?? selected.city}</span>
              <details class="event-why">
                <summary>Why this result?</summary>
                <p>{event.rationale}</p>
              </details>
            </div>
            <button
              class="save"
              type="button"
              disabled={itemSaved("event", event.url)}
              onclick={() => void onSaveEvent(event)}
              aria-label={`Save ${event.title}`}
            >
              <Icon name={itemSaved("event", event.url) ? "check" : "plus"} size={13} />
            </button>
          </li>
        {/each}
      </ol>
    {/if}
  </section>

  <section class="result-column activity-column">
    <div class="section-heading">
      <h3>Places and activities</h3>
      <span>Wikipedia · nearby</span>
    </div>
    {#if selected.activities.length === 0}
      <p class="empty">
        This place has no coordinates, or no nearby places were found.
      </p>
    {:else}
      <ol class="activity-list">
        {#each selected.activities as activity (activity.id)}
          <li>
            {#if activity.imageUrl}
              <img src={activity.imageUrl} alt="" />
            {:else}
              <span class="activity-image"><Icon name="compass" size={18} /></span>
            {/if}
            <div>
              <a href={activity.url} target="_blank" rel="noreferrer">{activity.title}</a>
              <p>{activity.description || "Place worth seeing nearby"}</p>
            </div>
            <button
              class="save"
              type="button"
              disabled={itemSaved("activity", activity.id)}
              onclick={() => void onSavePlace(activity)}
              aria-label={`Save ${activity.title}`}
            >
              <Icon
                name={itemSaved("activity", activity.id) ? "check" : "plus"}
                size={13}
              />
            </button>
          </li>
        {/each}
      </ol>
    {/if}
  </section>
</div>

<style>
  .source-notice {
    margin: 0 0 0.75rem;
    color: var(--warning-ink);
    font-size: var(--text-2xs);
  }

  .results-grid {
    display: grid;
    gap: 1rem;
  }

  .result-column {
    min-width: 0;
  }

  .section-heading {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.75rem;
    margin-bottom: 0.65rem;
  }

  .section-heading h3 {
    margin: 0;
    font-size: 0.875rem;
  }

  .section-heading span {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    font-size: 0.625rem;
  }

  .journey-sort {
    display: inline-flex;
    padding: 0.15rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--surface);
  }

  .journey-sort button {
    padding: 0.25rem 0.4rem;
    border: 0;
    border-radius: calc(var(--radius-sm) - 2px);
    background: transparent;
    color: var(--text-tertiary);
    font: inherit;
    font-size: 0.625rem;
    cursor: pointer;
  }

  .journey-sort button.active {
    background: var(--card-bg);
    color: var(--text-primary);
    box-shadow: 0 1px 2px rgb(0 0 0 / 8%);
  }

  .priority-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.75rem;
  }

  .priority-label {
    font-size: var(--text-xs);
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .sort-options {
    display: inline-flex;
    gap: 0.25rem;
    padding: 0.15rem;
    background: var(--surface);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
  }

  .sort-options button {
    padding: 0.25rem 0.5rem;
    border: 0;
    border-radius: calc(var(--radius-sm) - 2px);
    background: transparent;
    color: var(--text-tertiary);
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 500;
    cursor: pointer;
    transition: background-color var(--motion-fast) ease, box-shadow var(--motion-fast) ease, color var(--motion-fast) ease;
  }

  .sort-options button:hover {
    color: var(--text-primary);
  }

  .sort-options button.active {
    background: var(--card-bg);
    color: var(--text-primary);
    box-shadow: 0 1px 2px rgb(0 0 0 / 8%);
  }

  .priority-phrase {
    flex: 1 1 16rem;
    min-width: 12rem;
  }

  .journey-list,
  .event-list,
  .activity-list {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background: var(--card-bg);
  }

  .event-list li:last-child,
  .activity-list li:last-child {
    border-bottom: 0;
  }

  .save {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    margin-top: 0.2rem;
    padding: 0.2rem 0.35rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
    color: var(--primary);
    font: inherit;
    font-size: 0.625rem;
    font-weight: 600;
    cursor: pointer;
  }

  .save:disabled {
    color: var(--success);
    cursor: default;
  }

  .event-list li {
    display: grid;
    grid-template-columns: 3rem 1fr auto;
    gap: 0.65rem;
    align-items: start;
    padding: 0.75rem;
    border-bottom: 1px solid var(--card-border);
  }

  .event-list time {
    color: var(--primary);
    font-family: var(--font-mono);
    font-size: 0.625rem;
    font-weight: 600;
  }

  .event-list a {
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .event-list a:hover {
    color: var(--primary);
  }

  .event-list p {
    display: -webkit-box;
    overflow: hidden;
    margin: 0.15rem 0;
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }

  .event-list span {
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .event-why {
    margin-top: 0.25rem;
  }

  .event-why summary {
    color: var(--text-tertiary);
    font-size: 0.625rem;
    cursor: pointer;
  }

  .event-why p {
    margin-top: 0.25rem;
  }

  .event-list .save {
    margin: 0;
  }

  .activity-list li {
    display: grid;
    grid-template-columns: 4.5rem minmax(0, 1fr) auto;
    gap: 0.65rem;
    align-items: start;
    min-height: 4.5rem;
    padding: 0.65rem;
    border-bottom: 1px solid var(--card-border);
  }

  .activity-list img,
  .activity-image {
    width: 4.5rem;
    height: 3.6rem;
    border-radius: var(--radius-sm);
    object-fit: cover;
  }

  .activity-image {
    display: grid;
    place-items: center;
    background: var(--primary-soft);
    color: var(--primary);
  }

  .activity-list a {
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .activity-list p {
    display: -webkit-box;
    overflow: hidden;
    margin: 0.2rem 0 0;
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }

  .empty {
    margin: 0;
    padding: 1rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  .empty a {
    display: inline-block;
    margin-left: 0.25rem;
    color: var(--primary);
  }

  @media (width >= 54rem) {
    .results-grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }

    .activity-column {
      grid-column: 1 / -1;
    }
  }
</style>
