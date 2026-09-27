<script lang="ts">
  import { link } from "$lib/nav";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import RelatedTools from "$lib/RelatedTools.svelte";
  import StationField from "$lib/travel/StationField.svelte";
  import JourneyOption from "$lib/travel/JourneyOption.svelte";
  import TravelerProfileModal from "$lib/travel/TravelerProfileModal.svelte";
  import {
    axonStatus,
    transit,
    JOURNEY_PRIORITIES,
    type Journey,
    type JourneyOverride,
    type SplitResult,
    type Station,
    type TrainMatch,
  } from "$lib/api";

  function tomorrow(): string {
    const d = new Date();
    d.setDate(d.getDate() + 1);
    return [
      d.getFullYear(),
      String(d.getMonth() + 1).padStart(2, "0"),
      String(d.getDate()).padStart(2, "0"),
    ].join("-");
  }

  let from = $state<Station | null>(null);
  let to = $state<Station | null>(null);
  let date = $state(tomorrow());
  let time = $state("10:00");

  let loading = $state(false);
  let error = $state<string | null>(null);
  let searched = $state(false);
  let journeys = $state<Journey[]>([]);
  let split = $state<SplitResult | null>(null);
  let splitNote = $state<string | null>(null);
  let tab = $state<"direct" | "split">("direct");
  let journeySort = $state<"ranked" | "price" | "duration" | "departure">("ranked");
  let journeyPriority = $state("");
  let journeyPhrase = $state("");
  let expandedJourneyId = $state<string | null>(null);
  let profileOpen = $state(false);

  function journeyOverride(): JourneyOverride | undefined {
    const phrase = journeyPhrase.trim();
    if (phrase) return { phrase };
    if (journeyPriority) return { priority: journeyPriority };
    return undefined;
  }

  function orderedJourneys(list: Journey[]): Journey[] {
    return [...list].sort((a, b) => {
      if (journeySort === "ranked") {
        return (
          (a.ranking?.rank ?? Number.POSITIVE_INFINITY) -
          (b.ranking?.rank ?? Number.POSITIVE_INFINITY)
        );
      }
      if (journeySort === "duration") {
        return a.total_duration_minutes - b.total_duration_minutes;
      }
      if (journeySort === "departure") {
        return (a.legs[0]?.departure_time ?? "").localeCompare(b.legs[0]?.departure_time ?? "");
      }
      return (
        (a.total_price ?? Number.POSITIVE_INFINITY) -
        (b.total_price ?? Number.POSITIVE_INFINITY)
      );
    });
  }

  // transit declares itself on-demand, and this page is the only surface that uses it —
  // the station fields above resolve names through it and the search below queries it.
  // Nothing else on /travel starts it, so without this the page renders a complete form
  // whose lookups quietly return nothing: a working-looking page that cannot answer.
  // Reads no state, so it runs once on mount. A failure here is not fatal on its own;
  // the lookup or search that follows reports the real error.
  $effect(() => {
    void axonStatus.start("transit").catch(() => undefined);
  });

  async function search(): Promise<void> {
    if (!from || !to) {
      error = "Select an origin and destination.";
      return;
    }
    loading = true;
    error = null;
    journeys = [];
    split = null;
    splitNote = null;

    const when = `${date}T${time}:00`;
    try {
      // The split search is a second opinion on the same query, not a precondition for
      // showing the first: transit answers 404 for a route with no cheaper combination,
      // which is an outcome to report rather than a failed search, and letting it reject
      // here would throw away the direct connections that did arrive.
      const override = journeyOverride();
      const [direct, cheaper] = await Promise.all([
        transit.search(from.id, to.id, when, override),
        transit.split(from.id, to.id, when).catch((e: unknown) => {
          splitNote = e instanceof Error ? e.message : String(e);
          return null;
        }),
      ]);
      journeys = direct;
      split = cheaper;
      searched = true;
      tab = "direct";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  function swap(): void {
    [from, to] = [to, from];
  }

  const hhmm = (iso: string) =>
    new Date(iso).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });

  function duration(minutes: number): string {
    const h = Math.floor(minutes / 60);
    return h > 0 ? `${h} h ${minutes % 60} min` : `${minutes} min`;
  }

  // Regional legs come back without a fare, so a missing price is normal and says
  // "not priced here", never "free".
  const price = (value: number | null | undefined) =>
    value == null ? "—" : value.toLocaleString("en-GB", { style: "currency", currency: "EUR" });

  // What the number actually is: the share of this train type's stops at the destination,
  // in this arrival hour, that ran at least six minutes off schedule -- measured over
  // seven months by capabilities/punctuality. Six minutes is DB's own threshold. It is
  // not the chance the trip works out: it says nothing about catching a transfer. The
  // label says "late" rather than "risk" for exactly that reason, and the tooltip
  // carries the rest. Absent for a train type with too few observations to mean anything.
  function risk(score: number | null): { text: string; title: string; level: string } | null {
    if (score == null) return null;
    const pct = Math.round(score * 100);
    const title =
      `${pct}% of stops by this train type at this station during this hour ` +
      `were at least 6 minutes late (measured over 7 months). ` +
      `This says nothing about connections.`;
    const level = score > 0.6 ? "high" : score > 0.35 ? "mid" : "low";
    return { text: `${pct}% late`, title, level };
  }

  const savedPercent = (s: SplitResult) =>
    s.savings !== null && s.original_price !== null && s.original_price > 0
      ? Math.round((s.savings / s.original_price) * 100)
      : null;

  // Why a chain might not be buyable as shown. Each segment is priced by its own
  // fare search, and nothing forces that search to return the train you are on.
  const CONFIDENCE_NOTE: Record<SplitResult["confidence"], string | null> = {
    exact: null,
    partial:
      "Some tickets could not be matched to the exact train on this route, " +
      "or a fare lookup failed. Check each ticket before buying.",
    low:
      "At least one ticket is priced for a different train than this journey uses. " +
      "Buying this chain does not buy a seat on this trip.",
  };

  const MATCH_LABEL: Record<TrainMatch, string | null> = {
    exact: null,
    partial: "partial train match",
    different: "different train",
    unknown: "train unknown",
  };
