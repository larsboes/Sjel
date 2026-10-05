<script lang="ts">
  import Icon from '$lib/Icon.svelte';
  import type { ActionChoiceWidgetData } from '../types';

  let {
    card,
    onSelect,
  }: {
    card: ActionChoiceWidgetData;
    onSelect?: (prompt: string) => void;
  } = $props();
</script>

<div class="choice-widget">
  <div class="widget-header">
    <Icon name="sparkles" size={14} />
    <strong>{card.title}</strong>
  </div>

  {#if card.description}
    <p class="widget-desc">{card.description}</p>
  {/if}

  <div class="choices-list">
    {#each card.choices as choice}
      <button
        type="button"
        class="choice-btn"
        onclick={() => onSelect?.(choice.prompt)}
      >
        <div class="choice-icon-wrap">
          <Icon name={(choice.icon as any) || 'arrow-right'} size={14} />
        </div>
        <div class="choice-content">
          <span class="choice-label">{choice.label}</span>
          {#if choice.description}
            <span class="choice-desc">{choice.description}</span>
          {/if}
        </div>
      </button>
    {/each}
  </div>
</div>

<style>
  .choice-widget {
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

  .widget-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--primary);
  }

  .widget-header strong {
    font-weight: 600;
  }

  .widget-desc {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .choices-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .choice-btn {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    background-color: var(--surface);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    text-align: left;
    cursor: pointer;
    font: inherit;
    transition:
      background-color var(--motion-fast) ease,
      border-color var(--motion-fast) ease,
      transform var(--motion-fast) ease;
  }

  .choice-btn:hover {
    background-color: var(--card-bg);
    border-color: var(--primary);
    transform: translateX(2px);
  }

  .choice-icon-wrap {
    display: grid;
    place-items: center;
    width: 1.75rem;
    height: 1.75rem;
    border-radius: var(--radius-sm);
    background-color: var(--primary-soft);
    color: var(--primary);
    flex-shrink: 0;
  }

  .choice-content {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    min-width: 0;
  }

  .choice-label {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-primary);
  }

  .choice-desc {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }
</style>
