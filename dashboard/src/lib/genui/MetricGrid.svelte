<script lang="ts">
  import type { MetricGridSpec } from "./types";

  let { spec }: { spec: MetricGridSpec } = $props();

  const colsClass = $derived(
    spec.columns === 2 ? "cols-2" : spec.columns === 4 ? "cols-4" : "cols-3",
  );
</script>

<div class="genui-card metric-grid-container">
  {#if spec.title}
    <div class="metric-grid-header">
      <div class="header-left">
        <span class="genui-badge">GenUI · Metrics</span>
        <h4 class="metric-grid-title">{spec.title}</h4>
      </div>
      {#if spec.subtitle}
        <span class="metric-grid-sub">{spec.subtitle}</span>
      {/if}
    </div>
  {/if}

  <div class="metrics-grid {colsClass}">
    {#each spec.items as item, idx (idx)}
      <div class="metric-tile" class:status-good={item.status === "good" || item.status === "nominal"} class:status-warn={item.status === "warning"} class:status-crit={item.status === "critical"} class:status-idle={item.status === "idle"}>
        <div class="tile-top">
          <span class="metric-label">{item.label}</span>
          {#if item.status}
            <span class="status-indicator {item.status}">{item.status}</span>
          {/if}
        </div>
        <div class="tile-value-row">
          <span class="metric-value">{item.value}</span>
          {#if item.unit}
            <span class="metric-unit">{item.unit}</span>
          {/if}
        </div>
        {#if item.delta || item.hint}
          <div class="tile-bottom">
            {#if item.delta}
              <span class="metric-delta">{item.delta}</span>
            {/if}
            {#if item.hint}
              <span class="metric-hint">{item.hint}</span>
            {/if}
          </div>
        {/if}
      </div>
    {/each}
  </div>
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

  .metric-grid-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-bottom: 0.8rem;
    padding-bottom: 0.4rem;
    border-bottom: 1px solid var(--rule, #e2e8f0);
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

  .metric-grid-title {
    margin: 0;
    font-size: var(--text-sm, 0.9rem);
    font-weight: 600;
    color: var(--text-primary, #0f172a);
  }

  .metric-grid-sub {
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-secondary, #64748b);
  }

  .metrics-grid {
    display: grid;
    gap: 0.6rem;
  }

  .cols-2 {
    grid-template-columns: repeat(auto-fit, minmax(13rem, 1fr));
  }

  .cols-3 {
    grid-template-columns: repeat(auto-fit, minmax(10.5rem, 1fr));
  }

  .cols-4 {
    grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr));
  }

  .metric-tile {
    background: var(--surface, #f8fafc);
    border: 1px solid var(--rule, #e2e8f0);
    border-radius: var(--radius-sm, 5px);
    padding: 0.65rem 0.75rem;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    transition: border-color var(--motion-fast) ease, background-color var(--motion-fast) ease;
  }

  .metric-tile:hover {
    background: var(--surface, #ffffff);
    border-color: var(--primary, #2563eb);
  }

  .tile-top {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.4rem;
    margin-bottom: 0.25rem;
  }

  .metric-label {
    font-size: var(--text-xs);
    color: var(--text-secondary, #64748b);
    font-weight: 500;
  }

  .status-indicator {
    font-size: var(--text-2xs);
    text-transform: uppercase;
    font-weight: 700;
    padding: 0.05rem 0.3rem;
    border-radius: 3px;
  }

  .status-indicator.good,
  .status-indicator.nominal {
    background: #ecfdf5;
    color: #047857;
    border: 1px solid #a7f3d0;
  }

  .status-indicator.warning {
    background: #fffbeb;
    color: #b45309;
    border: 1px solid #fde68a;
  }

  .status-indicator.critical {
    background: #fef2f2;
    color: #b91c1c;
    border: 1px solid #fecaca;
  }

  .status-indicator.idle {
    background: #f1f5f9;
    color: #475569;
    border: 1px solid #cbd5e1;
  }

  .status-indicator.active {
    background: #eff6ff;
    color: #1d4ed8;
    border: 1px solid #bfdbfe;
  }

  .tile-value-row {
    display: flex;
    align-items: baseline;
    gap: 0.3rem;
    margin: 0.15rem 0;
  }

  .metric-value {
    font-size: var(--text-md);
    font-weight: 700;
    color: var(--text-primary, #0f172a);
    letter-spacing: -0.02em;
  }

  .metric-unit {
    font-size: var(--text-xs);
    color: var(--text-secondary, #64748b);
    font-weight: 500;
  }

  .tile-bottom {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.3rem;
    font-size: var(--text-2xs);
    margin-top: 0.25rem;
  }

  .metric-delta {
    font-weight: 600;
    color: var(--primary, #2563eb);
  }

  .metric-hint {
    color: var(--text-secondary, #64748b);
  }
</style>
