<script lang="ts">
  import type { ReviewedHoldingsSnapshot } from "$lib/api";
  import DataTable, { type Column } from "$lib/ui/DataTable.svelte";
  import Chip from "$lib/ui/Chip.svelte";

  type Holding = ReviewedHoldingsSnapshot["holdings"][number];

  let { snapshot }: { snapshot: ReviewedHoldingsSnapshot | null } = $props();

  function decimal(mantissa: string, scale: number): string {
    const sign = mantissa.startsWith("-") ? "-" : "";
    const digits = (sign ? mantissa.slice(1) : mantissa).padStart(scale + 1, "0");
    if (scale === 0) return `${sign}${digits}`;
    return `${sign}${digits.slice(0, -scale)}.${digits.slice(-scale)}`;
  }

  function reviewRange(snapshot: ReviewedHoldingsSnapshot): string {
    const dates = snapshot.sources.map((source) => source.reviewed_at).sort();
    if (dates.length === 0) return `Reviewed ${snapshot.reviewed_at}`;
    if (dates[0] === dates.at(-1)) return `All sources reviewed ${dates[0]}`;
    return `Source reviews ${dates[0]} to ${dates.at(-1)}`;
  }

  function sourceCount(snapshot: ReviewedHoldingsSnapshot): number {
    return Math.max(snapshot.sources.length, 1);
  }

  function partialSourceCount(snapshot: ReviewedHoldingsSnapshot): number {
    const count = snapshot.sources.filter((source) => source.coverage === "partial").length;
    return snapshot.coverage === "partial" ? Math.max(count, 1) : count;
  }

  const columns: Column<Holding>[] = [
    { id: "instrument", label: "Instrument", cell: instrumentCell },
    { id: "quantity", label: "Quantity", width: "8rem", align: "end", cell: quantityCell },
    { id: "price", label: "Latest activity price", width: "11rem", align: "end", cell: priceCell },
  ];
</script>

{#snippet instrumentCell(h: Holding)}{h.instrument}{/snippet}
{#snippet quantityCell(h: Holding)}{decimal(h.quantity.mantissa, h.quantity.scale)}{/snippet}
{#snippet priceCell(h: Holding)}
  {h.latest_unit_price === null ? "" : `${decimal(h.latest_unit_price.mantissa, h.latest_unit_price.scale)} ${h.currency}`}
{/snippet}

<section class="portfolio">
  <div class="heading">
    <div>
      <h2>Reviewed holdings</h2>
      {#if snapshot}<p>Only confirmed imports are included. {reviewRange(snapshot)}; prices are from the latest activity, not a live quote.</p>{/if}
    </div>
    {#if snapshot}<strong>{snapshot.holdings.length} open · {sourceCount(snapshot)} source{sourceCount(snapshot) === 1 ? "" : "s"}</strong>{/if}
  </div>
  {#if snapshot === null}
    <p>No reviewed holdings snapshot yet.</p>
  {:else}
    {#if snapshot.coverage === "partial"}
      <p class="coverage-warning"><strong>Incomplete portfolio.</strong> {partialSourceCount(snapshot)} source{partialSourceCount(snapshot) === 1 ? " is" : "s are"} based on partial evidence, so position count and totals are lower bounds.</p>
    {/if}
    {#if snapshot.holdings.length === 0}
      <p>The reviewed snapshot contains no open positions.</p>
    {:else}
      {#if snapshot.sources.length > 0}
        <ul class="sources" aria-label="Reviewed holding sources">
          {#each snapshot.sources as source (source.source_key)}
            <li><span>{source.source_key}</span><Chip label={source.coverage} tone={source.coverage === "partial" ? "warning" : "muted"} /><time datetime={source.reviewed_at}>{source.reviewed_at}</time></li>
          {/each}
        </ul>
      {/if}
      <div class="table-wrap">
        <DataTable rows={snapshot.holdings} {columns} key={(h) => h.instrument} />
      </div>
    {/if}
  {/if}
</section>

<style>
  .portfolio { margin-top: var(--space-3); padding: var(--space-3); border: 1px solid var(--card-border); border-radius: var(--radius-md); background: var(--card-bg); }
  .heading { display: flex; align-items: start; justify-content: space-between; gap: var(--space-4); }
  h2 { margin: 0; font-size: var(--text-sm); }
  p { margin: 0.2rem 0 0; color: var(--text-tertiary); font-size: var(--text-2xs); }
  .heading > strong { font-size: var(--text-xs); font-variant-numeric: tabular-nums; }
  .sources { display: flex; flex-wrap: wrap; gap: var(--space-2); margin: var(--space-3) 0 0; padding: 0; list-style: none; }
  .sources li { display: flex; align-items: center; gap: var(--space-1); padding: 0.2rem 0.4rem; border: 1px solid var(--card-border); border-radius: var(--radius-sm); font-size: var(--text-2xs); }
  .sources time { color: var(--text-tertiary); font-family: var(--font-mono); font-variant-numeric: tabular-nums; }
  .coverage-warning { margin-top: var(--space-3); padding: 0.55rem 0.65rem; border-left: 3px solid var(--warning); background: var(--warning-soft); color: inherit; }
  .table-wrap { margin-top: var(--space-3); }
</style>
