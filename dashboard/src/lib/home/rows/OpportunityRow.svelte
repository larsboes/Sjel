<script lang="ts">
  import { scouting, type ScoutingOpportunity } from "../../api";
  import Icon from "../../Icon.svelte";
  import ListRow from "../../ListRow.svelte";
  import RowMeta from "../../RowMeta.svelte";
  import { dateLabel, metaParts } from "../format";
  import type { DecisionRowProps } from "../decisions";
  import type { OpportunitySource } from "../kinds/opportunity";
  import { tip } from "../../tip";

  let {
    row,
    busy,
    act,
    id,
    current,
    tone,
    href,
    whyHere,
    dataClass,
    candidateStatus,
  }: DecisionRowProps<ScoutingOpportunity, OpportunitySource> = $props();



  const kind = $derived(metaParts("Opportunity", row.starts_at ? dateLabel(row.starts_at) : null, row.city));
  /// A decided opportunity leaves the kind's source, not just the ladder. Locations lists
  /// every opportunity still `new` and Sources counts them, so hiding it from the queue
  /// alone left the page showing a call the operator had already made two tabs over. The
  /// base page dropped it from the same array by hand.
  const decide = (status: "saved" | "dismissed") => () =>
    act(() => scouting.setStatus(row.id, status).then(() => undefined), {
      dismiss: true,
      patch: (source) => ({
        ...source,
        opportunities: source.opportunities.filter((entry) => entry.id !== row.id),
      }),
    });
</script>

<ListRow {id} {current} {tone}>
  {#snippet mark()}<Icon name="compass" size={15} />{/snippet}

  <span class="row-kind">
    {#each kind as part}<span
      >{part}</span
    >{/each}
  </span>
  <a class="row-title" {href} target="_blank" rel="noreferrer">{row.title}</a>

  {#snippet meta()}<RowMeta {whyHere} {dataClass} {candidateStatus} />{/snippet}

  {#snippet actions()}
    <a class="btn" {href} target="_blank" rel="noreferrer">Open</a>
    <button
      class="btn btn-soft"
      type="button"
      disabled={busy}
      onclick={decide("saved")}
    >
      {#if busy}<Icon name="loader" size={13} />{:else}Save{/if}
    </button>
  {/snippet}

  {#snippet secondary()}
    <button
      class="btn"
      type="button"
      disabled={busy}
      aria-label="Dismiss opportunity"
      use:tip={"Dismiss"}
      onclick={decide("dismissed")}
    >
      <Icon name="close" size={13} />
    </button>
  {/snippet}
</ListRow>
