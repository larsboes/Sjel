<script lang="ts">
  import type { TripPlan } from "../../api";
  import { link } from "../../nav";
  import Icon from "../../Icon.svelte";
  import ListRow from "../../ListRow.svelte";
  import RowMeta from "../../RowMeta.svelte";
  import { dateLabel, metaParts } from "../format";
  import type { DecisionRowProps } from "../decisions";

  let {
    row,
    id,
    current,
    tone,
    href,
    whyHere,
    candidateStatus,
    dataClass,
  }: DecisionRowProps<TripPlan> = $props();
  const where = $derived(row.destinations.map((place) => place.name).join(" → "));



  const kind = $derived(metaParts("Travel", dateLabel(row.date_start), where));
</script>

<ListRow {id} {current} {tone} {href}>
  {#snippet mark()}<Icon name="map-pin" size={15} />{/snippet}

  <span class="row-kind">{#each kind as part}<span>{part}</span>{/each}</span>
  <a class="row-title" {href}>{row.title}</a>

  {#snippet meta()}<RowMeta {whyHere} {candidateStatus} {dataClass} />{/snippet}

  {#snippet actions()}
    <a class="btn btn-soft" href={link("/travel")}>
      Continue planning
    </a>
  {/snippet}
</ListRow>
