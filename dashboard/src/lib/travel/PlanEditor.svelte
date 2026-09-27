<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import PlaceField from "$lib/travel/PlaceField.svelte";
  import type { PlaceRef, TransportMode, TripPlan } from "$lib/api";

  let {
    plan,
    saving = false,
    deleting = false,
    onSave,
    onCancel,
    onDelete,
  }: {
    plan: TripPlan;
    saving?: boolean;
    deleting?: boolean;
    onSave: (patch: Partial<TripPlan>) => void | Promise<void>;
    onCancel: () => void;
    onDelete: () => void | Promise<void>;
  } = $props();

  const MODE_OPTIONS: Array<{ id: TransportMode; label: string }> = [
    { id: "train", label: "Train" },
    { id: "flight", label: "Flight" },
    { id: "car", label: "Car" },
    { id: "bus", label: "Bus" },
    { id: "ferry", label: "Ferry" },
    { id: "bike", label: "Bike" },
    { id: "walk", label: "Walk" },
  ];

  let initializedPlanId = $state("");
  let title = $state("");
  let origin = $state<PlaceRef | null>(null);
  let destinations = $state<Array<PlaceRef | null>>([]);
  let dateStart = $state("");
  let dateEnd = $state("");
  let interests = $state("");
  let travelers = $state("");
  let transportModes = $state<TransportMode[]>([]);
  // A NUMBER, because `bind:value` on `<input type="number">` coerces to
  // `number | null` (svelte/src/internal/client/dom/elements/bindings/input.js:
  // `is_numberlike_input` -> `to_number`), and an empty field is null rather
  // than "". Holding it as a string typechecked and threw at runtime the moment
  // anybody touched the field. Euros in, integer minor units out -- a price in
  // floating point is a price that eventually disagrees with the receipt.
  let budget = $state<number | null>(null);
  let currency = $state("EUR");
  let validation = $state<string | null>(null);
  let deleteArmed = $state(false);

  $effect(() => {
    if (initializedPlanId === plan.id) return;
    initializedPlanId = plan.id;
    title = plan.title;
    origin = { ...plan.origin };
    destinations = plan.destinations.map((destination) => ({ ...destination }));
    dateStart = plan.date_start;
    dateEnd = plan.date_end;
    interests = plan.interests;
    travelers = plan.travelers.join(", ");
    transportModes = [...plan.transport_modes];
    budget = plan.budget_cents === null ? null : plan.budget_cents / 100;
    currency = plan.currency ?? "EUR";
    validation = null;
    deleteArmed = false;
  });

  function toggleMode(mode: TransportMode): void {
    transportModes = transportModes.includes(mode)
      ? transportModes.filter((current) => current !== mode)
      : [...transportModes, mode];
  }

  function addDestination(): void {
    if (destinations.length < 4) destinations = [...destinations, null];
  }

  function removeDestination(index: number): void {
    if (destinations.length <= 1) return;
    destinations = destinations.filter((_, current) => current !== index);
  }

  async function submit(): Promise<void> {
    const selectedDestinations = destinations.filter(
      (destination): destination is PlaceRef => destination !== null,
    );
    if (!title.trim() || !origin || selectedDestinations.length === 0) {
      validation = "A title, origin, and at least one destination are required.";
      return;
    }
    if (dateStart > dateEnd) {
      validation = "The end date must be after the start date.";
      return;
    }
    if (transportModes.length === 0) {
      validation = "Select at least one transport mode.";
      return;
    }
    const code = currency.trim().toUpperCase();
    if (!/^[A-Z]{3}$/.test(code)) {
      validation = "Currency is a three-letter ISO code, e.g. EUR.";
      return;
    }
    // `!(budget >= 0)` and not `budget < 0`: a half-typed entry reaches this as
    // NaN, which fails every comparison.
    if (budget !== null && !(budget >= 0)) {
      validation = "A budget is a number, and never negative.";
      return;
    }
    validation = null;
    await onSave({
      title: title.trim(),
      origin,
      destinations: selectedDestinations,
      date_start: dateStart,
      date_end: dateEnd,
      interests: interests.trim(),
      travelers: travelers
        .split(",")
        .map((traveler) => traveler.trim())
        .filter(Boolean),
      transport_modes: transportModes,
      // Without these two the budget half can never be populated from this UI,
      // which is why no plan carries one. `currency` is load-bearing twice: it
      // denominates the budget AND gates whether a retrospective may record a
      // cost at all.
      // An emptied field sends an explicit null, which `update_plan` reads as
      // "clear it" rather than as "not supplied".
      budget_cents: budget === null ? null : Math.round(budget * 100),
      currency: code,
    });
  }
