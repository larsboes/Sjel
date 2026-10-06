<script lang="ts">
  import { eventItem } from "$lib/inspector/connections";
  import { onMount } from 'svelte';
  import { tip } from '$lib/tip';
  import Icon from '$lib/Icon.svelte';
  import { link } from '$lib/nav';
  import {
    entities,
    finance,
    interior,
    comms,
    type CalendarEntry,
    type MacmonSample,
    type TripPlan,
    type Entity,
    type LocatedPerson,
    type Burn,
    type InteriorLayoutSummary,
    type FeedEntry,
  } from '$lib/api';
  import { assistantStore } from '$lib/assistant/assistant.svelte';
  import { omniStore } from '$lib/omni/omni.svelte';
  import { inspectorStore } from '$lib/inspector/inspector.svelte';

  let {
    entries = [],
    plans = [],
    macmon = null,
  }: {
    entries: CalendarEntry[];
    plans: TripPlan[];
    macmon: MacmonSample | null;
  } = $props();

  const now = new Date();
  const todayStr = now.toISOString().slice(0, 10);
  const currentHour = now.getHours();

  const greeting = $derived(
    currentHour < 12
      ? "Good morning"
      : currentHour < 18
        ? "Good afternoon"
        : "Good evening"
  );

  // Background life signals loaded gracefully on mount
  let people = $state<Entity[]>([]);
  let located = $state<LocatedPerson[]>([]);
  let burn = $state<Burn | null>(null);
  let layouts = $state<InteriorLayoutSummary[]>([]);
  let feedItems = $state<FeedEntry[]>([]);

  onMount(() => {
    void Promise.allSettled([
      entities.list('person'),
      entities.located(todayStr),
      finance.burn(todayStr),
      interior.layouts(),
      comms.feed({ days: 7 }),
    ]).then(([peopleRes, locatedRes, burnRes, interiorRes, feedRes]) => {
      if (peopleRes.status === 'fulfilled') people = peopleRes.value;
      if (locatedRes.status === 'fulfilled') located = locatedRes.value.located;
      if (burnRes.status === 'fulfilled') burn = burnRes.value;
      if (interiorRes.status === 'fulfilled') layouts = interiorRes.value;
      if (feedRes.status === 'fulfilled') feedItems = feedRes.value;
    });
  });

  // Next event today
  const todayEntries = $derived(
    entries
      .filter((e) => e.starts_at.slice(0, 10) === todayStr)
      .sort((a, b) => a.starts_at.localeCompare(b.starts_at))
  );

  const nextEntry = $derived(todayEntries[0] ?? null);
  const upcomingTrip = $derived(plans[0] ?? null);
  const tripDestination = $derived(
    upcomingTrip?.destinations?.[0]?.name ?? upcomingTrip?.title ?? "Travel"
  );
  const tripDates = $derived(
    upcomingTrip ? `${upcomingTrip.date_start} – ${upcomingTrip.date_end}` : "Upcoming"
  );

  // System stats (discrete footnote)
  const cpuTemp = $derived(
    macmon?.temp?.cpu_temp_avg != null ? `${macmon.temp.cpu_temp_avg.toFixed(0)}°C` : null
  );

  const passingLayout = $derived(layouts.find((l) => l.pass) ?? layouts[0] ?? null);
  const burnMonthly = $derived(
    burn?.currencies?.[0]
      ? `${(burn.currencies[0].monthly_cents / 100).toFixed(0)} ${burn.currencies[0].currency}/mo`
      : null
  );

  function handleInspectNextEvent() {
    if (!nextEntry) return;
    inspectorStore.open(eventItem(nextEntry));
  }

  function handleInspectTrip() {
    if (!upcomingTrip) return;
    inspectorStore.inspectTrip({
      id: upcomingTrip.id,
      title: upcomingTrip.title,
      destination: tripDestination,
      dates: tripDates,
    });
  }
</script>

