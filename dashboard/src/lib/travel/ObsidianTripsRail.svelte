<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import RailSection from "$lib/rail/RailSection.svelte";
  import PlaceField from "$lib/travel/PlaceField.svelte";
  import type { ObsidianTripCandidate, PlaceRef } from "$lib/api";

  let {
    candidates,
    scanning = false,
    importing = null,
    importingAll = false,
    notice = null,
    importOrigin = $bindable(null),
    importOriginNeeded = $bindable(false),
    onScan,
    onImport,
    onImportAll,
  }: {
    candidates: ObsidianTripCandidate[];
    scanning?: boolean;
    importing?: string | null;
    importingAll?: boolean;
    notice?: string | null;
    importOrigin?: PlaceRef | null;
    importOriginNeeded?: boolean;
    onScan: () => void;
    onImport: (candidate: ObsidianTripCandidate) => void;
    onImportAll: () => void;
  } = $props();

  let importOriginField = $state<HTMLDivElement | null>(null);

  const shortDate = (value: string | null | undefined) => (value ? value.slice(5) : "");

  const readyCount = $derived(
    candidates.filter(
      (candidate) => candidate.issues.length === 0 && !candidate.imported_plan_id,
    ).length,
  );

  const pendingCount = $derived(
    candidates.filter((candidate) => !candidate.imported_plan_id).length,
  );

  export function focusOriginField(): void {
    queueMicrotask(() => importOriginField?.querySelector("input")?.focus());
  }
</script>

<RailSection label="Obsidian" count={pendingCount}>
  <p class="rail-hint">
    Scans only notes marked as trips. Everything remains unchanged until import.
  </p>

  {#if notice}
    <p class="rail-notice" aria-live="polite">{notice}</p>
  {/if}

  {#if candidates.length > 0}
    <div
      class="import-origin"
      class:missing={importOriginNeeded && !importOrigin}
      bind:this={importOriginField}
    >
      <PlaceField
        label="Origin for imports"
        placeholder="Applies to the trips imported now"
        bind:place={importOrigin}
      />
      <p>
        {#if importOriginNeeded && !importOrigin}
          Enter an origin or select one from the suggestions.
        {:else}
          Missing origins are not guessed. You can change them later for each trip.
        {/if}
      </p>
    </div>
    <ol class="import-list">
      {#each candidates as candidate (candidate.reference)}
        <li>
          <div>
            <strong>{candidate.title}</strong>
            <span>
              {shortDate(candidate.date_start)} – {shortDate(candidate.date_end)}
              · {candidate.destination?.name ?? "Destination open"}
            </span>
            {#if candidate.issues.length > 0}
              <small>{candidate.issues.join(" · ")}</small>
            {:else if candidate.travelers.length > 0}
              <small>{candidate.travelers.join(", ")}</small>
            {/if}
          </div>
          {#if candidate.imported_plan_id}
            <span class="imported"><Icon name="check" size={13} /> Imported</span>
          {:else}
            <button
              type="button"
              disabled={candidate.issues.length > 0 || importingAll || importing !== null}
              onclick={() => onImport(candidate)}
            >
              {importing === candidate.reference ? "Importing…" : "Import"}
            </button>
          {/if}
        </li>
      {/each}
    </ol>
  {/if}

  <div class="rail-actions">
    <button
      class="btn btn-outline"
      type="button"
      disabled={scanning || importingAll}
      onclick={onScan}
    >
      <Icon name={scanning ? "loader" : "database"} size={13} />
      {scanning ? "Scanning…" : "Scan"}
    </button>
    {#if candidates.length > 0}
      <button
        class="btn btn-primary"
        type="button"
        disabled={readyCount === 0 || importingAll || importing !== null}
        onclick={onImportAll}
      >
        <Icon name={importingAll ? "loader" : "check"} size={13} />
        {importingAll ? "Importing…" : `Import all (${readyCount})`}
      </button>
    {/if}
  </div>
</RailSection>

<style>
  .import-origin {
    margin: 0.5rem 0 0.85rem;
    padding: 0.65rem 0.75rem;
    border: 1px solid var(--rule);
    border-radius: var(--radius-md);
    background: var(--surface);
    display: grid;
    gap: 0.45rem;
  }

  .import-origin p {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.35;
    color: var(--text-secondary);
  }

  .import-origin.missing {
    border-color: var(--danger, #d32f2f);
    background: var(--danger-soft, rgba(211, 47, 47, 0.08));
  }

  .import-origin.missing p {
    color: var(--danger, #d32f2f);
    font-weight: 500;
  }

  .import-origin.missing :global(input) {
    border-color: var(--danger, #d32f2f);
    background: var(--card-bg);
  }

  .import-list {
    margin: 0 0 0.85rem;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 0.55rem;
  }

  .import-list li {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.6rem;
    padding-bottom: 0.55rem;
    border-bottom: 1px solid var(--rule);
  }

  .import-list strong,
  .import-list span,
  .import-list small {
    display: block;
  }

  .import-list strong {
    font-size: var(--text-sm);
    color: var(--text-primary);
  }

  .import-list span,
  .import-list small {
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .import-list small {
    margin-top: 0.15rem;
  }

  .import-list button,
  .imported {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.25rem 0.6rem;
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    font-weight: 600;
    white-space: nowrap;
  }

  .import-list button {
    border: 1px solid var(--input-border);
    background: var(--card-bg);
    color: var(--text-primary);
    cursor: pointer;
    transition: background var(--motion-fast) ease;
  }

  .import-list button:hover:not(:disabled) {
    background: var(--surface);
  }

  .import-list button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .import-list .imported {
    color: var(--success, #2e7d32);
    background: var(--success-soft, rgba(46, 125, 50, 0.12));
  }

  .rail-actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    margin-top: 0.5rem;
  }
</style>
