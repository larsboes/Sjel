<script lang="ts">
  import Icon from '$lib/Icon.svelte';
  import { link } from '$lib/nav';
  import type { TelemetryPulseWidgetData } from '../types';

  let { card }: { card: TelemetryPulseWidgetData } = $props();
</script>

<div class="telemetry-widget">
  <div class="widget-header">
    <div class="header-title">
      <span class="pulse-indicator" class:warn={!card.overallOk}></span>
      <Icon name="cpu" size={14} />
      <strong>{card.title}</strong>
    </div>
    {#if card.capabilitiesSummary}
      <span class="caps-badge">
        <Icon name="server" size={11} />
        <span class="mono">{card.capabilitiesSummary.up}/{card.capabilitiesSummary.total} up</span>
      </span>
    {/if}
  </div>

  {#if card.subtitle}
    <p class="widget-sub">{card.subtitle}</p>
  {/if}

  <div class="metrics-grid">
    {#each card.metrics as metric}
      <div class="metric-card">
        <span class="metric-label">{metric.label}</span>
        <div class="metric-value-row">
          <span class="metric-val mono" class:tone-warn={metric.tone === 'warn'} class:tone-alarm={metric.tone === 'alarm'} class:tone-good={metric.tone === 'good'}>
            {metric.value}
          </span>
          {#if metric.subvalue}
            <span class="metric-sub mono">{metric.subvalue}</span>
          {/if}
        </div>
        {#if metric.percent !== undefined}
          <div class="meter-track">
            <div
              class="meter-fill"
              class:fill-good={metric.tone === 'good'}
              class:fill-warn={metric.tone === 'warn'}
              class:fill-alarm={metric.tone === 'alarm'}
              style={`transform: scaleX(${Math.min(100, Math.max(0, metric.percent)) / 100})`}
            ></div>
          </div>
        {/if}
      </div>
    {/each}
  </div>

  {#if card.actions && card.actions.length > 0}
    <div class="widget-actions">
      {#each card.actions as action}
        {#if action.route}
          <a class="action-chip" href={link(action.route)}>
            {#if action.icon}
              <Icon name={action.icon as any} size={12} />
            {/if}
            <span>{action.label}</span>
            <Icon name="arrow-right" size={11} />
          </a>
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .telemetry-widget {
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

  .pulse-indicator {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background-color: var(--success);
    box-shadow: 0 0 6px var(--success);
  }

  .pulse-indicator.warn {
    background-color: var(--warning);
    box-shadow: 0 0 6px var(--warning);
  }

  .caps-badge {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.1rem 0.45rem;
    border-radius: var(--radius-sm);
    font-size: var(--text-2xs);
    font-weight: 600;
    color: var(--text-secondary);
    background-color: var(--surface);
  }

  .widget-sub {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .metrics-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-2);
  }

  .metric-card {
    background-color: var(--surface);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }

  .metric-label {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.03em;
    font-weight: 500;
  }

  .metric-value-row {
    display: flex;
    align-items: baseline;
    gap: 0.35rem;
  }

  .metric-val {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-primary);
  }

  .metric-val.tone-good {
    color: var(--success);
  }

  .metric-val.tone-warn {
    color: var(--warning-ink);
  }

  .metric-val.tone-alarm {
    color: var(--danger);
  }

  .metric-sub {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .meter-track {
    width: 100%;
    height: 4px;
    border-radius: 9999px;
    background-color: var(--card-border);
    overflow: hidden;
    margin-top: 0.2rem;
  }

  .meter-fill {
    height: 100%;
    background-color: var(--primary);
    transform-origin: left;
    transition: transform var(--motion-slow) var(--ease-out);
  }

  .meter-fill.fill-good {
    background-color: var(--success);
  }

  .meter-fill.fill-warn {
    background-color: var(--warning);
  }

  .meter-fill.fill-alarm {
    background-color: var(--danger);
  }

  .widget-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-top: 0.2rem;
  }

  .action-chip {
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
    transition: background-color var(--motion-fast) ease, border-color var(--motion-fast) ease, color var(--motion-fast) ease;
  }

  .action-chip:hover {
    background-color: var(--surface);
    border-color: var(--primary);
    color: var(--primary);
  }
</style>
