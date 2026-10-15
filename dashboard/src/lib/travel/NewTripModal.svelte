<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import Overlay from "$lib/Overlay.svelte";
  import PlaceField from "$lib/travel/PlaceField.svelte";
  import PlanSearchPanel from "$lib/travel/PlanSearchPanel.svelte";
  import BaseFinderPanel from "$lib/travel/BaseFinderPanel.svelte";
  import type { BaseCandidate, PlaceRef, TransportMode } from "$lib/api";
  import type { RankedCandidate } from "$lib/travel/api";

  interface Props {
    open: boolean;
    origin: PlaceRef | null;
    firstDestination: PlaceRef | null;
    secondDestination: PlaceRef | null;
    showSecondDestination: boolean;
    startDate: string;
    endDate: string;
    interests: string;
    travelers: string;
    transportModes: TransportMode[];
    intentSentence: string;
    intentFeedback: { assumptions: string[]; unresolved: string[] } | null;
    draftingIntent: boolean;
    creating: boolean;
    plannerNotice: string | null;
    modeOptions: Array<{ id: TransportMode; label: string }>;
    onClose: () => void;
    onCreatePlan: () => Promise<void>;
    onDraftIntent: () => Promise<void>;
    onToggleTransportMode: (mode: TransportMode) => void;
    onAdoptSearchCandidate: (job: number, candidate: RankedCandidate) => Promise<string>;
    onAdoptBaseCandidate: (
      base: BaseCandidate,
      from: PlaceRef,
      anchor: PlaceRef,
      fromDate: string,
      anchorDate: string,
    ) => Promise<void>;
  }

  let {
    open,
    origin = $bindable(),
    firstDestination = $bindable(),
    secondDestination = $bindable(),
    showSecondDestination = $bindable(),
    startDate = $bindable(),
    endDate = $bindable(),
    interests = $bindable(),
    travelers = $bindable(),
    transportModes,
    intentSentence = $bindable(),
    intentFeedback,
    draftingIntent,
    creating,
    plannerNotice,
    modeOptions,
    onClose,
    onCreatePlan,
    onDraftIntent,
    onToggleTransportMode,
    onAdoptSearchCandidate,
    onAdoptBaseCandidate,
  }: Props = $props();

  let plannerTab = $state<"find" | "base" | "new">("new");
</script>

