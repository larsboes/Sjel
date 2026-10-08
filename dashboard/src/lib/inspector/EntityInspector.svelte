<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import { link } from "$lib/nav";
  import { modal } from "$lib/modal";
  import { inspectorStore, type InspectableItem } from "./inspector.svelte";
  import { assistantStore } from "$lib/assistant/assistant.svelte";
  import { page } from "$app/state";
  import { tip } from "$lib/tip";
  import { untrack } from "svelte";
  import { capabilities } from "$lib/capabilities.svelte";
  import ConnectionGroups from "./ConnectionGroups.svelte";
  import { connectionsFor, deepLink, expand, type ConnectionGroup } from "./connections";
  import { recordHref } from "./record";

  let groups = $state<ConnectionGroup[] | null>(null);

  // Read the connections each time the panel shows a different item. A slower answer for an
  // item already left behind is dropped, so the list never shows another item's links.
  $effect(() => {
    const current = inspectorStore.item;
    void capabilities.linksKey;
    groups = null;
    if (!current) return;
    let live = true;
    // The registry re-polls every 15 s; reading it tracked would refetch and blank the list.
    const registry = untrack(() => capabilities.items);
    void connectionsFor(current, registry).then((found) => {
      if (live) groups = found;
    });
    return () => {
      live = false;
    };
  });

  let touchStartY = 0;
  let touchDeltaY = $state(0);

  function handleTouchStart(e: TouchEvent) {
    if (e.touches.length === 1) {
      touchStartY = e.touches[0].clientY;
    }
  }

  function handleTouchMove(e: TouchEvent) {
    if (e.touches.length === 1) {
      const delta = e.touches[0].clientY - touchStartY;
      if (delta > 0) {
        touchDeltaY = delta;
      }
    }
  }

  function handleTouchEnd() {
    if (touchDeltaY > 80) {
      inspectorStore.close();
    }
    touchDeltaY = 0;
  }

  async function follow(next: InspectableItem) {
    inspectorStore.follow(await expand(next));
  }

  function askAssistantAbout(item: InspectableItem) {
    let prompt = "";
    if (item.type === "event") {
      prompt = `Tell me about the event "${item.title}" on ${item.startsAt.slice(0, 10)}`;
    } else if (item.type === "person") {
      prompt = `Show me recent context and notes for ${item.name}`;
    } else if (item.type === "trip") {
      prompt = `Summarize logistics, transit and weather for trip "${item.title}" to ${item.destination}`;
    } else if (item.type === "transaction") {
      prompt = `Explain the expense of ${item.amount} at ${item.merchant}`;
    } else if (item.type === "layout") {
      prompt = `What are the clearance verdicts and furniture options for room layout "${item.name}"?`;
    }
    inspectorStore.close();
    assistantStore.openDrawer();
    if (prompt) {
      void assistantStore.send(prompt, page.url.pathname);
    }
  }
</script>

