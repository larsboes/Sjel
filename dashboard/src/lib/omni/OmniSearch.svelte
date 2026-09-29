<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import Icon from "$lib/Icon.svelte";
  import { modal } from "$lib/modal";
  import { link } from "$lib/nav";
  import { omniStore } from "./omni.svelte";
  import { assistantStore } from "$lib/assistant/assistant.svelte";
  import {
    entities,
    calendar,
    trips,
    finance,
    comms,
    type Entity,
    type CalendarEntry,
    type TripPlan,
    type Subscription,
    type FeedEntry,
  } from "$lib/api";

  import { focusRow } from "$lib/list-cursor.svelte";

  interface SearchResult {
    id: string;
    domain: "action" | "people" | "calendar" | "travel" | "finance" | "feed";
    domainLabel: string;
    icon: string;
    title: string;
    subtitle: string;
    badge?: string;
    href?: string;
    action?: () => void;
  }

  let query = $state("");
  let selectedIndex = $state(0);
  let inputEl: HTMLInputElement | undefined = $state();
  let searchListEl: HTMLElement | undefined = $state();

  let peopleList = $state<Entity[]>([]);
  let calendarList = $state<CalendarEntry[]>([]);
  let tripsList = $state<TripPlan[]>([]);
  let subsList = $state<Subscription[]>([]);
  let feedList = $state<FeedEntry[]>([]);
  let dataLoaded = $state(false);

  async function loadData() {
    if (dataLoaded) return;
    const now = new Date();
    const fromStr = now.toISOString().slice(0, 10);
    const future = new Date(now.getFullYear(), now.getMonth() + 2, now.getDate());
    const toStr = future.toISOString().slice(0, 10);

    const [peopleRes, calRes, tripRes, subsRes, feedRes] = await Promise.allSettled([
      entities.list("person"),
      calendar.entries.list(fromStr, toStr),
      trips.list(),
      finance.subscriptions(),
      comms.feed({ days: 14 }),
    ]);

    if (peopleRes.status === "fulfilled") peopleList = peopleRes.value;
    if (calRes.status === "fulfilled") calendarList = calRes.value;
    if (tripRes.status === "fulfilled") tripsList = tripRes.value;
    if (subsRes.status === "fulfilled") subsList = subsRes.value;
    if (feedRes.status === "fulfilled") feedList = feedRes.value;
    dataLoaded = true;
  }

  $effect(() => {
    if (omniStore.isOpen) {
      void loadData();
      query = "";
      selectedIndex = 0;
      setTimeout(() => inputEl?.focus(), 20);
    }
  });

  const q = $derived(query.trim().toLowerCase());

  const results = $derived.by<SearchResult[]>(() => {
    const list: SearchResult[] = [];

    // 1. Actions
    if (q) {
      list.push({
        id: "act-ask",
        domain: "action",
        domainLabel: "AI Assistant",
        icon: "sparkles",
        title: `Ask Sjel: "${query.trim()}"`,
        subtitle: "Query the on-device assistant across live capabilities",
        action: () => {
          omniStore.close();
          assistantStore.openDrawer();
          void assistantStore.send(query.trim(), page.url.pathname);
        },
      });
    }

    list.push({
      id: "act-cal",
      domain: "action",
      domainLabel: "Schedule",
      icon: "calendar",
      title: "Schedule new event",
      subtitle: "Open calendar to schedule a commitment or rhythm",
      href: link("/calendar"),
    });

    list.push({
      id: "act-person",
      domain: "action",
      domainLabel: "People",
      icon: "users",
      title: "Add new person",
      subtitle: "Register a contact, companion or host in entities",
      href: link("/people"),
    });

    list.push({
      id: "act-train",
      domain: "action",
      domainLabel: "Travel",
      icon: "train",
      title: "Check train connections",
      subtitle: "Search live Deutsche Bahn routes, Sparpreis and delays",
      href: link("/travel/connections"),
    });

    // 2. People
    for (const p of peopleList) {
      if (!q || p.name.toLowerCase().includes(q) || (p.note_ref && p.note_ref.toLowerCase().includes(q))) {
        const homeFact = p.facts?.find((f) => f.predicate === "home_base");
        const awayFact = p.facts?.find((f) => f.predicate === "away");
        const loc = awayFact ? `Away in ${awayFact.place}` : homeFact ? homeFact.place : "Contact";
        list.push({
          id: `person-${p.id}`,
          domain: "people",
          domainLabel: "People",
          icon: "users",
          title: p.name,
          subtitle: loc,
          badge: homeFact ? "Home" : undefined,
          href: link(`/people?id=${encodeURIComponent(p.id)}`),
        });
      }
    }

    // 3. Calendar
    for (const c of calendarList) {
      if (
        !q ||
        c.title.toLowerCase().includes(q) ||
        (c.location && c.location.toLowerCase().includes(q)) ||
        (c.notes && c.notes.toLowerCase().includes(q))
      ) {
        list.push({
          id: `cal-${c.id}`,
          domain: "calendar",
          domainLabel: "Schedule",
          icon: "calendar",
          title: c.title,
          subtitle: `${c.starts_at.slice(0, 10)}${c.location ? ` · ${c.location}` : ""}`,
          badge: c.commitment,
          href: link("/calendar"),
        });
      }
    }

    // 4. Trips
    for (const t of tripsList) {
      const destNames = t.destinations?.map((d) => d.name).join(", ") || "";
      if (!q || t.title.toLowerCase().includes(q) || destNames.toLowerCase().includes(q)) {
        list.push({
          id: `trip-${t.id}`,
          domain: "travel",
          domainLabel: "Travel",
          icon: "train",
          title: t.title,
          subtitle: `${t.date_start} – ${t.date_end}${destNames ? ` · ${destNames}` : ""}`,
          href: link("/travel"),
        });
      }
    }

    // 5. Finance
    for (const s of subsList) {
      if (!q || s.name.toLowerCase().includes(q)) {
        const latestPrice = s.prices?.[s.prices.length - 1];
        list.push({
          id: `sub-${s.id}`,
          domain: "finance",
          domainLabel: "Finance",
          icon: "wallet",
          title: s.name,
          subtitle: latestPrice
            ? `Subscription · ${(latestPrice.amount_cents / 100).toFixed(2)} ${latestPrice.currency}`
            : "Active subscription",
          href: link("/finance"),
        });
      }
    }

    // 6. Feed
    for (const f of feedList) {
      const feedTitle = f.title || f.url;
      if (!q || feedTitle.toLowerCase().includes(q) || f.url.toLowerCase().includes(q)) {
        list.push({
          id: `feed-${f.id}`,
          domain: "feed",
          domainLabel: "Feed",
          icon: "feed",
          title: feedTitle,
          subtitle: `${f.url} · ${f.stream}`,
          href: link(`/feed/${f.id}`),
        });
      }
    }

    return list.slice(0, 40);
  });

  function selectItem(item: SearchResult) {
    omniStore.close();
    if (item.action) {
      item.action();
    } else if (item.href) {
      void goto(item.href);
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      omniStore.close();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      selectedIndex = (selectedIndex + 1) % Math.max(1, results.length);
      scrollToSelected();
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      selectedIndex = (selectedIndex - 1 + results.length) % Math.max(1, results.length);
      scrollToSelected();
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      const current = results[selectedIndex];
      if (current) selectItem(current);
    }
  }

  function scrollToSelected() {
    focusRow(document.getElementById(`omni-item-${selectedIndex}`));
  }