</script>

<section class="editor card" aria-labelledby="plan-editor-title">
  <header>
    <div>
      <p class="eyebrow">Edit trip</p>
      <h2 id="plan-editor-title">{plan.title}</h2>
    </div>
    <button class="close" type="button" onclick={onCancel} aria-label="Close editor">
      <Icon name="close" size={16} />
    </button>
  </header>

  {#if validation}
    <p class="validation" aria-live="polite">{validation}</p>
  {/if}

  <form
    onsubmit={(event) => {
      event.preventDefault();
      void submit();
    }}
  >
    <label class="title-field">
      <span>Title</span>
      <input class="input" bind:value={title} />
    </label>

    <div class="route-grid">
      <PlaceField
        label="Origin"
        placeholder="Address, city, or station"
        bind:place={origin}
      />
      {#each destinations as _, index (`destination-${index}`)}
        <div class="destination-field">
          <PlaceField
            label={index === 0 ? "Destination" : `Stop ${index + 1}`}
            placeholder="Place, venue, or station"
            bind:place={destinations[index]}
          />
          {#if destinations.length > 1}
            <button
              class="remove-stop"
              type="button"
              onclick={() => removeDestination(index)}
            >
              Remove
            </button>
          {/if}
        </div>
      {/each}
      {#if destinations.length < 4}
        <button class="add-stop" type="button" onclick={addDestination}>
          <Icon name="plus" size={13} /> Add another stop
        </button>
      {/if}
    </div>

    <div class="detail-grid">
      <label>
        <span>From</span>
        <input class="input" type="date" bind:value={dateStart} />
      </label>
      <label>
        <span>To</span>
        <input class="input" type="date" min={dateStart} bind:value={dateEnd} />
      </label>
      <label>
        <span>Local interests</span>
        <input class="input" bind:value={interests} />
      </label>
      <label>
        <span>Travellers</span>
        <input class="input" bind:value={travelers} placeholder="Separate with commas" />
      </label>
      <label>
        <span>Budget</span>
        <input
          class="input"
          type="number"
          step="0.01"
          min="0"
          inputmode="decimal"
          bind:value={budget}
          placeholder="What it should cost"
        />
      </label>
      <label>
        <span>Currency</span>
        <input class="input" bind:value={currency} maxlength="3" placeholder="EUR" />
      </label>
    </div>

    <fieldset>
      <legend>Possible transport modes</legend>
      <div class="mode-list">
        {#each MODE_OPTIONS as option (option.id)}
          <button
            type="button"
            class:active={transportModes.includes(option.id)}
            aria-pressed={transportModes.includes(option.id)}
            onclick={() => toggleMode(option.id)}
          >
            {option.label}
          </button>
        {/each}
      </div>
    </fieldset>

    <footer>
      <div class="danger-zone">
        {#if deleteArmed}
          <button
            class="delete confirm"
            type="button"
            disabled={deleting}
            onclick={() => void onDelete()}
          >
            {deleting ? "Deleting…" : "Delete permanently from Sjel"}
          </button>
          <button type="button" onclick={() => (deleteArmed = false)}>Cancel deletion</button>
        {:else}
          <button class="delete" type="button" onclick={() => (deleteArmed = true)}>
            Delete trip
          </button>
        {/if}
        {#if plan.source?.kind === "obsidian"}
          <small>The original Obsidian note remains unchanged.</small>
        {/if}
      </div>
      <div class="primary-actions">
        <button type="button" onclick={onCancel}>Close editor</button>
        <button class="save" type="submit" disabled={saving}>
          {#if saving}<Icon name="loader" size={13} />{/if}
          {saving ? "Saving…" : "Save changes"}
        </button>
      </div>
    </footer>
  </form>
</section>

<style>
  .editor {
    padding: clamp(1rem, 2.5vw, 1.6rem);
    margin-bottom: 1rem;
  }

  header,
  footer,
  .primary-actions,
  .mode-list,
  .add-stop {
    display: flex;
    align-items: center;
  }

  header,
  footer {
    justify-content: space-between;
    gap: 1rem;
  }

  header {
    padding-bottom: 1rem;
    border-bottom: 1px solid var(--card-border);
  }

  .eyebrow {
    margin: 0 0 0.2rem;
    color: var(--primary);
    font-size: 0.625rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  h2 {
    margin: 0;
    font-size: 1.15rem;
  }

  button {
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text-secondary);
    font: inherit;
    cursor: pointer;
  }

  button:hover,
  button:focus-visible {
    border-color: var(--primary);
    color: var(--primary);
  }

  button:disabled {
    opacity: 0.55;
    cursor: wait;
  }

  .close {
    padding: 0.5rem;
  }

  form {
    display: grid;
    gap: 1rem;
    padding-top: 1rem;
  }

  label,
  fieldset {
    display: grid;
    gap: 0.3rem;
  }

  label > span,
  legend {
    color: var(--text-tertiary);
    font-size: 0.625rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .route-grid,
  .detail-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 0.8rem;
  }

  .destination-field {
    position: relative;
  }

  .remove-stop {
    margin-top: 0.3rem;
    padding: 0.25rem 0.45rem;
    border: 0;
    background: none;
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .add-stop {
    align-self: end;
    justify-content: center;
    gap: 0.4rem;
    min-height: 2.45rem;
    padding: 0.55rem;
    border-style: dashed;
    font-size: var(--text-2xs);
  }

  fieldset {
    padding: 0;
    border: 0;
  }

  .mode-list {
    flex-wrap: wrap;
    gap: 0.4rem;
  }

  .mode-list button {
    padding: 0.42rem 0.65rem;
    font-size: var(--text-2xs);
  }

  .mode-list button.active {
    border-color: var(--primary);
    background: var(--primary-soft);
    color: var(--primary);
  }

  footer {
    align-items: flex-end;
    padding-top: 1rem;
    border-top: 1px solid var(--card-border);
  }

  .danger-zone {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.45rem;
  }

  .danger-zone button,
  .primary-actions button {
    padding: 0.5rem 0.7rem;
    font-size: var(--text-2xs);
  }

  .danger-zone small {
    flex-basis: 100%;
    color: var(--text-tertiary);
    font-size: 0.5625rem;
  }

  .delete {
    color: var(--danger, #c55);
  }

  .delete.confirm {
    border-color: var(--danger, #c55);
    background: color-mix(in srgb, var(--danger, #c55) 10%, transparent);
  }

  .primary-actions {
    gap: 0.45rem;
  }

  .primary-actions .save {
    gap: 0.4rem;
    border-color: var(--primary);
    background: var(--primary);
    color: var(--button-primary-text, white);
  }

  .validation {
    margin: 0.8rem 0 0;
    color: var(--danger, #c55);
    font-size: var(--text-xs);
  }

  @media (max-width: 46rem) {
    .route-grid,
    .detail-grid {
      grid-template-columns: 1fr;
    }

    footer {
      align-items: stretch;
      flex-direction: column-reverse;
    }

    .primary-actions {
      justify-content: flex-end;
    }
  }
</style>
