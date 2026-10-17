<script lang="ts">
  import Chip from "./Chip.svelte";
  import { fastestInterval, freshnessLine, intervalLabel, type FreshnessSource } from "./freshness";

  /**
   * The age of a view, in one place.
   *
   * Quiet while the data is as new as the page's own beat, and one warning chip when it is
   * not: "as of 12m ago", with the beat it missed in the tip. A line that always names an
   * age is a line the reader learns to skip, and the reader who skips it is the one who
   * needed it — so nothing renders until something is late.
   *
   * `of` takes the pollers behind the region this chip describes. Several of them resolve
   * to the oldest, and the tip names the fastest beat: a page reading a 3 s beat and a 4 s
   * beat is as old as the slower one.
   */
  let { of }: { of: FreshnessSource | FreshnessSource[] } = $props();

  const sources = $derived(Array.isArray(of) ? of : [of]);
  const line = $derived(freshnessLine(sources));
  const beat = $derived(fastestInterval(sources));
  const hint = $derived(beat === null ? "" : `read again every ${intervalLabel(beat)} while this page is open`);
</script>

{#if line}
  <span class="freshness"><Chip label={line} tone="warning" {hint} /></span>
{/if}

<style>
  .freshness {
    display: inline-flex;
    align-items: center;
  }
</style>