</script>

{#if omniStore.isOpen}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="omni-scrim" onclick={() => omniStore.close()}>
    <div
      class="omni-dialog"
      role="dialog"
      aria-modal="true"
      tabindex="-1"
      use:modal={{ onClose: () => omniStore.close() }}
      onclick={(e) => e.stopPropagation()}
    >
      <div class="omni-header">
        <Icon name="search" size={18} />
        <input
          bind:this={inputEl}
          bind:value={query}
          onkeydown={handleKeydown}
          placeholder="Search people, schedule, travel, finances, feed, actions..."
          aria-label="Search across Sjel"
          autocomplete="off"
          spellcheck="false"
        />
        <kbd class="esc-kbd">ESC</kbd>
      </div>

      <div class="omni-results" bind:this={searchListEl}>
        {#if results.length === 0}
          <div class="omni-empty">
            <p>No matching items found for "{query}".</p>
            <button
              type="button"
              class="btn btn-outline"
              onclick={() => {
                omniStore.close();
                assistantStore.openDrawer();
                void assistantStore.send(query.trim(), page.url.pathname);
              }}
            >
              <Icon name="sparkles" size={14} /> Ask Sjel Assistant instead
            </button>
          </div>
        {:else}
          {#each results as item, index (item.id)}
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
            <div
              id={`omni-item-${index}`}
              tabindex="-1"
              class="omni-item"
              class:selected={index === selectedIndex}
              data-index={index}
              onclick={() => selectItem(item)}
              onmouseenter={() => (selectedIndex = index)}
            >
              <div class="item-icon" class:action-icon={item.domain === "action"}>
                <Icon name={item.icon as never} size={15} />
              </div>
              <div class="item-body">
                <div class="item-title-row">
                  <span class="item-title">{item.title}</span>
                  {#if item.badge}
                    <span class="item-badge">{item.badge}</span>
                  {/if}
                </div>
                <span class="item-subtitle">{item.subtitle}</span>
              </div>
              <span class="item-domain-tag">{item.domainLabel}</span>
            </div>
          {/each}
        {/if}
      </div>

      <div class="omni-footer">
        <div class="shortcut-hints">
          <span><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
          <span><kbd>↵</kbd> select</span>
          <span><kbd>esc</kbd> close</span>
        </div>
        <div class="omni-brand">
          <Icon name="sparkles" size={12} />
          <span>Sjel Omni-Search</span>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .omni-scrim {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: clamp(2rem, 10vh, 6rem);
    padding-inline: var(--space-4);
    background-color: rgb(0 0 0 / 55%);
    -webkit-backdrop-filter: blur(4px);
    backdrop-filter: blur(4px);
    overscroll-behavior: contain;
  }

  .omni-dialog {
    width: 100%;
    max-width: 44rem;
    max-height: 80vh;
    max-height: 80dvh;
    display: flex;
    flex-direction: column;
    background-color: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--card-shadow-hover);
    overflow: hidden;
    outline: none;
    animation: omni-fade-in 0.15s ease-out;
  }

  @keyframes omni-fade-in {
    from {
      opacity: 0;
      transform: translateY(-8px) scale(0.98);
    }
    to {
      opacity: 1;
      transform: translateY(0) scale(1);
    }
  }

  .omni-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5);
    border-bottom: 1px solid var(--card-border);
    background-color: var(--card-bg);
    color: var(--primary);
  }

  .omni-header input {
    flex: 1;
    border: none;
    background: transparent;
    font-size: var(--text-base);
    color: var(--text-primary);
    outline: none;
  }

  .omni-header input::placeholder {
    color: var(--text-tertiary);
    font-size: var(--text-sm);
  }

  .esc-kbd {
    font-size: var(--text-2xs);
    font-family: inherit;
    padding: 0.15rem 0.4rem;
    border-radius: var(--radius-sm);
    background-color: var(--surface);
    color: var(--text-tertiary);
    border: 1px solid var(--card-border);
  }

  .omni-results {
    flex: 1;
    overflow-y: auto;
    -webkit-overflow-scrolling: touch;
    overscroll-behavior: contain;
    padding: var(--space-2);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .omni-empty {
    padding: var(--space-7) var(--space-4);
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .omni-item {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-md);
    cursor: pointer;
    text-decoration: none;
    color: inherit;
    border: 1px solid transparent;
    transition: background-color 0.1s ease;
  }

  .omni-item:hover,
  .omni-item.selected {
    background-color: var(--surface);
    border-color: var(--card-border);
  }

  .item-icon {
    display: grid;
    place-items: center;
    width: 2rem;
    height: 2rem;
    border-radius: var(--radius-md);
    background-color: var(--surface);
    color: var(--primary);
    flex-shrink: 0;
  }

  .action-icon {
    background-color: var(--primary-soft);
    color: var(--primary);
  }

  .item-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  .item-title-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .item-title {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .item-badge {
    font-size: var(--text-2xs);
    padding: 0.1rem 0.35rem;
    border-radius: var(--radius-sm);
    background-color: var(--primary-soft);
    color: var(--primary);
    font-weight: 600;
  }

  .item-subtitle {
    font-size: var(--text-xs);
    color: var(--text-tertiary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .item-domain-tag {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    background-color: var(--card-bg);
    padding: 0.15rem 0.45rem;
    border-radius: var(--radius-sm);
    border: 1px solid var(--card-border);
    flex-shrink: 0;
  }

  .omni-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-4);
    background-color: var(--surface);
    border-top: 1px solid var(--card-border);
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .shortcut-hints {
    display: flex;
    gap: var(--space-3);
  }

  .shortcut-hints kbd {
    font-family: inherit;
    padding: 0.1rem 0.25rem;
    background-color: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    margin-right: 0.15rem;
  }

  .omni-brand {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--primary);
  }

  @media (max-width: 48rem) {
    .omni-scrim {
      padding-top: max(1rem, env(safe-area-inset-top, 1rem));
      padding-inline: 0.75rem;
      padding-bottom: max(1rem, env(safe-area-inset-bottom, 1rem));
    }

    .omni-dialog {
      max-height: 85dvh;
    }
  }
</style>
