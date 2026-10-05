<script lang="ts">
  import Icon from '$lib/Icon.svelte';
  import { link } from '$lib/nav';
  import type { FeedDigestWidgetData } from '../types';

  let { card }: { card: FeedDigestWidgetData } = $props();
</script>

<div class="feed-digest-widget">
  <div class="widget-header">
    <div class="header-title">
      <Icon name="feed" size={14} />
      <strong>{card.title}</strong>
    </div>
    {#if card.moreCount && card.moreCount > 0}
      <span class="more-badge mono">+{card.moreCount} more</span>
    {/if}
  </div>

  <div class="items-list">
    {#each card.items as item}
      <a class="item-card" href={item.url} target="_blank" rel="noopener noreferrer">
        <div class="item-meta">
          {#if item.domain}
            <span class="domain-tag">{item.domain}</span>
          {/if}
          {#if item.author}
            <span class="author-name">{item.author}</span>
          {/if}
          {#if item.age}
            <span class="age-label mono">{item.age}</span>
          {/if}
        </div>
        <div class="item-title-row">
          <span class="item-title">{item.title}</span>
          <Icon name="external" size={11} />
        </div>
      </a>
    {/each}
  </div>

  <div class="widget-footer">
    <a class="view-feed-btn" href={link('/feed')}>
      <Icon name="feed" size={12} />
      <span>Open full Feed</span>
      <Icon name="arrow-right" size={11} />
    </a>
  </div>
</div>

<style>
  .feed-digest-widget {
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
    justify-content: space-between;
    align-items: center;
  }

  .header-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-primary);
  }

  .header-title strong {
    font-weight: 600;
  }

  .more-badge {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    background-color: var(--surface);
    padding: 0.1rem 0.45rem;
    border-radius: var(--radius-sm);
  }

  .items-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .item-card {
    background-color: var(--surface);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    text-decoration: none;
    transition: background-color var(--motion-fast) ease, border-color var(--motion-fast) ease;
  }

  .item-card:hover {
    background-color: var(--card-bg);
    border-color: var(--card-border-hover);
  }

  .item-meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .domain-tag {
    font-weight: 600;
    color: var(--primary);
  }

  .author-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 140px;
  }

  .age-label {
    margin-left: auto;
  }

  .item-title-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    color: var(--text-primary);
  }

  .item-title {
    font-size: var(--text-xs);
    font-weight: 550;
    line-height: var(--leading-tight);
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  .item-card:hover .item-title {
    color: var(--primary);
  }

  .widget-footer {
    display: flex;
    justify-content: flex-end;
  }

  .view-feed-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--primary);
    text-decoration: none;
    padding: 0.25rem 0.5rem;
    border-radius: var(--radius-sm);
    transition: background-color var(--motion-fast) ease;
  }

  .view-feed-btn:hover {
    background-color: var(--primary-soft);
  }
</style>
