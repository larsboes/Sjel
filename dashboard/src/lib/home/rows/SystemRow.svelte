<script lang="ts">
  import type { SjelStatusHealth } from "../../api";
  import { link } from "../../nav";
  import Icon from "../../Icon.svelte";
  import ListRow from "../../ListRow.svelte";
  import RowMeta from "../../RowMeta.svelte";
  import type { DecisionRowProps } from "../decisions";

  let { row, id, current, tone, whyHere }: DecisionRowProps<SjelStatusHealth> = $props();

  const down = $derived(
    Object.entries(row.capabilities)
      .filter(([, state]) => !state.up)
      .map(([name]) => name),
  );
</script>

<ListRow {id} {current} {tone}>
  {#snippet mark()}<span class="alarm"><Icon name="alert" size={15} /></span>{/snippet}

  <span class="row-kind">System</span>
  <a class="row-title" href={link("/capabilities")}>Check autostart</a>
  {#if down.length > 0}<p class="row-text mono">{down.join(", ")}</p>{/if}

  {#snippet meta()}<RowMeta {whyHere} />{/snippet}

  {#snippet actions()}
    <a class="btn btn-soft" href={link("/capabilities")}>
      Check
    </a>
  {/snippet}
</ListRow>

<style>
  .alarm {
    color: var(--band-alarm);
  }
</style>
