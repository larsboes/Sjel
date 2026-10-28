<script lang="ts">
  import SidePeek from "$lib/ui/SidePeek.svelte";
  import Property from "$lib/ui/Property.svelte";
  import Chip from "$lib/ui/Chip.svelte";
  import Icon from "$lib/Icon.svelte";
  import { tip } from "$lib/tip";
  import type { FeedEntry, FeedStatus } from "$lib/api";
  import EvaluationBreakdown from "./EvaluationBreakdown.svelte";
  import { FEED_KIND_LABEL } from "./FeedItemRow.svelte";

  let {
    entry,
    busy = false,
    onClose,
    onSetStatus,
  }: {
    entry: FeedEntry;
    busy?: boolean;
    onClose: () => void;
    onSetStatus: (id: string, status: FeedStatus) => Promise<void>;
  } = $props();

  const statusOptions = [
    { value: "new", label: "New" },
    { value: "keeper", label: "Kept" },
    { value: "dismissed", label: "Dismissed" },
  ];

  const travelFactor = $derived(
    entry.evaluation?.factors.find((f) => f.key === "travel" && f.score > 0),
  );

  function formatDate(iso: string): string {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return iso;
    return d.toLocaleDateString("en-GB", {
      day: "numeric",
      month: "short",
      year: "numeric",
    });
  }

</script>

<SidePeek
  title={entry.title ?? entry.url}
  eyebrow={FEED_KIND_LABEL[entry.kind] ?? entry.kind}
  {onClose}
>
  <div class="properties">
    <Property
      label="Status"
      kind="select"
      value={entry.status}
      options={statusOptions}
      onCommit={(v) => v && onSetStatus(entry.id, v as FeedStatus)}
    />
    <Property
      label="Type"
      value={FEED_KIND_LABEL[entry.kind] ?? entry.kind}
      readonly
    />
    {#if entry.author}
      <Property label="Author" value={entry.author} readonly />
    {/if}
    <Property label="Ingested" value={formatDate(entry.created_at)} readonly />
    {#if entry.relevance?.profile_label}
      <Property label="TELOS Lens" value={entry.relevance.profile_label} readonly />
    {/if}
    {#if travelFactor?.context}
      <Property
        label="Trip Context"
        value={`${travelFactor.context.label} (${Math.round(travelFactor.score * 100)}% fit)`}
        readonly
      />
    {/if}
  </div>

  {#if entry.summary}
    <div class="section">
      <h3 class="section-title">Summary</h3>
      <p class="summary-text">{entry.summary}</p>
    </div>
  {/if}

  {#if entry.evaluation}
    <div class="section">
      <h3 class="section-title">Evaluation</h3>
      <EvaluationBreakdown evaluation={entry.evaluation} />
    </div>
  {/if}

  {#snippet footer()}
    <div class="peek-footer">
      <div class="status-actions">
        {#if entry.status === "new"}
          <button
            type="button"
            class="act-btn success"
            disabled={busy}
            onclick={() => onSetStatus(entry.id, "keeper")}
          >
            <Icon name="check" size={13} />
            Keep
          </button>
          <button
            type="button"
            class="act-btn danger"
            disabled={busy}
            onclick={() => onSetStatus(entry.id, "dismissed")}
          >
            <Icon name="close" size={13} />
            Dismiss
          </button>
        {:else if entry.status === "keeper"}
          <button
            type="button"
            class="act-btn"
            disabled={busy}
            onclick={() => onSetStatus(entry.id, "new")}
          >
            Mark New
          </button>
          <button
            type="button"
            class="act-btn danger"
            disabled={busy}
            onclick={() => onSetStatus(entry.id, "dismissed")}
          >
            Dismiss
          </button>
        {:else}
          <button
            type="button"
            class="act-btn success"
            disabled={busy}
            onclick={() => onSetStatus(entry.id, "keeper")}
          >
            Keep
          </button>
          <button
            type="button"
            class="act-btn"
            disabled={busy}
            onclick={() => onSetStatus(entry.id, "new")}
          >
            Mark New
          </button>
        {/if}
      </div>

      <a
        href={entry.url}
        target="_blank"
        rel="noreferrer"
        class="external-btn"
        aria-label="Open original source URL"
        use:tip={"Open source link"}
      >
        <Icon name="external" size={14} />
      </a>
    </div>
  {/snippet}
</SidePeek>

<style>
  .properties {
    display: flex;
    flex-direction: column;
    border-bottom: 1px solid var(--card-border);
    margin-bottom: var(--space-3);
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-bottom: var(--space-4);
  }

  .section-title {
    font-size: var(--text-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-tertiary);
    margin: 0;
  }

  .summary-text {
    font-size: var(--text-sm);
    line-height: var(--leading-relaxed, 1.55);
    color: var(--text-secondary);
    margin: 0;
    white-space: pre-wrap;
  }

  .peek-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-3);
    border-top: 1px solid var(--card-border);
    background: var(--card-bg);
  }

  .status-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .act-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.35rem 0.65rem;
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    font-weight: 500;
    cursor: pointer;
    background: var(--surface);
    border: 1px solid var(--card-border);
    color: var(--text-primary);
  }

  .act-btn:hover:not(:disabled) {
    background: var(--nav-hover);
  }

  .act-btn.success {
    background: var(--success-soft);
    border-color: var(--success);
    color: var(--success);
  }

  .act-btn.danger {
    background: var(--danger-soft);
    border-color: var(--danger);
    color: var(--danger);
  }

  .act-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .external-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    border: 1px solid var(--card-border);
    background: var(--surface);
  }

  .external-btn:hover {
    color: var(--text-primary);
    background: var(--nav-hover);
  }
</style>
