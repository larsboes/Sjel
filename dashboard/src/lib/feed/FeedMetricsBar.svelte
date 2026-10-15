<script lang="ts">
  import { tip } from "$lib/tip";
  import type { FeedEntry } from "$lib/api";
  import { FEED_KIND_LABEL } from "./FeedItemRow.svelte";

  let {
    entries,
  }: {
    entries: FeedEntry[];
  } = $props();

  const total = $derived(entries.length);
  const newCount = $derived(entries.filter((e) => e.status === "new").length);
  const keepers = $derived(entries.filter((e) => e.status === "keeper").length);
  const evaluatedCount = $derived(entries.filter((e) => e.evaluation != null).length);
  const travelCount = $derived(
    entries.filter((e) => {
      const travel = e.evaluation?.factors.find((f) => f.key === "travel");
      return travel && travel.score > 0;
    }).length,
  );

  const typeCounts = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const entry of entries) {
      counts.set(entry.kind, (counts.get(entry.kind) ?? 0) + 1);
    }
    return [...counts.entries()].sort((a, b) => b[1] - a[1]);
  });

  function share(count: number): string {
    return `${total === 0 ? 0 : (count / total) * 100}%`;
  }
</script>

<div class="metrics-bar">
  <div class="summary-stats">
    <div class="stat">
      <span class="stat-label">Total</span>
      <strong class="stat-value mono">{total}</strong>
    </div>
    <div class="stat">
      <span class="stat-label">New</span>
      <strong class="stat-value mono accent">{newCount}</strong>
    </div>
    <div class="stat">
      <span class="stat-label">Kept</span>
      <strong class="stat-value mono success">{keepers}</strong>
    </div>
    <div class="stat">
      <span class="stat-label">Ranked</span>
      <strong class="stat-value mono">{evaluatedCount}</strong>
    </div>
    <div class="stat">
      <span class="stat-label">Travel Fit</span>
      <strong class="stat-value mono">{travelCount}</strong>
    </div>
  </div>

  {#if typeCounts.length > 0}
    <div class="distribution" aria-label="Source distribution">
      <div class="bar" aria-hidden="true">
        {#each typeCounts as [entryKind, count] (entryKind)}
          <span
            class={`segment kind-${entryKind}`}
            style:width={share(count)}
            use:tip={`${FEED_KIND_LABEL[entryKind] ?? entryKind}: ${count} (${Math.round((count / total) * 100)}%)`}
          ></span>
        {/each}
      </div>
      <ul class="legend">
        {#each typeCounts as [entryKind, count] (entryKind)}
          <li>
            <span class={`dot kind-${entryKind}`}></span>
            <span class="kind-name">{FEED_KIND_LABEL[entryKind] ?? entryKind}</span>
            <span class="mono count">{count}</span>
          </li>
        {/each}
      </ul>
    </div>
  {/if}
</div>

<style>
  .metrics-bar {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3) 0;
    border-bottom: 1px solid var(--card-border);
  }

  .summary-stats {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    flex-wrap: wrap;
  }

  .stat {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    padding-right: var(--space-4);
    border-right: 1px solid var(--card-border);
  }

  .stat:last-child {
    border-right: none;
    padding-right: 0;
  }

  .stat-label {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .stat-value {
    font-size: var(--text-lg);
    line-height: 1;
    color: var(--text-primary);
  }

  .stat-value.accent {
    color: var(--accent);
  }

  .stat-value.success {
    color: var(--success);
  }

  .distribution {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .bar {
    display: flex;
    height: 4px;
    border-radius: var(--radius-full);
    overflow: hidden;
    background: var(--card-border);
  }

  .segment {
    height: 100%;
    min-width: 2px;
  }

  .legend {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .legend li {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .count {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .kind-youtube { background: var(--danger); }
  .kind-article { background: var(--accent); }
  .kind-podcast { background: var(--warning-ink); }
  .kind-github { background: var(--text-secondary); }
  .kind-mail { background: var(--success); }
  .kind-reddit { background: var(--accent); }
  .kind-arxiv { background: var(--text-tertiary); }
  .kind-instagram { background: var(--accent); }
</style>
