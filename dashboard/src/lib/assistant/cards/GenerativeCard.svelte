<script lang="ts">
  import Icon from '$lib/Icon.svelte';
  import { link } from '$lib/nav';
  import type { GenerativeCustomCardData } from '../types';

  let {
    card,
    onActionPrompt,
  }: {
    card: GenerativeCustomCardData;
    onActionPrompt?: (prompt: string) => void;
  } = $props();
</script>

<div class="gen-card" class:tone-good={card.tone === 'good'} class:tone-warn={card.tone === 'warn'} class:tone-alarm={card.tone === 'alarm'} class:tone-primary={card.tone === 'primary'}>
  <div class="gen-header">
    <div class="gen-header-text">
      {#if card.kicker}
        <span class="gen-kicker">{card.kicker}</span>
      {/if}
      <strong class="gen-title">{card.title}</strong>
      {#if card.subtitle}
        <span class="gen-subtitle">{card.subtitle}</span>
      {/if}
    </div>
    {#if card.tone}
      <span class="tone-pill" class:tone-good={card.tone === 'good'} class:tone-warn={card.tone === 'warn'} class:tone-alarm={card.tone === 'alarm'} class:tone-primary={card.tone === 'primary'}></span>
    {/if}
  </div>

  {#if card.chips && card.chips.length > 0}
    <div class="gen-chips">
      {#each card.chips as chip}
        <span class="gen-chip">{chip}</span>
      {/each}
    </div>
  {/if}

  {#if card.metrics && card.metrics.length > 0}
    <div class="gen-metrics-grid">
      {#each card.metrics as m}
        <div class="gen-metric">
          <span class="metric-label">{m.label}</span>
          <span class="metric-value mono" class:val-good={m.tone === 'good'} class:val-warn={m.tone === 'warn'} class:val-alarm={m.tone === 'alarm'}>
            {m.value}
            {#if m.change}
              <small class="metric-change">{m.change}</small>
            {/if}
          </span>
        </div>
      {/each}
    </div>
  {/if}

  {#if card.actions && card.actions.length > 0}
    <div class="gen-actions">
      {#each card.actions as action}
        {#if action.route}
          <a class="gen-action-btn" href={link(action.route)}>
            {#if action.icon}
              <Icon name={action.icon as any} size={12} />
            {/if}
            <span>{action.label}</span>
            <Icon name="arrow-right" size={11} />
          </a>
        {:else if action.prompt}
          <button
            type="button"
            class="gen-action-btn prompt-btn"
            onclick={() => onActionPrompt?.(action.prompt!)}
          >
            {#if action.icon}
              <Icon name={action.icon as any} size={12} />
            {/if}
            <span>{action.label}</span>
          </button>
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .gen-card {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    padding: var(--space-4);
    box-shadow: var(--card-shadow);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-top: var(--space-2);
  }

  .gen-card.tone-primary {
    border-left: 3px solid var(--primary);
  }

  .gen-card.tone-good {
    border-left: 3px solid var(--success);
  }

  .gen-card.tone-warn {
    border-left: 3px solid var(--warning);
  }

  .gen-card.tone-alarm {
    border-left: 3px solid var(--danger);
  }

  .gen-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: var(--space-2);
  }

  .gen-header-text {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  .gen-kicker {
    font-size: var(--text-2xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-tertiary);
    font-weight: 600;
  }

  .gen-title {
    font-size: var(--text-sm);
    font-weight: 650;
    color: var(--text-primary);
    line-height: var(--leading-tight);
  }

  .gen-subtitle {
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .tone-pill {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background-color: var(--text-tertiary);
    flex-shrink: 0;
    margin-top: 0.25rem;
  }

  .tone-pill.tone-good { background-color: var(--success); box-shadow: 0 0 6px var(--success); }
  .tone-pill.tone-warn { background-color: var(--warning); box-shadow: 0 0 6px var(--warning); }
  .tone-pill.tone-alarm { background-color: var(--danger); box-shadow: 0 0 6px var(--danger); }
  .tone-pill.tone-primary { background-color: var(--primary); box-shadow: 0 0 6px var(--primary); }

  .gen-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .gen-chip {
    padding: 0.1rem 0.45rem;
    border-radius: var(--radius-sm);
    background-color: var(--surface);
    font-size: var(--text-2xs);
    color: var(--text-secondary);
    font-weight: 500;
  }

  .gen-metrics-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(110px, 1fr));
    gap: var(--space-2);
  }

  .gen-metric {
    background-color: var(--surface);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  .metric-label {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    text-transform: uppercase;
    font-weight: 500;
  }

  .metric-value {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-primary);
  }

  .metric-value.val-good { color: var(--success); }
  .metric-value.val-warn { color: var(--warning-ink); }
  .metric-value.val-alarm { color: var(--danger); }

  .metric-change {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    margin-left: 0.25rem;
  }

  .gen-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-top: 0.2rem;
  }

  .gen-action-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.3rem 0.65rem;
    border-radius: var(--radius-sm);
    border: 1px solid var(--card-border);
    background-color: var(--card-bg);
    color: var(--text-primary);
    font-size: var(--text-xs);
    font-weight: 600;
    text-decoration: none;
    cursor: pointer;
    transition: background-color var(--motion-fast) ease, border-color var(--motion-fast) ease, color var(--motion-fast) ease;
  }

  .gen-action-btn:hover {
    background-color: var(--surface);
    border-color: var(--primary);
    color: var(--primary);
  }

  .prompt-btn {
    background-color: var(--primary-soft);
    border-color: transparent;
    color: var(--primary);
  }

  .prompt-btn:hover {
    background-color: color-mix(in srgb, var(--primary) 15%, transparent);
    border-color: var(--primary);
  }
</style>
