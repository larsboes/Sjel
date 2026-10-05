<script lang="ts">
  import { tip } from "$lib/tip";
  import { link } from "$lib/nav";
  import type { FeedEvaluation } from "$lib/api";

  let {
    evaluation,
    compact = false,
  }: {
    evaluation: FeedEvaluation;
    compact?: boolean;
  } = $props();

  const percentage = $derived(Math.round(evaluation.overall_score * 100));
  const modeLabel = $derived(
    evaluation.mode === "reranked"
      ? "reranked"
      : evaluation.mode === "semantic"
        ? "semantic"
      : evaluation.mode === "lexical"
        ? "lexical"
        : "without TELOS",
  );
</script>

<div class="evaluation" class:compact>
  <div class="evaluation-head">
    <div>
      <span class="score mono">{percentage}</span>
      <span class="unit mono">/100</span>
    </div>
    <div class="evaluation-title">
      <strong>Explained ranking</strong>
      <span>{modeLabel} · {evaluation.evaluator_revision}</span>
    </div>
  </div>

  <div class="factors">
    {#each evaluation.factors as factor (factor.key)}
      <div class:travel={factor.key === "travel" && factor.score > 0} class="factor" use:tip={factor.rationale}>
        <div class="factor-label">
          <span>{factor.label}</span>
          <span class="mono">{Math.round(factor.score * 100)}</span>
        </div>
        <div class="track" aria-hidden="true">
          <span style:width={`${Math.round(factor.score * 100)}%`}></span>
        </div>
        {#if compact}
          <!-- The weight, in compact mode too. A bar whose share of the score is
               invisible cannot be argued with, and the share is now the thing
               that moves: a refused or inert factor sits at zero. -->
          <span class="compact-weight mono">{Math.round(factor.weight * 100)}%</span>
        {:else}
          <p>{factor.rationale} · Weight {Math.round(factor.weight * 100)}%</p>
          {#if factor.context?.kind === "trip"}
            <a class="factor-context" href={link("/travel")}>
              {factor.context.label}
              {#if factor.context.matched_terms.length > 0}
                · {factor.context.matched_terms.join(", ")}
              {/if}
            </a>
          {:else if factor.context?.kind === "urgency"}
            <!-- The model rung's own judgement, not a link: nothing else
                 renders one mail's urgency. -->
            <span class="factor-context">{factor.context.label}</span>
          {/if}
        {/if}
      </div>
    {/each}
  </div>

  {#if !compact}
    <p class="explanation">{evaluation.explanation}</p>
  {/if}
</div>

<style>
  .evaluation {
    min-width: 0;
  }

  .evaluation-head {
    display: flex;
    align-items: baseline;
    gap: 0.65rem;
    margin-bottom: 0.8rem;
  }

  .score {
    color: var(--primary);
    font-size: 1.35rem;
    font-weight: 620;
  }

  .unit {
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .evaluation-title {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 0.08rem;
  }

  .evaluation-title strong {
    color: var(--text-primary);
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .evaluation-title span {
    overflow: hidden;
    color: var(--text-tertiary);
    font-size: 0.5625rem;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .factors {
    display: grid;
    gap: 0.65rem;
  }

  .factor-label {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    margin-bottom: 0.25rem;
    color: var(--text-secondary);
    font-size: 0.625rem;
  }

  .factor-label span:last-child {
    color: var(--text-tertiary);
  }

  .track {
    height: 0.25rem;
    overflow: hidden;
    background: var(--surface);
  }

  .track span {
    display: block;
    height: 100%;
    background: var(--primary);
  }

  .factor.travel .track span {
    background: var(--success);
  }

  .factor-context {
    display: inline-block;
    margin-top: 0.25rem;
    color: var(--success);
    font-size: 0.625rem;
  }

  .factor p,
  .explanation {
    margin: 0.3rem 0 0;
    color: var(--text-tertiary);
    font-size: 0.625rem;
    line-height: 1.45;
  }

  .explanation {
    margin-top: 0.8rem;
    padding-top: 0.7rem;
    border-top: 1px solid var(--card-border);
    color: var(--text-secondary);
  }

  .compact .evaluation-head {
    margin-bottom: 0.55rem;
  }

  .compact .score {
    font-size: 0.875rem;
  }

  .compact .evaluation-title strong {
    font-size: 0.625rem;
  }

  .compact .evaluation-title span,
  .compact .unit {
    font-size: 0.5rem;
  }

  /* Factor-count agnostic. `repeat(4, …)` against a loop that renders however
     many factors the evaluator produced made a fifth factor wrap ragged the
     moment one arrived. */
  .compact .factors {
    grid-template-columns: repeat(auto-fit, minmax(4.5rem, 1fr));
    gap: 0.4rem;
  }

  .compact-weight {
    display: block;
    margin-top: 0.15rem;
    font-size: 0.5rem;
    color: var(--text-tertiary);
  }

  .compact .factor-label {
    gap: 0.25rem;
    font-size: 0.5rem;
  }

  .compact .factor-label span:first-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