{#if inspectorStore.isOpen && inspectorStore.item}
  {@const item = inspectorStore.item}
  <div class="inspector-scrim">
    <button
      class="backdrop"
      aria-label="Close inspector"
      onclick={() => inspectorStore.close()}
    ></button>

    <div
      class="inspector-panel"
      style={touchDeltaY > 0 ? `transform: translateY(${touchDeltaY}px); transition: none;` : undefined}
      use:modal={{ onClose: () => inspectorStore.close() }}
      role="dialog"
      aria-modal="true"
      aria-label="Inspector"
      tabindex="-1"
    >
      <!-- Mobile drag pill -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="mobile-drag-wrap"
        ontouchstart={handleTouchStart}
        ontouchmove={handleTouchMove}
        ontouchend={handleTouchEnd}
        aria-hidden="true"
      >
        <div class="mobile-drag-pill"></div>
      </div>

      <!-- Header -->
      <div class="panel-header">
        {#if inspectorStore.trail.length > 0}
          <button
            type="button"
            class="btn-close"
            onclick={() => inspectorStore.back()}
            aria-label="Back to the previous item"
            use:tip={"Back"}
          >
            <Icon name="arrow-left" size={14} />
          </button>
        {/if}
        <div class="header-type-pill">
          <Icon
            name={item.type === "event"
              ? "calendar"
              : item.type === "person"
                ? "users"
                : item.type === "trip"
                  ? "train"
                  : item.type === "transaction"
                    ? "wallet"
                    : item.type === "link"
                      ? "boxes"
                      : "layout"}
            size={13}
          />
          <span class="type-name">{(item.type === "link" ? item.kind : item.type).toUpperCase()}</span>
        </div>

        {@const asPage = recordHref(item)}
        {#if asPage}
          <a class="btn-close as-page" href={asPage} onclick={() => inspectorStore.close()} use:tip={"Open as page"}>
            <Icon name="external" size={14} />
          </a>
        {/if}
        <button
          type="button"
          class="btn-close"
          onclick={() => inspectorStore.close()}
          aria-label="Close inspector"
        >
          <Icon name="close" size={14} />
        </button>
      </div>

      <!-- Content Body -->
      <div class="panel-body">
        {#if item.type === "event"}
          <div class="entity-hero">
            <h2 class="entity-title">{item.title}</h2>
            <div class="entity-sub">
              <Icon name="clock" size={13} />
              <span>{item.startsAt.slice(0, 10)} {item.startsAt.slice(11, 16) || ""}</span>
            </div>
            {#if item.location}
              <div class="entity-sub">
                <Icon name="map-pin" size={13} />
                <span>{item.location}</span>
              </div>
            {/if}
          </div>

          <!-- Synapses / Connected Context -->
          <div class="connected-section">
            <span class="section-kicker">Connected Life</span>
            <div class="connected-chips">
              {#if item.attendees && item.attendees.length > 0}
                {#each item.attendees as person}
                  <button
                    type="button"
                    class="synapse-chip"
                    onclick={() => inspectorStore.inspectPerson({ id: person, name: person })}
                  >
                    <Icon name="users" size={12} />
                    <span>{person}</span>
                  </button>
                {/each}
              {/if}
              {#if item.tripId}
                <a class="synapse-chip" href={link(`/travel?plan=${encodeURIComponent(item.tripId)}`)}>
                  <Icon name="train" size={12} />
                  <span>Linked Trip</span>
                </a>
              {/if}
              {#if item.onEdit}
                <button
                  type="button"
                  class="synapse-chip"
                  onclick={() => {
                    const editFn = item.onEdit;
                    inspectorStore.close();
                    editFn?.();
                  }}
                >
                  <Icon name="pencil" size={12} />
                  <span>Edit in Full Form</span>
                </button>
              {/if}
              <a class="synapse-chip" href={deepLink(item)}>
                <Icon name="calendar" size={12} />
                <span>Open in Calendar</span>
              </a>
            </div>
          </div>

          {#if item.notes}
            <div class="detail-box">
              <span class="box-kicker">Notes</span>
              <p class="notes-text">{item.notes}</p>
            </div>
          {/if}

        {:else if item.type === "person"}
          <div class="entity-hero">
            <div class="avatar-ring">
              <Icon name="users" size={20} />
            </div>
            <h2 class="entity-title">{item.name}</h2>
            {#if item.location}
              <div class="entity-sub">
                <Icon name="map-pin" size={13} />
                <span>Currently in {item.location}</span>
              </div>
            {/if}
            {#if item.role || item.relationship}
              <span class="role-badge">{item.role ?? item.relationship}</span>
            {/if}
          </div>

          <div class="connected-section">
            <span class="section-kicker">Connected Life</span>
            <div class="connected-chips">
              <a class="synapse-chip" href={link(`/people?id=${item.id}`)}>
                <Icon name="users" size={12} />
                <span>View Full Profile</span>
              </a>
              <a class="synapse-chip" href={link("/calendar")}>
                <Icon name="calendar" size={12} />
                <span>Check Availability</span>
              </a>
            </div>
          </div>

          {#if item.events && item.events.length > 0}
            <div class="detail-box">
              <span class="box-kicker">Upcoming Shared Rhythm</span>
              <ul class="linked-list">
                {#each item.events as ev}
                  <li>
                    <span class="linked-date mono">{ev.date}</span>
                    <span class="linked-title">{ev.title}</span>
                  </li>
                {/each}
              </ul>
            </div>
          {/if}

        {:else if item.type === "trip"}
          <div class="entity-hero">
            <h2 class="entity-title">{item.title}</h2>
            <div class="entity-sub">
              <Icon name="map-pin" size={13} />
              <span>{item.destination}</span>
              <span class="sep">·</span>
              <span>{item.dates}</span>
            </div>
            {#if item.weather}
              <div class="entity-sub">
                <Icon name="sun" size={13} />
                <span>Forecast: {item.weather}</span>
              </div>
            {/if}
          </div>

          <div class="connected-section">
            <span class="section-kicker">Connected Life</span>
            <div class="connected-chips">
              {#if item.companions && item.companions.length > 0}
                {#each item.companions as companion}
                  <button
                    type="button"
                    class="synapse-chip"
                    onclick={() => inspectorStore.inspectPerson({ id: companion, name: companion })}
                  >
                    <Icon name="users" size={12} />
                    <span>{companion}</span>
                  </button>
                {/each}
              {/if}
              {#if item.budget}
                <span class="synapse-chip static">
                  <Icon name="wallet" size={12} />
                  <span>Budget: {item.budget}</span>
                </span>
              {/if}
              <a class="synapse-chip" href={deepLink(item)}>
                <Icon name="train" size={12} />
                <span>Open Itinerary</span>
              </a>
            </div>
          </div>

          {#if item.stages && item.stages.length > 0}
            <div class="detail-box">
              <span class="box-kicker">Itinerary Segments</span>
              <div class="stages-timeline">
                {#each item.stages as stage}
                  <div class="stage-entry">
                    <span class="stage-time mono">{stage.time}</span>
                    <div class="stage-main">
                      <strong class="stage-title">{stage.title}</strong>
                      {#if stage.detail}<span class="stage-detail">{stage.detail}</span>{/if}
                    </div>
                  </div>
                {/each}
              </div>
            </div>
          {/if}

        {:else if item.type === "transaction"}
          <div class="entity-hero">
            <h2 class="entity-title">{item.merchant}</h2>
            <div class="amount-hero mono">{item.amount}</div>
            <div class="entity-sub">
              <Icon name="calendar" size={13} />
              <span>{item.date}</span>
              <span class="sep">·</span>
              <span class="category-badge">{item.category}</span>
            </div>
          </div>

          <div class="connected-section">
            <span class="section-kicker">Connected Life</span>
            <div class="connected-chips">
              <a class="synapse-chip" href={deepLink(item)}>
                <Icon name="wallet" size={12} />
                <span>Open in Ledger</span>
              </a>
            </div>
          </div>

        {:else if item.type === "link"}
          <div class="entity-hero">
            <h2 class="entity-title">{item.title}</h2>
            {#if item.at || item.meta}
              <div class="entity-sub mono">
                {#if item.at}<span>{item.at.slice(0, 10)}</span>{/if}
                {#if item.at && item.meta}<span class="sep">·</span>{/if}
                {#if item.meta}<span>{item.meta}</span>{/if}
              </div>
            {/if}
            <span class="role-badge">via <span class="mono">{item.via}</span></span>
          </div>
          {#if deepLink(item)}
            <div class="connected-chips">
              <a class="synapse-chip" href={deepLink(item)}>
                <Icon name="external" size={12} />
                <span>Open</span>
              </a>
            </div>
          {/if}

        {:else if item.type === "layout"}
          <div class="entity-hero">
            <h2 class="entity-title">{item.name}</h2>
            <div class="entity-sub">
              <span class="pass-pill" class:pass={item.pass}>
                {item.pass ? "Passes clearances" : "Needs attention"}
              </span>
              {#if item.itemsCount != null}
                <span class="sep">·</span>
                <span>{item.itemsCount} furniture items</span>
              {/if}
            </div>
          </div>

          <div class="connected-section">
            <span class="section-kicker">Connected Life</span>
            <div class="connected-chips">
              {#if item.totalCost}
                <a class="synapse-chip" href={link("/finance")}>
                  <Icon name="wallet" size={12} />
                  <span>Cost: {item.totalCost}</span>
                </a>
              {/if}
              <a class="synapse-chip" href={link("/interior")}>
                <Icon name="layout" size={12} />
                <span>Open 3D Plan</span>
              </a>
            </div>
          </div>
        {/if}

        <ConnectionGroups {groups} onfollow={(next) => void follow(next)} />

        <!-- Persistent Assistant Quick Action -->
        <div class="assistant-bar">
          <button
            type="button"
            class="assistant-trigger-btn"
            onclick={() => askAssistantAbout(item)}
          >
            <Icon name="sparkles" size={14} />
            <span>Ask Assistant about this</span>
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .inspector-scrim {
    position: fixed;
    inset: 0;
    z-index: 90;
    display: flex;
    justify-content: flex-end;
    align-items: stretch;
    animation: fade-in var(--motion-base) var(--ease-out);
  }

  @keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .backdrop {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    border: none;
    background: rgba(0, 0, 0, 0.36);
    cursor: default;
    -webkit-tap-highlight-color: transparent;
  }

  .inspector-panel {
    position: relative;
    width: min(28rem, 100vw);
    height: 100%;
    max-height: 100dvh;
    background: var(--card-bg);
    border-left: 1px solid var(--card-border);
    box-shadow: -8px 0 32px rgba(0, 0, 0, 0.22);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: slide-in var(--motion-slow) var(--ease-out);
  }

  @keyframes slide-in {
    from { transform: translateX(100%); }
    to { transform: translateX(0); }
  }

  .mobile-drag-wrap {
    display: none;
    width: 100%;
    padding: 0.6rem 0 0.3rem;
    justify-content: center;
    align-items: center;
    cursor: grab;
  }

  .mobile-drag-pill {
    width: 38px;
    height: 4px;
    border-radius: var(--radius-full);
    background: var(--card-border);
  }

  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-4) var(--space-5);
    border-bottom: 1px solid var(--card-border);
  }

  .header-type-pill {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0.2rem 0.6rem;
    border-radius: var(--radius-full);
    background: var(--primary-soft);
    color: var(--primary);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
  }

  .btn-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    border-radius: var(--radius-full);
    border: none;
    background: transparent;
    color: var(--text-tertiary);
    cursor: pointer;
    transition: background var(--motion-fast) ease, color var(--motion-fast) ease;
  }

  .btn-close:hover {
    background: var(--surface);
    color: var(--text-primary);
  }

  /* Pushes itself and the close button to the right; space-between alone would centre it. */
  .as-page {
    margin-left: auto;
  }

  .panel-body {
    flex: 1;
    overflow-y: auto;
    -webkit-overflow-scrolling: touch;
    padding: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  .entity-hero {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .entity-title {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 600;
    letter-spacing: -0.015em;
    color: var(--text-primary);
    line-height: var(--leading-tight);
  }

  .entity-sub {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .amount-hero {
    font-size: var(--text-2xl);
    font-weight: 700;
    color: var(--text-primary);
    margin: var(--space-1) 0;
  }

  .avatar-ring {
    width: 3.5rem;
    height: 3.5rem;
    border-radius: var(--radius-full);
    background: var(--primary-soft);
    color: var(--primary);
    display: grid;
    place-items: center;
    margin-bottom: var(--space-2);
  }

  .role-badge, .category-badge {
    align-self: flex-start;
    padding: 0.15rem 0.5rem;
    border-radius: var(--radius-sm);
    font-size: var(--text-2xs);
    font-weight: 500;
    background: var(--surface);
    color: var(--text-secondary);
    border: 1px solid var(--card-border);
  }

  .pass-pill {
    font-size: var(--text-2xs);
    font-weight: 600;
    padding: 0.15rem 0.5rem;
    border-radius: var(--radius-sm);
    background: var(--warning-soft);
    color: var(--warning-ink);
  }

  .pass-pill.pass {
    background: var(--success-soft);
    color: var(--success);
  }

  .connected-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    background: var(--surface);
    border-radius: var(--radius-md);
    border: 1px solid var(--card-border);
  }

  .section-kicker, .box-kicker {
    font-size: var(--text-2xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-tertiary);
  }

  .connected-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .synapse-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0.35rem 0.65rem;
    border-radius: var(--radius-full);
    border: 1px solid var(--card-border);
    background: var(--card-bg);
    color: var(--text-primary);
    font-size: var(--text-xs);
    font-weight: 500;
    text-decoration: none;
    cursor: pointer;
    transition: border-color var(--motion-fast) ease, color var(--motion-fast) ease, transform var(--motion-fast) ease;
  }

  .synapse-chip:hover {
    border-color: var(--primary);
    color: var(--primary);
    transform: translateY(-1px);
  }

  .detail-box {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4);
    background: var(--card-bg);
    border-radius: var(--radius-md);
    border: 1px solid var(--card-border);
  }

  .notes-text {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-secondary);
    line-height: var(--leading-normal);
  }

  .linked-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .linked-list li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    font-size: var(--text-xs);
  }

  .linked-date {
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .box-kicker {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  .synapse-chip.static {
    cursor: default;
  }

  .synapse-chip.static:hover {
    border-color: var(--card-border);
    color: var(--text-primary);
    transform: none;
  }

  .linked-title {
    color: var(--text-primary);
    font-weight: 500;
  }

  .stages-timeline {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .stage-entry {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    font-size: var(--text-xs);
  }

  .stage-time {
    color: var(--primary);
    font-weight: 600;
  }

  .stage-main {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
  }

  .stage-title {
    color: var(--text-primary);
  }

  .stage-detail {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .assistant-bar {
    margin-top: auto;
    padding-top: var(--space-4);
  }

  .assistant-trigger-btn {
    width: 100%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: 0.75rem var(--space-4);
    border-radius: var(--radius-md);
    border: 1px solid var(--primary-soft);
    background: var(--primary-soft);
    color: var(--primary);
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
    transition: background-color var(--motion-fast) var(--ease-out), color var(--motion-fast) var(--ease-out), transform var(--motion-fast) var(--ease-out);
  }

  .assistant-trigger-btn:hover {
    background: var(--primary);
    color: var(--text-inverse);
  }

  .assistant-trigger-btn:active {
    transform: scale(0.98);
  }

  @media (max-width: 560px) {
    .inspector-scrim {
      align-items: flex-end;
    }

    .mobile-drag-wrap {
      display: flex;
    }

    .inspector-panel {
      width: 100%;
      height: auto;
      max-height: 88dvh;
      border-radius: var(--radius-lg) var(--radius-lg) 0 0;
      border-left: none;
      border-top: 1px solid var(--card-border);
      animation: sheet-up var(--motion-slow) var(--ease-out);
    }

    @keyframes sheet-up {
      from { transform: translateY(100%); }
      to { transform: translateY(0); }
    }
  }
</style>
