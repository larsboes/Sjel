<script lang="ts" module>
  export interface Factor {
    key: string;
    label: string;
    /** 0..1. */
    score: number;
    /** 0..1, the factor's share of the overall score. */
    weight: number;
    /** An Icon name rendered beside the label, where the factor's meaning is not in
     *  its words. Carries what a second hue used to, and survives a reader who cannot
     *  separate two hues. */
    mark?: string;
    rationale?: string;
    context?: { label: string; href?: string; terms?: string[] } | null;
  }
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import { tip } from "$lib/tip";
  import Icon from "./Icon.svelte";

  /**
   * A row of meters: how much each factor contributed to one score.
   *
   * Every bar is the SAME hue. A meter measures magnitude, and magnitude is a sequential
   * encoding — one hue, light track to dark fill. The previous version painted the travel
   * factor with `--success`, which is a reserved status colour used as series identity;
   * the factor's meaning now rides a glyph and its `aria-label`, which also works on the
   * compact card where the context link does not render at all.
   *
   * The grid is factor-count agnostic. A hardcoded four-column rule against a generic
   * loop produces a ragged second row the moment a fifth factor ships.
   */
  let {
    factors,
    compact = false,
    weighted = false,
    detail,
  }: {
    factors: Factor[];
    /** The card form: labels and values only, no rationale, no context link. */
    compact?: boolean;
    /** Show each factor's weight beside its score. */
    weighted?: boolean;
    detail?: Snippet<[Factor]>;
  } = $props();

  const percent = (value: number) => Math.round(Math.min(1, Math.max(0, value)) * 100);

  const describe = (factor: Factor): string => {
    const parts = [`${factor.label}: ${percent(factor.score)} of 100`];
    if (weighted) parts.push(`weight ${percent(factor.weight)}%`);
    if (factor.context?.label) parts.push(factor.context.label);
    return parts.join(", ");
  };
</script>

<div class="factors" class:compact>
  {#each factors as factor (factor.key)}
    <div class="factor" use:tip={factor.rationale ?? describe(factor)}>
      <div class="label">
        <span class="name">
          {#if factor.mark}<Icon name={factor.mark as never} size={11} />{/if}
          {factor.label}
        </span>
        <!-- The score alone. `100/20` for "scored 100, weighted 20%" reads as "100 out of
             20", which is the opposite of a number that helps; the weight goes in the
             expanded rationale and in the bar's own aria-label. -->
        <span class="value mono">{percent(factor.score)}</span>
      </div>

      <div
        class="track"
        role="meter"
        aria-valuenow={percent(factor.score)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={describe(factor)}
      >
        <span class="fill" style:width={`${percent(factor.score)}%`}></span>
      </div>

      {#if !compact}
        {#if detail}
          {@render detail(factor)}
        {:else}
          {#if factor.rationale}<p class="rationale">{factor.rationale}</p>{/if}
          {#if factor.context}
            <span class="context">
              {#if factor.mark}<Icon name={factor.mark as never} size={10} />{/if}
              {factor.context.label}
              {#if factor.context.terms?.length}· {factor.context.terms.join(", ")}{/if}
            </span>
          {/if}
        {/if}
      {/if}
    </div>
  {/each}
</div>

<style>
  .factors {
    display: grid;
    gap: var(--space-4);
    min-width: 0;
  }

  /* auto-fit, not a fixed column count: four was hardcoded against a loop over however
     many factors the evaluator published, so a fifth wrapped into a ragged row. */
  /* 6.5rem, not 4.75: at the smaller step "Travel relevance" and "Content evidence"
     both truncated to a word and a half on every card, which names nothing. */
  .compact {
    grid-template-columns: repeat(auto-fit, minmax(6.5rem, 1fr));
    gap: var(--space-3) var(--space-4);
  }

  .factor {
    min-width: 0;
  }

  .label {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: var(--space-2);
    margin-bottom: var(--space-1);
    color: var(--text-secondary);
    font-size: var(--text-2xs);
  }

  .name {
    display: inline-flex;
    align-items: center;
    gap: 0.25em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* The number wears a text token, never the mark's colour. */
  .value {
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  /* The unfilled track is a tinted step of the same hue, so the state reads across the
     whole bar rather than only where the fill reaches. */
  .track {
    height: 0.25rem;
    overflow: hidden;
    border-radius: 2px;
    background-color: var(--primary-soft);
  }

  .fill {
    display: block;
    height: 100%;
    /* Anchored to the baseline at the left, rounded only at the data end. */
    border-radius: 0 2px 2px 0;
    background-color: var(--primary);
  }

  .rationale {
    margin: var(--space-1) 0 0;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    line-height: var(--leading-normal);
  }

  .context {
    display: inline-flex;
    align-items: center;
    gap: 0.25em;
    margin-top: var(--space-1);
    color: var(--text-secondary);
    font-size: var(--text-2xs);
  }

  .compact .label {
    gap: var(--space-1);
    font-size: 0.625rem;
  }
</style>
