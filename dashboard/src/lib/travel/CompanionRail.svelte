<script lang="ts">
  /**
   * The waiting companion-register decisions, read-only, with a link to the one
   * place they are decided.
   *
   * Read-only on purpose. The review surface already exists on /map with its own
   * optimistic confirm/dismiss and rollback; a second set of buttons means a
   * second optimistic-update implementation that can disagree with the first.
   * What this section adds is the DATE RANGE, which the map card drops and which
   * is exactly what makes a proposal actionable while you are planning a trip.
   */
  import { link } from "$lib/nav";
  import Icon from "$lib/Icon.svelte";
  import type { PersonPlaceProposal } from "$lib/api";

  let {
    proposals,
    loading = false,
    /**
     * The capability's own sentence when places is absent. In the published demo
     * the shim answers 503 with a reason, and it is rendered verbatim rather than
     * translated: a synthetic companion register is the one fixture this demo
     * must not have.
     */
    notice = null,
  }: {
    proposals: PersonPlaceProposal[];
    loading?: boolean;
    notice?: string | null;
  } = $props();

  const shortDate = (iso: string | null) => (iso ? iso.slice(0, 10) : null);

  /** The range as one string, with an open end said rather than left blank. */
  function range(proposal: PersonPlaceProposal): string {
    const from = shortDate(proposal.date_start);
    const to = shortDate(proposal.date_end);
    if (from && to) return `${from} – ${to}`;
    if (from) return `since ${from}`;
    if (to) return `until ${to}`;
    return "no dates — reads as “lives there”";
  }
</script>

<p class="rail-hint">
  Proposed companion-register rows waiting for a decision. Confirming one is what lets a
  plan say a known companion is at a destination — without ever naming them.
</p>

{#if notice}
  <p class="rail-notice" aria-live="polite">{notice}</p>
{:else if loading}
  <p class="rail-empty">Checking the companion register…</p>
{:else if proposals.length === 0}
  <p class="rail-empty">No companion-register rows are waiting for a decision.</p>
{:else}
  <ol class="companion-list">
    {#each proposals as proposal (proposal.id)}
      <li>
        <p class="who">
          <strong>{proposal.person}</strong>
          <span>{proposal.place_name}{proposal.city ? ` · ${proposal.city}` : ""}</span>
        </p>
        <p class="when">{range(proposal)}</p>
        <p class="meta">
          {(proposal.confidence_bp / 100).toFixed(0)}% confidence · {proposal.source}
        </p>
      </li>
    {/each}
  </ol>
{/if}

<a class="btn btn-outline rail-action" href={link("/map")}>
  <Icon name="map-pin" size={13} /> Decide on the map
</a>

<style>
  /* The three rail lines below are a copy of the ones in
   * `src/routes/travel/+page.svelte` (the "Shared by both rail sections" block),
   * kept here because Svelte scopes a component's CSS to that component: the
   * page's rules carry the page's scope class and never reach these elements,
   * so without the copy the hint, the notice and the empty line render as
   * unstyled paragraphs inside a styled rail. `src/app.css` would be the one
   * home for them, and it belongs to another stream tonight; a local copy is
   * the smaller merge. */
  .rail-hint {
    margin: 0 0 0.4rem;
    color: var(--text-secondary);
    font-size: var(--text-xs);
    line-height: 1.45;
  }

  .rail-notice {
    margin: 0 0 0.4rem;
    padding: 0.4rem 0.5rem;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
    color: var(--text-secondary);
    font-size: 0.72rem;
    line-height: 1.4;
  }

  .rail-empty {
    margin: 0;
    color: var(--text-tertiary);
    font-size: 0.78rem;
  }

  .companion-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin: 0 0 0.6rem;
    padding: 0;
    list-style: none;
  }

  .companion-list li {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    padding: 0.5rem 0.6rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
  }

  .who {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.4rem;
    margin: 0;
    font-size: var(--text-sm);
  }

  .who span {
    color: var(--text-secondary);
  }

  .when {
    margin: 0;
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-primary);
  }

  .meta {
    margin: 0;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  /* Sized like the other rails' action button, for the same reason. */
  .rail-action {
    display: inline-flex;
    width: 100%;
    align-items: center;
    justify-content: center;
    gap: 0.35rem;
    margin-top: 0.5rem;
    padding: 0.3rem 0.5rem;
    font-size: 0.72rem;
    text-decoration: none;
  }
</style>
