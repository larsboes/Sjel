<script lang="ts">
  import { tip } from "$lib/tip";
  import Icon from "$lib/Icon.svelte";
  import RailSection from "$lib/rail/RailSection.svelte";
  import type { TravelCandidateAssessment } from "$lib/travel/travel-candidates";
  import type { TripPlan } from "$lib/api";

  let {
    candidates,
    loading = false,
    notice = null,
    savingId = null,
    savedIds = new Set<string>(),
    onAddCandidate,
    onOpenPlan,
    onSeedPlan,
    onRefresh,
  }: {
    candidates: TravelCandidateAssessment[];
    loading?: boolean;
    notice?: string | null;
    savingId?: string | null;
    savedIds?: Set<string>;
    onAddCandidate: (candidate: TravelCandidateAssessment) => void;
    onOpenPlan: (plan: TripPlan) => void;
    onSeedPlan: (candidate: TravelCandidateAssessment) => void;
    onRefresh: () => void;
  } = $props();

  const shortDate = (value: string | null | undefined) => (value ? value.slice(5) : "");

  function travelCandidateLabel(candidate: TravelCandidateAssessment): string {
    switch (candidate.state) {
      case "matching_trip": return "Matches a trip";
      case "free_window": return "Free window";
      case "needs_travel_day": return "Travel day needed";
      case "conflicts": return "Calendar says no";
      case "date_unresolved": return "Date unresolved";
      case "calendar_unavailable": return "Calendar unavailable";
    }
  }
</script>

<RailSection label="Candidates" count={candidates.length}>
  <p class="rail-hint">
    Far events matched to dated trip destinations or checked against your Calendar.
  </p>

  {#if notice}
    <p class="rail-notice" aria-live="polite">{notice}</p>
  {/if}

  {#if loading}
    <p class="rail-empty">Checking Scouting, Trips, and Calendar…</p>
  {:else if candidates.length === 0}
    <p class="rail-empty">No undismissed travel-candidate events are waiting.</p>
  {:else}
    <ol class="candidate-list">
      {#each candidates as candidate (candidate.opportunity.id)}
        <li class:blocked={candidate.state === "conflicts"}>
          <p class="candidate-when">
            <strong>{shortDate(candidate.opportunity.starts_at.slice(0, 10))}</strong>
            <span>{candidate.opportunity.city || candidate.opportunity.location || "Place open"}</span>
          </p>
          <span class="candidate-state {candidate.state}">{travelCandidateLabel(candidate)}</span>
          <a class="candidate-title" href={candidate.opportunity.url} target="_blank" rel="noreferrer">
            {candidate.opportunity.title}
          </a>
          <small use:tip={candidate.reason}>{candidate.reason}</small>
          <div class="candidate-actions">
            {#if candidate.plan_match}
              <button
                type="button"
                disabled={savingId !== null || savedIds.has(candidate.opportunity.id)}
                onclick={() => onAddCandidate(candidate)}
              >
                <Icon
                  name={savedIds.has(candidate.opportunity.id) ? "check" : "plus"}
                  size={13}
                />
                {savedIds.has(candidate.opportunity.id)
                  ? "Added"
                  : savingId === candidate.opportunity.id
                    ? "Adding…"
                    : "Add to trip"}
              </button>
              <button type="button" onclick={() => onOpenPlan(candidate.plan_match!.plan)}>
                Open trip
              </button>
            {:else if ["free_window", "needs_travel_day"].includes(candidate.state)}
              <button type="button" onclick={() => onSeedPlan(candidate)}>
                <Icon name="map-pin" size={13} /> Plan around event
              </button>
            {/if}
          </div>
        </li>
      {/each}
    </ol>
  {/if}

  <button
    class="btn btn-outline rail-action"
    type="button"
    disabled={loading}
    onclick={onRefresh}
  >
    <Icon name={loading ? "loader" : "refresh"} size={13} />
    {loading ? "Checking…" : "Refresh"}
  </button>
</RailSection>

<style>
  .candidate-list {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 0.65rem;
  }

  .candidate-list li {
    display: grid;
    gap: 0.25rem;
    padding-bottom: 0.65rem;
    border-bottom: 1px solid var(--rule);
  }

  .candidate-list li.blocked .candidate-title {
    opacity: 0.7;
  }

  .candidate-when {
    margin: 0;
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 0.5rem;
    font-size: var(--text-xs);
  }

  .candidate-when strong {
    color: var(--text-primary);
    font-weight: 600;
  }

  .candidate-when span {
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: right;
  }

  .candidate-title {
    color: var(--text-primary);
    text-decoration: none;
    font-weight: 600;
    line-height: 1.25;
    word-break: break-word;
    font-size: var(--text-sm);
  }

  .candidate-title:hover {
    color: var(--primary);
    text-decoration: underline;
  }

  .candidate-list small {
    color: var(--text-secondary);
    line-height: 1.35;
    overflow: hidden;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }

  .candidate-state {
    display: inline-flex;
    align-items: center;
    width: fit-content;
    padding: 0.15rem 0.45rem;
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    font-weight: 600;
    background: var(--surface);
    color: var(--text-secondary);
  }

  .candidate-state.matching_trip,
  .candidate-state.free_window {
    background: var(--success-soft, rgba(46, 125, 50, 0.12));
    color: var(--success, #2e7d32);
  }

  .candidate-state.needs_travel_day {
    background: var(--primary-soft);
    color: var(--primary);
  }

  .candidate-state.conflicts {
    background: var(--danger-soft, rgba(216, 34, 34, 0.08));
    color: var(--danger, #d82222);
  }

  .candidate-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    margin-top: 0.2rem;
  }

  .candidate-actions button {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.25rem 0.55rem;
    border: 1px solid var(--input-border);
    border-radius: var(--radius-sm);
    background: var(--card-bg);
    color: var(--text-primary);
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 500;
    cursor: pointer;
    transition: background var(--motion-fast) ease, border-color var(--motion-fast) ease;
  }

  .candidate-actions button:hover:not(:disabled) {
    background: var(--surface);
    border-color: var(--card-border-hover);
  }

  .candidate-actions button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