{#if open}
  <Overlay
    title="New trip"
    eyebrow="Trip plans"
    width="44rem"
    busy={creating}
    {onClose}
  >
    <p class="planner-lede">Start with the route, dates, and intent.</p>

    {#if plannerNotice}
      <p class="planner-notice" aria-live="polite">{plannerNotice}</p>
    {/if}

    <div class="planner-tabs" role="tablist" aria-label="How to start a trip">
      <button
        type="button"
        role="tab"
        aria-selected={plannerTab === "find"}
        class:active={plannerTab === "find"}
        onclick={() => (plannerTab = "find")}>Find a trip</button
      >
      <button
        type="button"
        role="tab"
        aria-selected={plannerTab === "base"}
        class:active={plannerTab === "base"}
        onclick={() => (plannerTab = "base")}>Find a base</button
      >
      <button
        type="button"
        role="tab"
        aria-selected={plannerTab === "new"}
        class:active={plannerTab === "new"}
        onclick={() => (plannerTab = "new")}>New trip</button
      >
    </div>

    {#if plannerTab === "find"}
      <PlanSearchPanel
        {origin}
        {modeOptions}
        onChoose={(seed) => {
          firstDestination = { ...seed.destination, kind: "city" };
          startDate = seed.startDate;
          endDate = seed.endDate;
          plannerTab = "new";
        }}
        onAdopt={onAdoptSearchCandidate}
      />
    {:else if plannerTab === "base"}
      <BaseFinderPanel
        initialFrom={origin}
        initialAnchor={firstDestination}
        onAdopt={onAdoptBaseCandidate}
      />
    {:else}
      <form
        class="planner-form"
        onsubmit={(event) => {
          event.preventDefault();
          void onCreatePlan();
        }}
      >
        <div class="intent-intake-box">
          <div class="intent-header">
            <span class="intent-title"><Icon name="compass" size={13} /> Draft with Natural Language</span>
            <span class="intent-hint">Describe your idea in plain prose</span>
          </div>
          <div class="intent-input-row">
            <input
              class="input intent-sentence-input"
              type="text"
              placeholder="e.g. Somewhere warm in October under 300 euro by train..."
              bind:value={intentSentence}
              aria-label="Trip idea in plain prose"
              onkeydown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void onDraftIntent();
                }
              }}
            />
            <button
              class="btn btn-secondary intent-draft-btn"
              type="button"
              disabled={draftingIntent || !intentSentence.trim()}
              onclick={onDraftIntent}
            >
              {#if draftingIntent}
                <Icon name="loader" size={13} /> Drafting…
              {:else}
                <Icon name="compass" size={13} /> Draft
              {/if}
            </button>
          </div>
          {#if intentFeedback}
            <div class="intent-feedback">
              {#if intentFeedback.assumptions.length > 0}
                <p class="intent-assumption">
                  <Icon name="check" size={11} /> {intentFeedback.assumptions.join(" · ")}
                </p>
              {/if}
              {#if intentFeedback.unresolved.length > 0}
                <p class="intent-unresolved">
                  <Icon name="alert" size={11} /> Confirm: {intentFeedback.unresolved.join(", ")}
                </p>
              {/if}
            </div>
          {/if}
        </div>

        <div class="route-fields">
          <PlaceField label="Origin" placeholder="Address, city, or station" bind:place={origin} />
          <PlaceField
            label="Destination"
            placeholder="Address, city, venue, or station"
            bind:place={firstDestination}
          />
          {#if showSecondDestination}
            <PlaceField
              label="Another stop"
              placeholder="Another place or stopover"
              bind:place={secondDestination}
            />
          {:else}
            <button
              class="add-stop"
              type="button"
              onclick={() => (showSecondDestination = true)}
            >
              <Icon name="plus" size={14} /> Add stop
            </button>
          {/if}
        </div>

        <div class="intent-fields">
          <label>
            <span>From</span>
            <input class="input" type="date" aria-label="Start date" bind:value={startDate} />
          </label>
          <label>
            <span>To</span>
            <input class="input" type="date" aria-label="End date" min={startDate} bind:value={endDate} />
          </label>
          <label class="interest-field">
            <span>What should happen?</span>
            <input
              class="input"
              bind:value={interests}
              aria-label="Trip interests"
              placeholder="Live music, architecture, Rust, design…"
            />
          </label>
          <label class="travelers-field">
            <span>Who is travelling?</span>
            <input
              class="input"
              bind:value={travelers}
              aria-label="Who is travelling"
              placeholder="Separate names with commas"
            />
          </label>
          <fieldset class="mode-field">
            <legend>Possible transport modes</legend>
            <div class="mode-chips">
              {#each modeOptions as option (option.id)}
                <button
                  type="button"
                  class="mode-btn"
                  class:active={transportModes.includes(option.id)}
                  aria-pressed={transportModes.includes(option.id)}
                  onclick={() => onToggleTransportMode(option.id)}
                >
                  {option.label}
                </button>
              {/each}
            </div>
          </fieldset>
          <button class="btn btn-primary plan-button" type="submit" disabled={creating}>
            {#if creating}
              <Icon name="loader" size={14} /> Planning…
            {:else}
              <Icon name="map-pin" size={14} /> Create trip plan
            {/if}
          </button>
        </div>
      </form>

      <div class="planner-context" aria-label="Planning scope">
        <span><b>01</b> Places, legs, and travellers</span>
        <span><b>02</b> Transport options for each leg</span>
        <span><b>03</b> Activities, events, and itinerary</span>
      </div>
    {/if}
  </Overlay>
{/if}

<style>
  .planner-lede {
    font-size: var(--text-sm);
    color: var(--text-secondary);
    margin: 0 0 var(--space-3);
  }

  .planner-notice {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: var(--text-xs);
    margin-bottom: var(--space-3);
  }

  .planner-tabs {
    display: flex;
    gap: var(--space-2);
    border-bottom: 1px solid var(--card-border);
    margin-bottom: var(--space-4);
  }

  .planner-tabs button {
    background: none;
    border: none;
    padding: var(--space-2) var(--space-3);
    font-size: var(--text-sm);
    color: var(--text-secondary);
    cursor: pointer;
    border-bottom: 2px solid transparent;
  }

  .planner-tabs button.active {
    color: var(--text-primary);
    border-bottom-color: var(--accent);
    font-weight: 500;
  }

  .planner-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .intent-intake-box {
    background: var(--surface);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .intent-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
  }

  .intent-title {
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--text-primary);
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
  }

  .intent-hint {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .intent-input-row {
    display: flex;
    gap: var(--space-2);
  }

  .intent-sentence-input {
    flex: 1;
  }

  .intent-feedback {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: var(--text-xs);
    padding-top: var(--space-2);
    border-top: 1px solid var(--card-border);
  }

  .intent-assumption {
    color: var(--success);
    margin: 0;
    display: flex;
    align-items: center;
    gap: 0.35rem;
  }

  .intent-unresolved {
    color: var(--warning-ink);
    margin: 0;
    display: flex;
    align-items: center;
    gap: 0.35rem;
  }

  .route-fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .add-stop {
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    background: none;
    border: 1px dashed var(--card-border);
    border-radius: var(--radius-sm);
    padding: 0.3rem 0.6rem;
    font-size: var(--text-xs);
    color: var(--text-secondary);
    cursor: pointer;
  }

  .add-stop:hover {
    color: var(--accent);
    border-color: var(--accent);
  }

  .intent-fields {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }

  .intent-fields label {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .interest-field,
  .travelers-field,
  .mode-field {
    grid-column: span 2;
  }

  .mode-field {
    border: none;
    margin: 0;
    padding: 0;
  }

  .mode-field legend {
    font-size: var(--text-xs);
    color: var(--text-secondary);
    margin-bottom: var(--space-2);
  }

  .mode-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .mode-btn {
    padding: 0.25rem 0.6rem;
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    background: var(--surface);
    border: 1px solid var(--card-border);
    color: var(--text-secondary);
    cursor: pointer;
  }

  .mode-btn.active {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 500;
  }

  .plan-button {
    grid-column: span 2;
    margin-top: var(--space-2);
  }

  .planner-context {
    display: flex;
    justify-content: space-between;
    margin-top: var(--space-4);
    padding-top: var(--space-3);
    border-top: 1px solid var(--card-border);
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .planner-context span b {
    color: var(--accent);
  }

  .input {
    padding: 0.4rem 0.6rem;
    border-radius: var(--radius-sm);
    border: 1px solid var(--card-border);
    background: var(--surface);
    color: var(--text-primary);
    font-size: var(--text-sm);
  }

  .input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.4rem;
    padding: 0.45rem 0.9rem;
    border-radius: var(--radius-sm);
    font-size: var(--text-sm);
    font-weight: 500;
    cursor: pointer;
    border: none;
  }

  .btn-primary {
    background: var(--accent);
    color: white;
  }

  .btn-primary:hover:not(:disabled) {
    opacity: 0.9;
  }

  .btn-secondary {
    background: var(--surface);
    border: 1px solid var(--card-border);
    color: var(--text-primary);
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--nav-hover);
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
