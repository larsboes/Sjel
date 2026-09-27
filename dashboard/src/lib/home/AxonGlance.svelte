<script lang="ts">
  import { onMount } from 'svelte';
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


  // System stats
  const cpuTemp = $derived(
    macmon?.temp?.cpu_temp_avg != null ? `${macmon.temp.cpu_temp_avg.toFixed(0)}°C` : null
  );
  const ramUsage = $derived(
    macmon?.memory?.ram_usage != null && macmon.memory.ram_total != null
      ? `${(macmon.memory.ram_usage / 1073741824).toFixed(1)} / ${(macmon.memory.ram_total / 1073741824).toFixed(0)} GB`
      : null
  );

  const passingLayout = $derived(layouts.find((l) => l.pass) ?? layouts[0] ?? null);
  const burnMonthly = $derived(
    burn?.currencies?.[0]
      ? `${(burn.currencies[0].monthly_cents / 100).toFixed(0)} ${burn.currencies[0].currency}/mo`
      : null
  );
</script>

<aside class="axon-glance card" aria-label="Sjel Integrated Life Cockpit">
  <div class="glance-top">
    <div class="glance-title">
      <span class="live-dot"></span>
      <Icon name="sparkles" size={14} />
      <strong>Sjel Life Cockpit</strong>
      <span class="sep">·</span>
      <span class="date-context">
        {now.toLocaleDateString("en-GB", { weekday: "short", day: "numeric", month: "short" })}
      </span>
    </div>

    <div class="glance-actions">
      {#if cpuTemp || ramUsage}
        <a class="hw-chip mono" href={link('/systems')} title="View live system monitor">
          <Icon name="activity" size={12} />
          {#if cpuTemp}<span>{cpuTemp}</span>{/if}
          {#if cpuTemp && ramUsage}<span class="sep">·</span>{/if}
          {#if ramUsage}<span>{ramUsage}</span>{/if}
        </a>
      {/if}

      <button
        type="button"
        class="ask-chip"
        onclick={() => assistantStore.openDrawer()}
        title="Open Sjel Assistant"
      >
        <Icon name="sparkles" size={12} />
        <span>Ask</span>
      </button>
    </div>
  </div>

  <!-- Connected Life Domains Mesh (Pillars Ribbon) -->
  <nav class="cockpit-ribbon" aria-label="Life pillars navigation">
    <a class="pillar-pill" href={link('/calendar')}>
      <Icon name="calendar" size={13} />
      <span class="pillar-label">Schedule</span>
      <span class="pillar-count mono">{todayEntries.length} today</span>
    </a>

    <a class="pillar-pill" href={link('/people')}>
      <Icon name="users" size={13} />
      <span class="pillar-label">People</span>
      <span class="pillar-count mono">{people.length || '–'}</span>
    </a>

    <a class="pillar-pill" href={link('/travel')}>
      <Icon name="train" size={13} />
      <span class="pillar-label">Travel</span>
      <span class="pillar-count mono">{plans.length} plans</span>
    </a>

    <a class="pillar-pill" href={link('/finance')}>
      <Icon name="wallet" size={13} />
      <span class="pillar-label">Finance</span>
      <span class="pillar-count mono">{burnMonthly ?? '–'}</span>
    </a>

    <a class="pillar-pill" href={link('/interior')}>
      <Icon name="layout" size={13} />
      <span class="pillar-label">Interior</span>
      <span class="pillar-count mono">{passingLayout?.pass ? 'Passes' : 'Plans'}</span>
    </a>

    <a class="pillar-pill" href={link('/feed')}>
      <Icon name="feed" size={13} />
      <span class="pillar-label">Feed</span>
      <span class="pillar-count mono">{feedItems.length} items</span>
    </a>

    <button
      type="button"
      class="pillar-pill search-pill"
      onclick={() => omniStore.open()}
      aria-label="Open Omni-Search"
    >
      <Icon name="search" size={13} />
      <span class="pillar-label">Search</span>
      <kbd class="pillar-kbd">⌘K</kbd>
    </button>
  </nav>
</aside>

<style>
  .axon-glance {
    position: relative;
    overflow: hidden;
    padding: var(--space-4) var(--space-5);
    margin-bottom: var(--space-4);
    background:
      radial-gradient(120% 90% at 100% 0%, var(--primary-soft) 0%, transparent 60%),
      var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--card-shadow);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .glance-top {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .glance-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--primary);
  }

  .date-context {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .live-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background-color: var(--primary);
    box-shadow: 0 0 6px var(--primary);
  }

  .glance-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .hw-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    background-color: var(--surface);
    padding: 0.15rem 0.5rem;
    border-radius: var(--radius-sm);
    text-decoration: none;
    border: 1px solid transparent;
    transition: border-color 0.15s ease, color 0.15s ease;
  }

  .hw-chip:hover {
    border-color: var(--card-border);
    color: var(--text-secondary);
  }

  .ask-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    color: var(--primary);
    background-color: var(--primary-soft);
    padding: 0.15rem 0.5rem;
    border-radius: var(--radius-sm);
    border: 1px solid transparent;
    cursor: pointer;
    transition: background-color 0.15s ease, color 0.15s ease;
  }

  .ask-chip:hover {
    background-color: var(--primary);
    color: var(--text-inverse);
  }

  .sep {
    opacity: 0.5;
  }

  .cockpit-ribbon {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    overflow-x: auto;
    padding-top: var(--space-2);
    border-top: 1px solid var(--card-border);
    scrollbar-width: none;
  }

  .cockpit-ribbon::-webkit-scrollbar {
    display: none;
  }

  .pillar-pill {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.25rem 0.55rem;
    border-radius: var(--radius-sm);
    background-color: var(--surface);
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    font-weight: 500;
    text-decoration: none;
    white-space: nowrap;
    border: 1px solid transparent;
    cursor: pointer;
    transition: background-color 0.12s ease, border-color 0.12s ease, color 0.12s ease;
  }

  .pillar-pill:hover {
    background-color: var(--card-bg);
    border-color: var(--card-border);
    color: var(--primary);
  }

  .pillar-count {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .search-pill {
    margin-left: auto;
    background-color: var(--primary-soft);
    color: var(--primary);
  }

  .pillar-kbd {
    font-size: 0.6rem;
    font-family: inherit;
    padding: 0.05rem 0.25rem;
    border-radius: var(--radius-sm);
    background-color: var(--card-bg);
    border: 1px solid var(--card-border);
    color: var(--primary);
  }
</style>
