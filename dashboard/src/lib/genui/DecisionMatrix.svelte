<script lang="ts">
  import DataTable, { type Column } from "$lib/ui/DataTable.svelte";
  import type { DecisionMatrixSpec } from "./types";

  let { spec }: { spec: DecisionMatrixSpec } = $props();

  type Scored = DecisionMatrixSpec["options"][number] & { score: number; percentage: number };

  // svelte-ignore state_referenced_locally
  let weights = $state<Record<string, number>>(
    Object.fromEntries(spec.criteria.map((c) => [c.id, c.weight || 3])),
  );

  // svelte-ignore state_referenced_locally
  let optionScores = $state<Record<string, Record<string, number>>>(
    Object.fromEntries(spec.options.map((o) => [o.id, { ...o.scores }])),
  );

  // Computed ranking and normalized weighted scores
  const results = $derived.by(() => {
    const totalWeight = Object.values(weights).reduce((a, b) => a + b, 0) || 1;
    const computed = spec.options.map((opt) => {
      const scores = optionScores[opt.id] ?? opt.scores;
      let rawWeighted = 0;
      for (const crit of spec.criteria) {
        const w = weights[crit.id] ?? 1;
        const s = scores[crit.id] ?? 5;
        rawWeighted += s * w;
      }
      const score = Math.round((rawWeighted / totalWeight) * 10) / 10;
      const percentage = Math.min(100, Math.round((score / 10) * 100));
      return {
        ...opt,
        score,
        percentage,
      };
    });

    return computed.toSorted((a, b) => b.score - a.score);
  });

  const bestOption = $derived(results[0]);

  // One column per criterion, because the criteria are the data. The weights live in the
  // toolbar above and change the last column, not the header, so a header cannot go stale.
  const columns: Column<Scored>[] = $derived([
    { id: "option", label: "Candidate option", wrap: true, cell: optionCell },
    ...spec.criteria.map((crit) => ({
      id: crit.id,
      label: crit.name,
      width: "5.5rem",
      align: "end" as const,
      cell: scoreCell,
    })),
    { id: "score", label: "Weighted score", width: "9rem", align: "end", cell: totalCell },
  ]);
</script>