</script>

<PageHeader
  badge="Travel · Connection"
  title="Find a connection"
  desc="Direct connections and a split-ticket calculation for the same route."
/>

<nav class="travel-nav" aria-label="Travel sections">
  <a href={link("/travel")}><Icon name="map-pin" size={14} /> Trip plans</a>
  <a class="active" href={link("/travel/connections")}><Icon name="train" size={14} /> Connections</a>
  <button type="button" class="nav-btn" onclick={() => (profileOpen = true)}>
    <Icon name="compass" size={14} /> Profile & preferences
  </button>
</nav>

{#if profileOpen}
  <TravelerProfileModal onClose={() => (profileOpen = false)} />
{/if}

<form
  class="card controls"
  onsubmit={(e) => {
    e.preventDefault();
    void search();
  }}
>
  <div class="route">
    <StationField label="From" placeholder="Origin station" bind:station={from} />
    <button class="btn swap" type="button" onclick={swap} aria-label="Swap origin and destination">
      <Icon name="swap" size={15} />
    </button>
    <StationField label="To" placeholder="Destination station" bind:station={to} />
  </div>

  <div class="when">
    <label>
      <span>Date</span>
      <span class="wrap">
        <Icon name="calendar" size={14} />
        <input class="input" type="date" bind:value={date} />
      </span>
    </label>
    <label>
      <span>Time</span>
      <span class="wrap">
        <Icon name="clock" size={14} />
        <input class="input" type="time" bind:value={time} />
      </span>
    </label>
  </div>

  <button class="btn btn-primary go" type="submit" disabled={loading}>
    {#if loading}<Icon name="loader" size={14} /> searching…{:else}<Icon name="search" size={14} /> Search{/if}
  </button>
</form>

<RelatedTools
  context="rail-search"
  title="Other rail tools"
  description="Specialised companions and the projects behind Sjel's split-ticket ideas."
/>

{#if error}
  <p class="notice"><Icon name="alert" size={15} /> {error}</p>
{/if}

{#if searched}
  <div class="tabs">
    <button class="tab" class:active={tab === "direct"} onclick={() => (tab = "direct")}>
      Direct ({journeys.length})
    </button>
    <button class="tab" class:active={tab === "split"} onclick={() => (tab = "split")}>
      <Icon name="ticket" size={13} /> Split-Ticket
    </button>
  </div>

  {#if tab === "direct"}
    {#if journeys.length === 0}
      <p class="empty card">No connections at this time.</p>
      <div class="direct-controls">
        <div class="section-heading">
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
              title="the weights stored on your profile"
              onclick={() => {
                journeyPriority = "";
                journeyPhrase = "";
                void search();
              }}
            >
              My usual
            </button>
            {#each JOURNEY_PRIORITIES as preset (preset.id)}
              <button
                class:active={journeyPriority === preset.id && !journeyPhrase.trim()}
                type="button"
                title={preset.hint}
                onclick={() => {
                  journeyPriority = preset.id;
                  journeyPhrase = "";
                  void search();
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
            onkeydown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void search();
              }
            }}
            placeholder="or say it: cheapest, direct, schnell und zuverlässig"
          />
        </div>
      </div>

      <ol class="journey-list">
        {#each orderedJourneys(journeys) as j (j.id)}
          <JourneyOption
            journey={j}
            expanded={expandedJourneyId === j.id}
            onToggle={() => (expandedJourneyId = expandedJourneyId === j.id ? null : j.id)}
          />
        {/each}
      </ol>
    {/if}
  {:else if split}
    <section class="card panel">
      <header>
        <div>
          <h2>Split-Ticket</h2>
          <p class="sub">The same journey split into separately purchased sections.</p>
        </div>
        {#if split.savings === null}
          <span class="tag mono" title="No direct fare came back, so there is nothing to compare against.">
            No direct fare to compare
          </span>
        {:else if split.savings > 0.01}
          <span class="tag win mono">{savedPercent(split)}% cheaper</span>
        {:else}
          <span class="tag mono">Direct is cheapest</span>
        {/if}
      </header>

      <div class="compare">
        <div>
          <span class="cap">Direct</span>
          <span class="mono old">{price(split.original_price)}</span>
        </div>
        <div>
          <span class="cap accent">Split</span>
          <span class="mono new">{price(split.split_price)}</span>
        </div>
      </div>

      {#if CONFIDENCE_NOTE[split.confidence]}
        <p class="chain-warn" class:severe={split.confidence === "low"}>
          {CONFIDENCE_NOTE[split.confidence]}
          {#if split.unpriced_pairs > 0}
            ({split.unpriced_pairs} of {split.queried_pairs} fare lookups returned nothing.)
          {/if}
        </p>
      {/if}

      <ol class="segments">
        {#each split.segments as seg, i (seg.journey.id)}
          <li class="card">
            <div class="seg-head">
              <span class="cap">Ticket {i + 1}</span>
              <span class="mono fare">{price(seg.journey.total_price)}</span>
            </div>
            <p class="seg-route">
              {seg.journey.start_station.name} → {seg.journey.end_station.name}
            </p>
            <p class="seg-meta mono">
              departs {hhmm(seg.journey.legs[0].departure_time)} · {duration(
                seg.journey.total_duration_minutes,
              )}
            </p>
            {#if MATCH_LABEL[seg.train_match]}
              <p
                class="seg-match"
                class:severe={seg.train_match === "different"}
                title={seg.expected_trains.length
                  ? `This journey uses ${seg.expected_trains.join(", ")}.`
                  : "The direct journey carried no train number to compare against."}
              >
                {MATCH_LABEL[seg.train_match]}
              </p>
            {/if}
          </li>
        {/each}
      </ol>

      {#if split.confidence === "low"}
        <p class="empty">
          No booking link while a ticket is priced for a different train. Search the route
          again, or buy the direct connection.
        </p>
      {:else}
        <a class="btn book" href="https://www.bahn.de" target="_blank" rel="noreferrer">
          Book on bahn.de <Icon name="external" size={13} />
        </a>
      {/if}
    </section>
  {:else}
    <p class="empty card">{splitNote ?? "No split calculation for this route."}</p>
  {/if}
{/if}

<style>
  .travel-nav {
    display: flex;
    gap: 0.25rem;
    margin: -0.5rem 0 1.25rem;
    padding-bottom: 0.65rem;
    border-bottom: 1px solid var(--card-border);
  }

  .travel-nav a,
  .travel-nav .nav-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.45rem 0.65rem;
    border: 0;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .travel-nav a:hover,
  .travel-nav a.active,
  .travel-nav .nav-btn:hover {
    color: var(--primary);
    background: var(--primary-soft);
  }

  .controls {
    display: grid;
    gap: 1rem;
    padding: 1.25rem;
    margin-bottom: 1.5rem;
  }

  .route {
    display: grid;
    gap: 0.75rem;
  }

  .swap {
    justify-self: start;
    padding: 0.4rem;
    color: var(--text-tertiary);
  }

  .when {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.75rem;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .wrap {
    position: relative;
    display: block;
  }

  .wrap :global(svg) {
    position: absolute;
    left: 0.7rem;
    top: 50%;
    transform: translateY(-50%);
    color: var(--text-tertiary);
    pointer-events: none;
  }

  .when input {
    padding-left: 2rem;
  }

  .go {
    padding-block: 0.6rem;
  }

  .notice {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0 0 1.25rem;
    padding: 0.75rem;
    border-radius: var(--radius-md);
    background-color: var(--danger-soft);
    color: var(--danger);
    font-size: var(--text-sm);
  }

  .tabs {
    display: flex;
    gap: 1.25rem;
    margin-bottom: 1rem;
    border-bottom: 1px solid var(--card-border);
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0 0 0.6rem;
    border: 0;
    border-bottom: 2px solid transparent;
    background: none;
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--text-secondary);
    cursor: pointer;
    transition:
      color 0.15s ease,
      border-color 0.15s ease;
  }

  .tab:hover {
    color: var(--text-primary);
  }

  .tab.active {
    color: var(--primary);
    border-bottom-color: var(--primary);
  }

  .empty {
    margin: 0;
    padding: 2rem;
    text-align: center;
    font-size: var(--text-sm);
    color: var(--text-tertiary);
  }

  .segments {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .direct-controls {
    margin-bottom: 0.75rem;
  }

  .section-heading {
    display: flex;
    justify-content: flex-end;
    margin-bottom: 0.5rem;
  }

  .journey-sort {
    display: inline-flex;
    padding: 0.15rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--surface);
  }

  .journey-sort button {
    padding: 0.25rem 0.5rem;
    border: 0;
    border-radius: calc(var(--radius-sm) - 2px);
    background: transparent;
    color: var(--text-tertiary);
    font: inherit;
    font-size: var(--text-xs);
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .journey-sort button:hover {
    color: var(--text-primary);
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
    transition: all 0.15s ease;
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

  .journey-list {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background: var(--card-bg);
  }

  .fare {
    font-size: 1rem;
    font-weight: 700;
    color: var(--primary);
  }

  .panel {
    padding: 1.25rem;
    animation: fade-up 0.3s ease-out both;
  }

  .panel header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 0.75rem;
  }

  h2 {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
  }

  .sub {
    margin: 0.15rem 0 0;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .win {
    background-color: var(--accent-soft);
    color: var(--accent);
  }

  .compare {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1rem;
    margin: 1rem 0;
    padding: 0.85rem 0;
    border-block: 1px solid var(--card-border);
  }

  .cap {
    display: block;
    font-size: 0.625rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-tertiary);
  }

  .cap.accent {
    color: var(--accent);
  }

  .old {
    font-size: 1rem;
    color: var(--text-secondary);
    text-decoration: line-through;
  }

  .new {
    font-size: 1.375rem;
    font-weight: 700;
    color: var(--accent);
  }

  .segments {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .segments li {
    padding: 0.75rem;
  }

  .seg-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .seg-route {
    margin: 0.3rem 0 0;
    font-size: var(--text-sm);
    font-weight: 500;
  }

  .seg-meta {
    margin: 0.2rem 0 0;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .seg-match {
    margin: 0.35rem 0 0;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .seg-match.severe,
  .chain-warn.severe {
    color: var(--danger, #b3261e);
    font-weight: 500;
  }

  .chain-warn {
    margin: 0 0 0.75rem;
    font-size: var(--text-xs);
    line-height: 1.45;
    color: var(--text-secondary);
  }

  .book {
    width: 100%;
    margin-top: 1rem;
    padding-block: 0.6rem;
    border: 1px solid var(--card-border);
  }

  @media (width >= 48rem) {
    .route {
      grid-template-columns: 1fr auto 1fr;
      align-items: end;
    }

    .swap {
      margin-bottom: 0.25rem;
    }
  }
</style>