<aside class="life-pulse card" aria-label="Sjel Life Pulse">
  <!-- Pulse Header & Ambient Greeting -->
  <div class="pulse-top">
    <div class="greeting-wrap">
      <div class="pulse-dot-wrap">
        <span class="live-dot"></span>
      </div>
      <div>
        <h3 class="greeting-text">{greeting}</h3>
        <p class="pulse-status">
          {#if nextEntry}
            <span>Next: <strong>{nextEntry.title}</strong> at {nextEntry.starts_at.slice(11, 16) || "today"}</span>
          {:else if upcomingTrip}
            <span>Upcoming: <strong>{upcomingTrip.title}</strong> to {tripDestination}</span>
          {:else}
            <span>Household rhythm calm · All signals steady</span>
          {/if}
        </p>
      </div>
    </div>

    <div class="pulse-actions">
      <button
        type="button"
        class="ask-chip"
        onclick={() => assistantStore.openDrawer()}
        use:tip={"Ask Sjel Assistant"}
      >
        <Icon name="sparkles" size={13} />
        <span>Ask Sjel</span>
      </button>

      <button
        type="button"
        class="search-chip"
        onclick={() => omniStore.open()}
        use:tip={"Search across Sjel (⌘K)"}
      >
        <Icon name="search" size={13} />
        <kbd class="kbd-hint">⌘K</kbd>
      </button>
    </div>
  </div>

  <!-- Connected Life Synapses Mesh -->
  <div class="synapses-grid">
    <!-- Schedule Synapse -->
    <button
      type="button"
      class="synapse-card"
      onclick={nextEntry ? handleInspectNextEvent : undefined}
    >
      <div class="synapse-head">
        <span class="synapse-icon"><Icon name="calendar" size={14} /></span>
        <span class="synapse-tag">Schedule</span>
      </div>
      <div class="synapse-body">
        <strong class="synapse-title">
          {todayEntries.length > 0 ? `${todayEntries.length} event${todayEntries.length > 1 ? "s" : ""} today` : "Open agenda"}
        </strong>
        <span class="synapse-meta">
          {nextEntry ? nextEntry.title : "No scheduled conflicts"}
        </span>
      </div>
    </button>

    <!-- Travel & Horizons Synapse -->
    <button
      type="button"
      class="synapse-card"
      onclick={upcomingTrip ? handleInspectTrip : undefined}
    >
      <div class="synapse-head">
        <span class="synapse-icon"><Icon name="train" size={14} /></span>
        <span class="synapse-tag">Travel</span>
      </div>
      <div class="synapse-body">
        <strong class="synapse-title">
          {upcomingTrip ? tripDestination : "No active trip"}
        </strong>
        <span class="synapse-meta">
          {plans.length > 0 ? `${plans.length} plan${plans.length > 1 ? "s" : ""} in motion` : "Ready to plan"}
        </span>
      </div>
    </button>

    <!-- People & Presence Synapse -->
    <a class="synapse-card" href={link("/people")}>
      <div class="synapse-head">
        <span class="synapse-icon"><Icon name="users" size={14} /></span>
        <span class="synapse-tag">People</span>
      </div>
      <div class="synapse-body">
        <strong class="synapse-title">
          {located.length > 0 ? `${located.length} nearby` : "Household ring"}
        </strong>
        <span class="synapse-meta">
          {people.length > 0 ? `${people.length} entities connected` : "Address book"}
        </span>
      </div>
    </a>

    <!-- Finance & Calm Synapse -->
    <a class="synapse-card" href={link("/finance")}>
      <div class="synapse-head">
        <span class="synapse-icon"><Icon name="wallet" size={14} /></span>
        <span class="synapse-tag">Finance</span>
      </div>
      <div class="synapse-body">
        <strong class="synapse-title">
          {burnMonthly ?? "Ledger active"}
        </strong>
        <span class="synapse-meta">Burn rate balanced</span>
      </div>
    </a>
  </div>

  <!-- Ambient Footer: Discreet Node & Health Indicator -->
  <div class="pulse-footer">
    <div class="footer-left">
      <span class="ambient-pill">
        <span class="ambient-indicator"></span>
        <span>Local node active</span>
      </span>
      {#if cpuTemp}
        <a class="system-link" href={link("/systems")}>
          <Icon name="activity" size={11} />
          <span>{cpuTemp}</span>
        </a>
      {/if}
    </div>

    <div class="footer-right">
      {#if passingLayout}
        <a class="context-link" href={link("/interior")}>
          <Icon name="layout" size={12} />
          <span>Interior: {passingLayout.name}</span>
        </a>
      {/if}
      {#if feedItems.length > 0}
        <a class="context-link" href={link("/feed")}>
          <Icon name="feed" size={12} />
          <span>{feedItems.length} unread</span>
        </a>
      {/if}
    </div>
  </div>
</aside>

<style>
  .life-pulse {
    position: relative;
    overflow: hidden;
    padding: var(--space-4) var(--space-5);
    margin-bottom: var(--space-4);
    background:
      radial-gradient(130% 100% at 100% 0%, var(--primary-soft) 0%, transparent 65%),
      radial-gradient(90% 80% at 0% 100%, var(--surface) 0%, transparent 60%),
      var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--card-shadow);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .pulse-top {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--space-4);
  }

  .greeting-wrap {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .pulse-dot-wrap {
    display: grid;
    place-items: center;
  }

  .live-dot {
    width: 8px;
    height: 8px;
    border-radius: var(--radius-full);
    background-color: var(--primary);
    box-shadow: 0 0 10px var(--primary);
    animation: gentle-pulse 3s infinite ease-in-out;
  }

  @keyframes gentle-pulse {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: 0.6; transform: scale(0.92); }
  }

  .greeting-text {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text-primary);
  }

  .pulse-status {
    margin: 0.1rem 0 0;
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .pulse-status strong {
    color: var(--text-primary);
    font-weight: 600;
  }

  .pulse-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .ask-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--primary);
    background-color: var(--primary-soft);
    padding: 0.35rem 0.75rem;
    border-radius: var(--radius-full);
    border: 1px solid transparent;
    cursor: pointer;
    transition:
      background-color var(--motion-fast) var(--ease-out),
      color var(--motion-fast) var(--ease-out),
      transform var(--motion-fast) var(--ease-out);
  }

  .ask-chip:hover {
    background-color: var(--primary);
    color: var(--text-inverse);
    transform: translateY(-1px);
  }

  .ask-chip:active {
    transform: scale(0.96);
  }

  .search-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-secondary);
    background-color: var(--surface);
    padding: 0.35rem 0.6rem;
    border-radius: var(--radius-full);
    border: 1px solid var(--card-border);
    cursor: pointer;
    transition: border-color var(--motion-fast) ease, color var(--motion-fast) ease;
  }

  .search-chip:hover {
    border-color: var(--card-border-hover);
    color: var(--text-primary);
  }

  .kbd-hint {
    font-size: var(--text-2xs);
    font-family: inherit;
    color: var(--text-tertiary);
  }

  /* Connected Synapses Grid */
  .synapses-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--space-3);
  }

  .synapse-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-md);
    background: var(--surface);
    border: 1px solid var(--card-border);
    text-decoration: none;
    text-align: left;
    cursor: pointer;
    transition:
      background-color var(--motion-base) var(--ease-out),
      border-color var(--motion-base) var(--ease-out),
      transform var(--motion-base) var(--ease-out),
      box-shadow var(--motion-base) var(--ease-out);
  }

  .synapse-card:hover {
    background: var(--card-bg);
    border-color: var(--card-border-hover);
    transform: translateY(-2px);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.05);
  }

  .synapse-card:active {
    transform: scale(0.98);
  }

  .synapse-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .synapse-icon {
    color: var(--primary);
    display: grid;
    place-items: center;
  }

  .synapse-tag {
    font-size: var(--text-2xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-tertiary);
  }

  .synapse-body {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  .synapse-title {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .synapse-meta {
    font-size: var(--text-2xs);
    color: var(--text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Pulse Footer */
  .pulse-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-top: var(--space-2);
    border-top: 1px solid var(--card-border);
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .footer-left, .footer-right {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .ambient-pill {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    color: var(--text-secondary);
  }

  .ambient-indicator {
    width: 5px;
    height: 5px;
    border-radius: var(--radius-full);
    background-color: var(--success);
  }

  .system-link, .context-link {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    color: var(--text-tertiary);
    text-decoration: none;
    transition: color var(--motion-fast) ease;
  }

  .system-link:hover, .context-link:hover {
    color: var(--primary);
  }

  @media (max-width: 768px) {
    .synapses-grid {
      grid-template-columns: repeat(2, 1fr);
    }
  }

  @media (max-width: 480px) {
    .pulse-top {
      flex-direction: column;
      align-items: flex-start;
    }

    .pulse-actions {
      width: 100%;
      justify-content: flex-end;
    }

    .synapses-grid {
      grid-template-columns: 1fr;
    }

    .pulse-footer {
      flex-direction: column;
      align-items: flex-start;
      gap: var(--space-2);
    }
  }
</style>