{#snippet optionCell(opt: Scored)}
  <div class="opt-name-row">
    <span class="opt-name">{opt.name}</span>
    {#if opt.tag}
      <span class="opt-tag">{opt.tag}</span>
    {/if}
  </div>
  {#if opt.description}
    <span class="opt-desc">{opt.description}</span>
  {/if}
{/snippet}

{#snippet scoreCell(opt: Scored, column: Column<Scored>)}
  <input
    type="number"
    min="1"
    max="10"
    class="score-input"
    bind:value={optionScores[opt.id][column.id]}
    aria-label={`${opt.name} score for ${column.label}`}
  />
{/snippet}

{#snippet totalCell(opt: Scored)}
  <div class="score-display">
    <span class="score-num">{opt.score.toFixed(1)}</span>
    <div class="score-bar-track">
      <div class:best={opt.id === bestOption?.id} class="score-bar-fill" style="width: {opt.percentage}%;"></div>
    </div>
  </div>
{/snippet}

<div class="genui-card decision-matrix-container">
  <div class="matrix-header">
    <div class="header-left">
      <span class="genui-badge">GenUI · Deliberation</span>
      <h4 class="matrix-title">{spec.title ?? "Decision & Trade-Off Matrix"}</h4>
    </div>
    {#if spec.description}
      <p class="matrix-desc">{spec.description}</p>
    {/if}
  </div>

  <!-- Realtime Ranking Summary Banner -->
  <div class="ranking-banner">
    <span class="banner-label">Top Recommendation:</span>
    <span class="banner-winner">{bestOption?.name} ({bestOption?.score}/10)</span>
    {#if bestOption?.tag}
      <span class="winner-tag">{bestOption.tag}</span>
    {/if}
  </div>

  <!-- Interactive Criteria Weights Toolbar -->
  <div class="criteria-bar">
    <span class="criteria-bar-label">Adjust Criteria Weights:</span>
    <div class="criteria-sliders">
      {#each spec.criteria as crit (crit.id)}
        <label class="crit-control">
          <span class="crit-name">{crit.name} (w: {weights[crit.id]})</span>
          <input
            type="range"
            min="1"
            max="5"
            step="1"
            bind:value={weights[crit.id]}
            aria-label={`Weight for ${crit.name}`}
          />
        </label>
      {/each}
    </div>
  </div>

  <DataTable
    rows={results}
    {columns}
    key={(opt) => opt.id}
    rowHeader
    selected={bestOption?.id}
    caption="Scores run 1 to 10. The weighted score is the sum of score times weight over the total weight, and the highlighted row is the highest."
  />
</div>

<style>
  .genui-card {
    background: var(--surface, #ffffff);
    border: 1px solid var(--rule, #e2e8f0);
    border-radius: var(--radius-sm, 6px);
    padding: 0.9rem;
    margin: 1rem 0;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.04);
  }

  .matrix-header {
    margin-bottom: 0.8rem;
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .genui-badge {
    text-transform: uppercase;
    font-size: var(--text-2xs);
    font-weight: 700;
    background: var(--primary-soft, #eff6ff);
    color: var(--primary, #2563eb);
    padding: 0.1rem 0.35rem;
    border-radius: 3px;
    letter-spacing: 0.04em;
  }

  .matrix-title {
    margin: 0;
    font-size: var(--text-sm, 0.9rem);
    font-weight: 600;
    color: var(--text-primary, #0f172a);
  }

  .matrix-desc {
    margin: 0.3rem 0 0;
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-secondary, #64748b);
  }

  .ranking-banner {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    background: #ecfdf5;
    border: 1px solid #a7f3d0;
    padding: 0.4rem 0.65rem;
    border-radius: var(--radius-sm, 4px);
    margin-bottom: 0.8rem;
    font-size: var(--text-xs, 0.75rem);
  }

  .banner-label {
    color: #065f46;
    font-weight: 600;
  }

  .banner-winner {
    color: #047857;
    font-weight: 700;
  }

  .winner-tag {
    font-size: var(--text-2xs);
    background: #d1fae5;
    color: #065f46;
    padding: 0.05rem 0.3rem;
    border-radius: 3px;
  }

  .criteria-bar {
    background: var(--surface, #f8fafc);
    border: 1px solid var(--rule, #e2e8f0);
    border-radius: var(--radius-sm, 4px);
    padding: 0.5rem 0.75rem;
    margin-bottom: 0.8rem;
  }

  .criteria-bar-label {
    display: block;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-secondary, #64748b);
    margin-bottom: 0.4rem;
  }

  .criteria-sliders {
    display: flex;
    flex-wrap: wrap;
    gap: 0.8rem;
  }

  .crit-control {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: var(--text-2xs);
    color: var(--text-primary, #0f172a);
  }

  .crit-control input[type="range"] {
    width: 6.5rem;
    accent-color: var(--primary, #2563eb);
    cursor: pointer;
  }

  .opt-name-row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .opt-name {
    font-weight: 600;
    color: var(--text-primary, #0f172a);
  }

  .opt-tag {
    font-size: var(--text-2xs);
    background: var(--surface, #f1f5f9);
    border: 1px solid var(--rule, #cbd5e1);
    padding: 0.05rem 0.3rem;
    border-radius: 3px;
    color: var(--text-secondary, #475569);
  }

  .opt-desc {
    display: block;
    font-size: var(--text-2xs);
    color: var(--text-secondary, #64748b);
    margin-top: 0.15rem;
  }

  .score-input {
    width: 3rem;
    text-align: center;
    padding: 0.15rem 0.2rem;
    border: 1px solid var(--rule, #cbd5e1);
    border-radius: 3px;
    background: var(--surface, #ffffff);
    font: inherit;
    font-weight: 600;
  }

  .score-display {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .score-num {
    font-weight: 700;
    font-size: var(--text-sm);
    color: var(--text-primary, #0f172a);
    min-width: 2rem;
  }

  .score-bar-track {
    flex: 1;
    height: 6px;
    background: var(--rule, #e2e8f0);
    border-radius: 3px;
    overflow: hidden;
  }

  .score-bar-fill {
    height: 100%;
    background: var(--primary, #2563eb);
    border-radius: 3px;
    transition: width var(--motion-base) ease-out;
  }

  .score-bar-fill.best {
    background: #059669;
  }
</style>
