<script lang="ts">
  import { tip } from "$lib/tip";
  import type { FinanceDashboard } from "$lib/api";

  let { data }: { data: FinanceDashboard } = $props();

  const palette = ["#4f7fa8", "#c58b32", "#a65f4c", "#708273", "#806b91"];
  const months = $derived(data.trend.map((point) => point.month).slice(-12));
  const categorySeries = $derived.by(() => {
    const totals = new Map<string, number>();
    for (const point of data.category_trend) {
      totals.set(point.category, (totals.get(point.category) ?? 0) + point.amount_cents);
    }
    const ranked = [...totals.entries()].sort((left, right) => right[1] - left[1]);
    const leaders = ranked.slice(0, 5).map(([category, total], index) => ({
      id: category,
      label: short(category),
      color: category.includes("uncategorized") ? "#6f6f6f" : palette[index],
      total,
      values: months.map((month) => data.category_trend
        .filter((point) => point.month === month && point.category === category)
        .reduce((sum, point) => sum + point.amount_cents, 0)),
    }));
    const remainder = ranked.slice(5);
    if (remainder.length) {
      const remainderIds = new Set(remainder.map(([category]) => category));
      leaders.push({
        id: "other",
        label: "Other",
        color: "#b5b5b5",
        total: remainder.reduce((sum, [, value]) => sum + value, 0),
        values: months.map((month) => data.category_trend
          .filter((point) => point.month === month && remainderIds.has(point.category))
          .reduce((sum, point) => sum + point.amount_cents, 0)),
      });
    }
    return leaders;
  });
  const monthTotals = $derived(months.map((_, index) => categorySeries.reduce((sum, item) => sum + item.values[index], 0)));
  const maximumMonthTotal = $derived(Math.max(1, ...monthTotals));
  const smallMultiples = $derived(categorySeries.filter((item) => item.id !== "other" && !item.id.includes("uncategorized")).slice(0, 4));

  function short(account: string) {
    return account.replace(/^expenses:/, "").split(":").join(" · ");
  }

  function money(cents: number) {
    return new Intl.NumberFormat("de-DE", { style: "currency", currency: data.summary.currency }).format(cents / 100);
  }

  function sparkPoints(values: number[]) {
    const maximum = Math.max(1, ...values);
    return values.map((value, index) => {
      const x = 4 + index * (252 / Math.max(1, values.length - 1));
      const y = 64 - value / maximum * 56;
      return `${x},${y}`;
    }).join(" ");
  }

  function median(values: number[]) {
    if (!values.length) return 0;
    const sorted = [...values].sort((left, right) => left - right);
    const middle = Math.floor(sorted.length / 2);
    return sorted.length % 2 ? sorted[middle] : Math.round((sorted[middle - 1] + sorted[middle]) / 2);
  }

  function baselineY(values: number[]) {
    return 64 - median(values) / Math.max(1, ...values) * 56;
  }
</script>

<section class="panel spending-composition">
  <div class="heading">
    <div>
      <h2>What personal spending was made of</h2>
      <p>Monthly composition uses reviewed personal shares. Bar length is the monthly total; segments show category mix.</p>
    </div>
  </div>

  {#if months.length && categorySeries.length}
    <div class="legend" aria-label="Spending category legend">
      {#each categorySeries as item (item.id)}
        <span><i style:background={item.color}></i>{item.label}<strong>{money(item.total)}</strong></span>
      {/each}
    </div>
    <div class="composition" aria-label="Monthly category composition">
      {#each months as month, monthIndex (month)}
        <div class="composition-row">
          <time>{month}</time>
          <div class="stack" style:width={`${Math.max(2, monthTotals[monthIndex] / maximumMonthTotal * 100)}%`} use:tip={`${month}: ${money(monthTotals[monthIndex])}`}>
            {#each categorySeries as item (item.id)}
              {#if item.values[monthIndex] > 0}
                <span
                  style:background={item.color}
                  style:width={`${item.values[monthIndex] / Math.max(1, monthTotals[monthIndex]) * 100}%`}
                  use:tip={`${item.label}: ${money(item.values[monthIndex])}`}
                ></span>
              {/if}
            {/each}
          </div>
          <strong>{money(monthTotals[monthIndex])}</strong>
        </div>
      {/each}
    </div>

    {#if smallMultiples.length}
      <div class="subheading">
        <h3>Category histories</h3>
        <p>Each category has its own scale; the dashed line is that category’s monthly median.</p>
      </div>
      <div class="multiples">
        {#each smallMultiples as item (item.id)}
          <article>
            <div><strong>{item.label}</strong><span>{money(item.values.at(-1) ?? 0)} latest</span></div>
            <svg viewBox="0 0 260 70" role="img" aria-label={`${item.label} monthly spending`}>
              <line x1="4" x2="256" y1={baselineY(item.values)} y2={baselineY(item.values)}></line>
              <polyline points={sparkPoints(item.values)} style:stroke={item.color}></polyline>
            </svg>
            <small>{money(Math.max(...item.values))} peak · {money(median(item.values))} median</small>
          </article>
        {/each}
      </div>
    {/if}
  {:else}
    <p class="muted">No reviewed expense history in this period.</p>
  {/if}
</section>

<style>
  .panel { border: 1px solid var(--border, #333); border-radius: 8px; background: var(--card-bg, transparent); padding: .9rem; margin-bottom: .75rem; min-width: 0; }
  h2, h3 { margin: 0; font-size: .85rem; }
  h3 { font-size: .76rem; }
  p, small { margin: .2rem 0 0; color: var(--muted, #888); font-size: .67rem; }
  .legend { display: flex; flex-wrap: wrap; gap: .45rem 1rem; margin: .8rem 0; font-size: .65rem; }
  .legend span { display: inline-flex; align-items: center; gap: .3rem; }
  .legend i { width: .62rem; height: .62rem; border-radius: 2px; }
  .legend strong { margin-left: .12rem; font-variant-numeric: tabular-nums; }
  .composition { display: grid; gap: .42rem; }
  .composition-row { display: grid; grid-template-columns: 4.5rem 1fr 6rem; align-items: center; gap: .6rem; font-size: .68rem; }
  time { color: var(--muted, #888); }
  .composition-row > strong { text-align: right; font-variant-numeric: tabular-nums; }
  .stack { display: flex; height: 12px; border-radius: 3px; overflow: hidden; background: color-mix(in srgb, currentColor 7%, transparent); }
  .stack span { min-width: 2px; height: 100%; }
  .subheading { border-top: 1px solid var(--border, #333); padding-top: .8rem; margin-top: .9rem; }
  .multiples { display: grid; grid-template-columns: repeat(auto-fit, minmax(13rem, 1fr)); gap: .65rem; margin-top: .65rem; }
  .multiples article { border: 1px solid color-mix(in srgb, currentColor 14%, transparent); border-radius: 6px; padding: .65rem; }
  .multiples article > div { display: flex; justify-content: space-between; gap: .5rem; font-size: .68rem; }
  .multiples article span, .multiples small { color: var(--muted, #888); }
  svg { width: 100%; height: 4.2rem; margin-top: .3rem; overflow: visible; }
  svg line { stroke: color-mix(in srgb, currentColor 22%, transparent); stroke-dasharray: 3 4; }
  svg polyline { fill: none; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
  .muted { color: var(--muted, #888); }
  @media (max-width: 520px) { .composition-row { grid-template-columns: 3.8rem 1fr; } .composition-row > strong { grid-column: 2; text-align: left; } }
</style>
