<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import Overlay from "$lib/Overlay.svelte";
  import {
    axonStatus,
    traveler,
    type TravelProfile,
    type DerivedTravelStats,
    type Provenance,
  } from "$lib/api";

  let {
    onClose,
  }: {
    onClose: () => void;
  } = $props();

  let loading = $state(true);
  let saving = $state(false);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);

  let activeTab = $state<"journey" | "destination" | "constraints" | "derived">("journey");

  let profile = $state<TravelProfile | null>(null);
  let stored = $state(false);
  let revision = $state(0);
  let derivedStats = $state<DerivedTravelStats | null>(null);

  // Editable fields for Hard Constraints
  let homeStationsText = $state("");
  let homeAirportsText = $state("");
  let cardsText = $state("");

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    error = null;
    try {
      await axonStatus.start("traveler").catch(() => undefined);
      const [profileRes, statsRes] = await Promise.all([
        traveler.profile(),
        traveler.derived().catch(() => null),
      ]);
      profile = profileRes.profile;
      stored = profileRes.stored;
      revision = profileRes.revision;
      derivedStats = statsRes;

      homeStationsText = profile.hard.home_stations.join(", ");
      homeAirportsText = profile.hard.home_airports.join(", ");
      cardsText = profile.hard.cards.join(", ");
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  // Weight sums and normalization
  const journeySum = $derived(
    profile
      ? profile.journey.price +
          profile.journey.duration +
          profile.journey.changes +
          profile.journey.reliability
      : 0,
  );

  const destinationSum = $derived(
    profile
      ? profile.soft.budget_fit +
          profile.soft.feasibility +
          profile.soft.season +
          profile.soft.events +
          profile.soft.retrospective
      : 0,
  );

  const journeySumPercent = $derived(Math.round(journeySum * 100));
  const destinationSumPercent = $derived(Math.round(destinationSum * 100));

  function normalizeJourney(): void {
    if (!profile || journeySum <= 0) return;
    const factor = 1.0 / journeySum;
    profile.journey.price = Math.round(profile.journey.price * factor * 100) / 100;
    profile.journey.duration = Math.round(profile.journey.duration * factor * 100) / 100;
    profile.journey.changes = Math.round(profile.journey.changes * factor * 100) / 100;
    profile.journey.reliability =
      Math.round(
        (1.0 - (profile.journey.price + profile.journey.duration + profile.journey.changes)) * 100,
      ) / 100;

    markStated("journey.price");
    markStated("journey.duration");
    markStated("journey.changes");
    markStated("journey.reliability");
  }

  function normalizeDestination(): void {
    if (!profile || destinationSum <= 0) return;
    const factor = 1.0 / destinationSum;
    profile.soft.budget_fit = Math.round(profile.soft.budget_fit * factor * 100) / 100;
    profile.soft.feasibility = Math.round(profile.soft.feasibility * factor * 100) / 100;
    profile.soft.season = Math.round(profile.soft.season * factor * 100) / 100;
    profile.soft.events = Math.round(profile.soft.events * factor * 100) / 100;
    profile.soft.retrospective =
      Math.round(
        (1.0 -
          (profile.soft.budget_fit +
            profile.soft.feasibility +
            profile.soft.season +
            profile.soft.events)) *
          100,
      ) / 100;

    markStated("soft.budget_fit");
    markStated("soft.feasibility");
    markStated("soft.season");
    markStated("soft.events");
    markStated("soft.retrospective");
  }

  function markStated(key: string): void {
    if (!profile) return;
    profile.basis = { ...profile.basis, [key]: "stated" };
  }

  async function save(): Promise<void> {
    if (!profile) return;

    if (Math.abs(journeySum - 1.0) > 0.01) {
      error = `Journey weights must sum to 100% (currently ${journeySumPercent}%). Click "Normalize" or adjust sliders.`;
      return;
    }

    if (Math.abs(destinationSum - 1.0) > 0.01) {
      error = `Destination weights must sum to 100% (currently ${destinationSumPercent}%). Click "Normalize" or adjust sliders.`;
      return;
    }

    saving = true;
    error = null;
    notice = null;

    // Apply parsed strings to hard constraints
    profile.hard.home_stations = homeStationsText
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    profile.hard.home_airports = homeAirportsText
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    profile.hard.cards = cardsText
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);

    try {
      const res = await traveler.updateProfile(profile, revision);
      profile = res.profile;
      stored = res.stored;
      revision = res.revision;
      notice = "Traveler profile saved successfully.";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  function provenanceBadge(prov?: Provenance): { text: string; cls: string } {
    switch (prov) {
      case "stated":
        return { text: "Stated", cls: "stated" };
      case "derived":
        return { text: "Derived", cls: "derived" };
      case "vault":
        return { text: "Vault", cls: "vault" };
      default:
        return { text: "Default", cls: "default" };
    }
  }
</script>

<Overlay
  title="Traveller profile"
  eyebrow="Preferences & constraints"
  width="46rem"
  busy={saving}
  {onClose}
>
  <p class="profile-lede">
    Personalized ranking weights, hard connection limits, and travel history insights.
  </p>

  {#if error}
    <p class="notice error"><Icon name="alert" size={16} /> {error}</p>
  {/if}
  {#if notice}
    <p class="notice success" aria-live="polite"><Icon name="check" size={16} /> {notice}</p>
  {/if}

  <div class="tabs" role="tablist">
    <button
      type="button"
      class:active={activeTab === "journey"}
      onclick={() => (activeTab = "journey")}
    >
      Journey weights
    </button>
    <button
      type="button"
      class:active={activeTab === "destination"}
      onclick={() => (activeTab = "destination")}
    >
      Destination weights
    </button>
    <button
      type="button"
      class:active={activeTab === "constraints"}
      onclick={() => (activeTab = "constraints")}
    >
      Hard limits & anchors
    </button>
    <button
      type="button"
      class:active={activeTab === "derived"}
      onclick={() => (activeTab = "derived")}
    >
      Historical baseline
    </button>
  </div>

  {#if loading}
    <div class="loading-state">
      <Icon name="loader" size={20} /> Loading traveller profile…
    </div>
  {:else if profile}
    {#if activeTab === "journey"}
      <section class="tab-panel">
        <header class="panel-header">
          <div>
            <h3>Connection ranking priorities</h3>
            <p>Weighs how connections are ordered in search and trips.</p>
          </div>
          <div class="sum-badge" class:valid={Math.abs(journeySum - 1.0) <= 0.01}>
            Total: {journeySumPercent}%
            {#if Math.abs(journeySum - 1.0) > 0.01}
              <button class="btn btn-sm btn-outline" type="button" onclick={normalizeJourney}>
                Normalize
              </button>
            {/if}
          </div>
        </header>

        <div class="weight-controls">
          <div class="control-row">
            <div class="label-group">
              <span class="weight-name">Price</span>
              <span class="prov-tag {provenanceBadge(profile.basis['journey.price']).cls}">
                {provenanceBadge(profile.basis['journey.price']).text}
              </span>
            </div>
            <input
              type="range"
              min="0"
              max="1"
              step="0.05"
              bind:value={profile.journey.price}
              aria-label="Journey weight: price"
              oninput={() => markStated("journey.price")}
            />
            <span class="weight-val">{Math.round(profile.journey.price * 100)}%</span>
          </div>

          <div class="control-row">
            <div class="label-group">
              <span class="weight-name">Duration</span>
              <span class="prov-tag {provenanceBadge(profile.basis['journey.duration']).cls}">
                {provenanceBadge(profile.basis['journey.duration']).text}
              </span>
            </div>
            <input
              type="range"
              min="0"
              max="1"
              step="0.05"
              bind:value={profile.journey.duration}
              aria-label="Journey weight: duration"
              oninput={() => markStated("journey.duration")}
            />
            <span class="weight-val">{Math.round(profile.journey.duration * 100)}%</span>
          </div>

          <div class="control-row">
            <div class="label-group">
              <span class="weight-name">Reliability</span>
              <span class="prov-tag {provenanceBadge(profile.basis['journey.reliability']).cls}">
                {provenanceBadge(profile.basis['journey.reliability']).text}
              </span>
            </div>
            <input
              type="range"
              min="0"
              max="1"
              step="0.05"
              bind:value={profile.journey.reliability}
              aria-label="Journey weight: reliability"
              oninput={() => markStated("journey.reliability")}
            />
            <span class="weight-val">{Math.round(profile.journey.reliability * 100)}%</span>
          </div>

          <div class="control-row">
            <div class="label-group">
              <span class="weight-name">Fewest Changes</span>
              <span class="prov-tag {provenanceBadge(profile.basis['journey.changes']).cls}">
                {provenanceBadge(profile.basis['journey.changes']).text}
              </span>
            </div>
            <input
              type="range"
              min="0"
              max="1"
              step="0.05"
              bind:value={profile.journey.changes}
              aria-label="Journey weight: changes"
              oninput={() => markStated("journey.changes")}
            />
            <span class="weight-val">{Math.round(profile.journey.changes * 100)}%</span>
          </div>
        </div>
      </section>
    {:else if activeTab === "destination"}
      <section class="tab-panel">
        <header class="panel-header">
          <div>
            <h3>Destination search ranking</h3>
            <p>Weights used by plan-search when suggesting where to go.</p>
          </div>
          <div class="sum-badge" class:valid={Math.abs(destinationSum - 1.0) <= 0.01}>
            Total: {destinationSumPercent}%
            {#if Math.abs(destinationSum - 1.0) > 0.01}
              <button class="btn btn-sm btn-outline" type="button" onclick={normalizeDestination}>
                Normalize
              </button>
            {/if}
          </div>
        </header>

        <div class="weight-controls">
          <div class="control-row">
            <div class="label-group">
              <span class="weight-name">Budget Fit</span>
              <span class="prov-tag {provenanceBadge(profile.basis['soft.budget_fit']).cls}">
                {provenanceBadge(profile.basis['soft.budget_fit']).text}
              </span>
            </div>
            <input
              type="range"
              min="0"
              max="1"
              step="0.05"
              bind:value={profile.soft.budget_fit}
              aria-label="Ranking weight: budget fit"
              oninput={() => markStated("soft.budget_fit")}
            />
            <span class="weight-val">{Math.round(profile.soft.budget_fit * 100)}%</span>
          </div>

          <div class="control-row">
            <div class="label-group">
              <span class="weight-name">Feasibility (Calendar)</span>
              <span class="prov-tag {provenanceBadge(profile.basis['soft.feasibility']).cls}">
                {provenanceBadge(profile.basis['soft.feasibility']).text}
              </span>
            </div>
            <input
              type="range"
              min="0"
              max="1"
              step="0.05"
              bind:value={profile.soft.feasibility}
              aria-label="Ranking weight: feasibility"
              oninput={() => markStated("soft.feasibility")}
            />
            <span class="weight-val">{Math.round(profile.soft.feasibility * 100)}%</span>
          </div>

          <div class="control-row">
            <div class="label-group">
              <span class="weight-name">Season & Climate</span>
              <span class="prov-tag {provenanceBadge(profile.basis['soft.season']).cls}">
                {provenanceBadge(profile.basis['soft.season']).text}
              </span>
            </div>
            <input
              type="range"
              min="0"
              max="1"
              step="0.05"
              bind:value={profile.soft.season}
              aria-label="Ranking weight: season"
              oninput={() => markStated("soft.season")}
            />
            <span class="weight-val">{Math.round(profile.soft.season * 100)}%</span>
          </div>

          <div class="control-row">
            <div class="label-group">
              <span class="weight-name">Events & Opportunities</span>
              <span class="prov-tag {provenanceBadge(profile.basis['soft.events']).cls}">
                {provenanceBadge(profile.basis['soft.events']).text}
              </span>
            </div>
            <input
              type="range"
              min="0"
              max="1"
              step="0.05"
              bind:value={profile.soft.events}
              aria-label="Ranking weight: events"
              oninput={() => markStated("soft.events")}
            />
            <span class="weight-val">{Math.round(profile.soft.events * 100)}%</span>
          </div>

          <div class="control-row">
            <div class="label-group">
              <span class="weight-name">Past Retrospectives</span>
              <span class="prov-tag {provenanceBadge(profile.basis['soft.retrospective']).cls}">
                {provenanceBadge(profile.basis['soft.retrospective']).text}
              </span>
            </div>
            <input
              type="range"
              min="0"
              max="1"
              step="0.05"
              bind:value={profile.soft.retrospective}
              aria-label="Ranking weight: retrospective"
              oninput={() => markStated("soft.retrospective")}
            />
            <span class="weight-val">{Math.round(profile.soft.retrospective * 100)}%</span>
          </div>
        </div>
      </section>
    {:else if activeTab === "constraints"}
      <section class="tab-panel">
        <header class="panel-header">
          <div>
            <h3>Hard limits & Anchors</h3>
            <p>Strict boundaries that connections and flights must respect.</p>
          </div>
        </header>

        <div class="form-grid">
          <label>
            <span>Home Stations (order of preference)</span>
            <input
              class="input"
              bind:value={homeStationsText}
              placeholder="e.g. Bonn Hbf, Köln Hbf"
              oninput={() => markStated("hard.home_stations")}
            />
          </label>

          <label>
            <span>Home Airports (IATA codes)</span>
            <input
              class="input"
              bind:value={homeAirportsText}
              placeholder="e.g. CGN, DUS, FRA"
              oninput={() => markStated("hard.home_airports")}
            />
          </label>

          <label>
            <span>Discount Cards</span>
            <input
              class="input"
              bind:value={cardsText}
              placeholder="e.g. bahncard_25, deutschlandticket"
              oninput={() => markStated("hard.cards")}
            />
          </label>

          <div class="row-2">
            <label>
              <span>Earliest Departure</span>
              <input
                class="input"
                type="time"
                bind:value={profile.hard.earliest_departure}
                oninput={() => markStated("hard.earliest_departure")}
              />
            </label>
            <label>
              <span>Latest Arrival</span>
              <input
                class="input"
                type="time"
                bind:value={profile.hard.latest_arrival}
                oninput={() => markStated("hard.latest_arrival")}
              />
            </label>
          </div>

          <div class="row-2">
            <label>
              <span>Max Changes</span>
              <input
                class="input"
                type="number"
                min="0"
                max="10"
                bind:value={profile.hard.max_changes}
                placeholder="No limit"
                oninput={() => markStated("hard.max_changes")}
              />
            </label>
            <label>
              <span>Min Transfer Buffer (minutes)</span>
              <input
                class="input"
                type="number"
                min="0"
                step="5"
                bind:value={profile.hard.min_transfer_buffer_min}
                placeholder="Standard buffer"
                oninput={() => markStated("hard.min_transfer_buffer_min")}
              />
            </label>
          </div>

          <label class="checkbox-label">
            <input
              type="checkbox"
              bind:checked={profile.hard.avoid_overnight_travel}
              onchange={() => markStated("hard.avoid_overnight_travel")}
            />
            <span>Avoid overnight travel</span>
          </label>
        </div>
      </section>
    {:else if activeTab === "derived"}
      <section class="tab-panel">
        <header class="panel-header">
          <div>
            <h3>Derived historical baseline</h3>
            <p>Pattern arithmetic computed from your recorded trips in Sjel.</p>
          </div>
        </header>

        {#if derivedStats}
          <div class="metrics-grid">
            <div class="metric-card">
              <span class="metric-val">{derivedStats.counts.total_plans}</span>
              <span class="metric-label">Total Plans</span>
              <small>{derivedStats.counts.not_taken_plans} not taken</small>
            </div>

            <div class="metric-card">
              <span class="metric-val">
                {derivedStats.lead_time_days ? `${derivedStats.lead_time_days.median} d` : "—"}
              </span>
              <span class="metric-label">Median Lead Time</span>
              <small>
                {derivedStats.lead_time_days
                  ? `${derivedStats.lead_time_days.min}–${derivedStats.lead_time_days.max} days spread`
                  : "No lead time data"}
              </small>
            </div>

            <div class="metric-card">
              <span class="metric-val">
                {derivedStats.trip_length_days ? `${derivedStats.trip_length_days.median} d` : "—"}
              </span>
              <span class="metric-label">Median Trip Length</span>
              <small>
                {derivedStats.trip_length_days
                  ? `${derivedStats.trip_length_days.min}–${derivedStats.trip_length_days.max} days spread`
                  : "No length data"}
              </small>
            </div>
          </div>

          <div class="company-shape card">
            <h4>Travel Company Breakdown</h4>
            <div class="company-bars">
              <div>
                <span>Solo</span>
                <strong>{derivedStats.company_shape.solo}</strong>
              </div>
              <div>
                <span>Pair</span>
                <strong>{derivedStats.company_shape.pair}</strong>
              </div>
              <div>
                <span>Group</span>
                <strong>{derivedStats.company_shape.group}</strong>
              </div>
              <div>
                <span>Unrecorded</span>
                <strong>{derivedStats.company_shape.unrecorded}</strong>
              </div>
            </div>
          </div>

          {#if derivedStats.notes.length > 0}
            <div class="derived-notes">
              <h4>Derivation Constraints & Limits</h4>
              <ul>
                {#each derivedStats.notes as note, i (i)}
                  <li>{note}</li>
                {/each}
              </ul>
            </div>
          {/if}
        {:else}
          <p class="empty-state">No derived history available yet.</p>
        {/if}
      </section>
    {/if}

    <footer class="modal-footer">
      <span class="status-indicator">
        Revision: {revision} · {stored ? "Stored in database" : "Using defaults"}
      </span>
      <div class="footer-actions">
        <button class="btn btn-outline" type="button" onclick={onClose}>Close</button>
        <button class="btn btn-primary" type="button" disabled={saving} onclick={save}>
          {#if saving}<Icon name="loader" size={14} /> Saving…{:else}Save profile{/if}
        </button>
      </div>
    </footer>
  {/if}
</Overlay>

<style>
  .profile-lede {
    margin: -0.25rem 0 1rem;
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .notice {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.65rem 0.85rem;
    margin-bottom: 1rem;
    border-radius: var(--radius-md);
    font-size: var(--text-sm);
  }

  .notice.error {
    background: var(--danger-soft);
    color: var(--danger);
  }

  .notice.success {
    background: var(--success-soft);
    color: var(--success);
  }

  .tabs {
    display: flex;
    gap: 0.5rem;
    margin-bottom: 1.25rem;
    border-bottom: 1px solid var(--card-border);
    padding-bottom: 0.5rem;
  }

  .tabs button {
    padding: 0.4rem 0.65rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-tertiary);
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 600;
    cursor: pointer;
    transition: background-color var(--motion-fast) ease, color var(--motion-fast) ease;
  }

  .tabs button:hover {
    color: var(--text-primary);
  }

  .tabs button.active {
    background: var(--primary-soft);
    color: var(--primary);
  }

  .tab-panel {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }

  .panel-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
  }

  .panel-header h3 {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
  }

  .panel-header p {
    margin: 0.2rem 0 0;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .sum-badge {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.25rem 0.6rem;
    border-radius: var(--radius-sm);
    background: var(--warning-soft);
    color: var(--warning-ink);
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .sum-badge.valid {
    background: var(--success-soft);
    color: var(--success);
  }

  .weight-controls {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 1rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .control-row {
    display: grid;
    grid-template-columns: 10rem 1fr 3.5rem;
    align-items: center;
    gap: 1rem;
  }

  .label-group {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .weight-name {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-primary);
  }

  .weight-val {
    font-family: var(--font-mono, monospace);
    font-size: var(--text-xs);
    text-align: right;
    font-weight: 600;
  }

  .prov-tag {
    font-size: var(--text-2xs);
    font-weight: 600;
    padding: 0.1rem 0.3rem;
    border-radius: var(--radius-sm);
    text-transform: uppercase;
  }

  .prov-tag.stated {
    background: var(--primary-soft);
    color: var(--primary);
  }

  .prov-tag.derived {
    background: var(--surface-secondary, rgba(0, 0, 0, 0.05));
    color: var(--text-secondary);
  }

  .prov-tag.default {
    background: transparent;
    color: var(--text-tertiary);
    border: 1px solid var(--card-border);
  }

  .form-grid {
    display: flex;
    flex-direction: column;
    gap: 0.85rem;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    font-size: var(--text-xs);
    color: var(--text-secondary);
    font-weight: 500;
  }

  .row-2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.75rem;
  }

  .checkbox-label {
    flex-direction: row;
    align-items: center;
    gap: 0.5rem;
    cursor: pointer;
  }

  .metrics-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.75rem;
  }

  .metric-card {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    padding: 0.85rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .metric-val {
    font-size: var(--text-xl);
    font-weight: 700;
    color: var(--primary);
  }

  .metric-label {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-primary);
  }

  .metric-card small {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .company-shape {
    padding: 0.85rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .company-shape h4 {
    margin: 0 0 0.5rem;
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .company-bars {
    display: flex;
    gap: 1.5rem;
  }

  .company-bars div {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    font-size: var(--text-xs);
  }

  .company-bars strong {
    font-size: var(--text-base);
    color: var(--text-primary);
  }

  .derived-notes {
    padding: 0.85rem;
    border-radius: var(--radius);
    background: var(--card-bg);
    border: 1px solid var(--card-border);
  }

  .derived-notes h4 {
    margin: 0 0 0.4rem;
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .derived-notes ul {
    margin: 0;
    padding-left: 1.25rem;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .modal-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-top: 1.5rem;
    padding-top: 1rem;
    border-top: 1px solid var(--card-border);
  }

  .status-indicator {
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .footer-actions {
    display: flex;
    gap: 0.5rem;
  }

  .loading-state,
  .empty-state {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    padding: 3rem 1rem;
    color: var(--text-tertiary);
    font-size: var(--text-sm);
  }
</style>
