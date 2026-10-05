<script lang="ts">
  import Icon from '$lib/Icon.svelte';
  import { link } from '$lib/nav';
  import type { SpatialSummaryWidgetData } from '../types';

  let { card }: { card: SpatialSummaryWidgetData } = $props();
</script>

<div class="spatial-widget">
  <div class="widget-header">
    <div class="header-title">
      <Icon name="layout" size={14} />
      <strong>{card.title}</strong>
    </div>
    <div class="counts-badge mono">
      <span>{card.roomsCount} room{card.roomsCount === 1 ? '' : 's'}</span>
      <span class="sep">·</span>
      <span>{card.furnitureCount} item{card.furnitureCount === 1 ? '' : 's'}</span>
    </div>
  </div>

  {#if card.rooms.length > 0}
    <div class="rooms-grid">
      {#each card.rooms as room}
        <div class="room-chip">
          <span class="room-name">{room.name}</span>
          <div class="room-stats mono">
            {#if room.areaSqMeters !== undefined}
              <span class="room-area">{room.areaSqMeters.toFixed(1)} m²</span>
            {/if}
            {#if room.objectsCount !== undefined}
              <span class="room-obj">{room.objectsCount} items</span>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}

  <div class="widget-footer">
    <a class="view-3d-btn" href={link('/interior')}>
      <Icon name="boxes" size={13} />
      <span>Open 3D Interior</span>
      <Icon name="arrow-right" size={11} />
    </a>
  </div>
</div>

<style>
  .spatial-widget {
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

  .counts-badge {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: var(--text-2xs);
    color: var(--text-secondary);
    background-color: var(--surface);
    padding: 0.1rem 0.45rem;
    border-radius: var(--radius-sm);
  }

  .sep {
    color: var(--text-tertiary);
  }

  .rooms-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(130px, 1fr));
    gap: var(--space-2);
  }

  .room-chip {
    background-color: var(--surface);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }

  .room-name {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-primary);
  }

  .room-stats {
    display: flex;
    gap: 0.4rem;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .widget-footer {
    display: flex;
    justify-content: flex-end;
  }

  .view-3d-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--primary);
    text-decoration: none;
    padding: 0.3rem 0.6rem;
    border-radius: var(--radius-sm);
    background-color: var(--primary-soft);
    transition: transform var(--motion-fast) ease, background-color var(--motion-fast) ease;
  }

  .view-3d-btn:hover {
    background-color: color-mix(in srgb, var(--primary) 15%, transparent);
  }
</style>
