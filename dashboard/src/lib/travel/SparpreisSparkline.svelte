<script lang="ts">
  import { tip } from "$lib/tip";
  export interface Observation {
    day: string;
    prices: number[];
  }

  let {
    history = [],
    width = 100,
    height = 24,
  }: {
    history: Observation[];
    width?: number;
    height?: number;
  } = $props();

  const dailyMins = $derived(
    history
      .map((h) => ({
        day: h.day,
        price: h.prices.length ? Math.min(...h.prices) : null,
      }))
      .filter((d): d is { day: string; price: number } => d.price !== null)
  );

  const minPrice = $derived(dailyMins.length ? Math.min(...dailyMins.map((d) => d.price)) : null);
  const maxPrice = $derived(dailyMins.length ? Math.max(...dailyMins.map((d) => d.price)) : null);
  const currentPrice = $derived(dailyMins.length ? dailyMins[dailyMins.length - 1].price : null);
  const startPrice = $derived(dailyMins.length ? dailyMins[0].price : null);
  const dropped = $derived(
    currentPrice !== null && startPrice !== null && currentPrice < startPrice
  );
  const diff = $derived(
    currentPrice !== null && startPrice !== null ? currentPrice - startPrice : 0
  );

  const points = $derived.by(() => {
    if (dailyMins.length < 2 || minPrice === null || maxPrice === null) return '';
    const span = maxPrice - minPrice || 1;
    const padding = 3;
    const effH = height - padding * 2;
    const effW = width - padding * 2;
    const stepX = effW / (dailyMins.length - 1);

    return dailyMins
      .map((d, i) => {
        const x = padding + i * stepX;
        // Invert Y so lower price is lower on screen or higher? Lower price is cheaper/better, but on graph standard is lower y = lower value
        const y = padding + effH - ((d.price - minPrice) / span) * effH;
        return `${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(' ');
  });

  const lastPoint = $derived.by(() => {
    if (dailyMins.length === 0 || minPrice === null || maxPrice === null) return null;
    const span = maxPrice - minPrice || 1;
    const padding = 3;
    const effH = height - padding * 2;
    const effW = width - padding * 2;
    const stepX = dailyMins.length > 1 ? effW / (dailyMins.length - 1) : effW / 2;
    const i = dailyMins.length - 1;
    const x = padding + i * stepX;
    const y = padding + effH - ((dailyMins[i].price - minPrice) / span) * effH;
    return { x, y };
  });
</script>

{#if dailyMins.length >= 2}
  <div class="sparkline-wrap" use:tip={`Price history: ${dailyMins.length} checks, low €${minPrice?.toFixed(2)}, current €${currentPrice?.toFixed(2)}`}>
    <svg {width} {height} viewBox={`0 0 ${width} ${height}`} class="sparkline-svg">
      <polyline
        fill="none"
        stroke={dropped ? 'var(--success)' : 'var(--text-tertiary)'}
        stroke-width="1.75"
        stroke-linecap="round"
        stroke-linejoin="round"
        {points}
      />
      {#if lastPoint}
        <circle
          cx={lastPoint.x}
          cy={lastPoint.y}
          r="2.5"
          fill={dropped ? 'var(--success)' : 'var(--primary)'}
        />
      {/if}
    </svg>
    {#if diff !== 0}
      <span class="diff-badge mono" class:diff-drop={dropped}>
        {diff < 0 ? `−€${Math.abs(diff).toFixed(0)}` : `+€${diff.toFixed(0)}`}
      </span>
    {/if}
  </div>
{/if}

<style>
  .sparkline-wrap {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    vertical-align: middle;
  }

  .sparkline-svg {
    display: block;
    overflow: visible;
  }

  .diff-badge {
    font-size: var(--text-2xs);
    font-weight: 600;
    color: var(--text-tertiary);
  }

  .diff-badge.diff-drop {
    color: var(--success);
  }
</style>
