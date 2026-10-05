<script lang="ts">
  import { comms } from "../../api";
  import Icon from "../../Icon.svelte";
  import ListRow from "../../ListRow.svelte";
  import RowMeta from "../../RowMeta.svelte";
  import { metaParts, relativeDate } from "../format";
  import type { MailRow } from "../kinds/mail";
  import type { DecisionRowProps } from "../decisions";
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
  }: DecisionRowProps<MailRow, MailRow[]> = $props();



  const kind = $derived(metaParts("Mail", row.from_addr ?? "unknown sender", row.internal_date ? relativeDate(row.internal_date) : null));
  const dismiss = () =>
    act(() => comms.setTriageStatus(row.id, "dismissed").then(() => undefined), {
      dismiss: true,
      patch: (source) => source.filter((item) => item.id !== row.id),
    });
</script>

<ListRow {id} {current} {tone} {href}>
  {#snippet mark()}<Icon name="mail" size={15} />{/snippet}

  <span class="row-kind">
    {#each kind as part}<span
      >{part}</span
    >{/each}
  </span>
  <a class="row-title" {href}>{row.subject ?? "(no subject)"}</a>
  {#if row.snippet}<p class="row-text">{row.snippet}</p>{/if}

  {#snippet meta()}
    <!-- The rung's own rationale and the method that produced it. This row's ladder
         position comes from a model, and PRD:227 says an agent ranks AND explains; both
         fields were already on the client's TriageItem and neither was ever shown. The
         class marker is visible and inert — the ranking policy states plainly that the
         data class does not move a score. -->
    <RowMeta {whyHere} {dataClass} {candidateStatus} method={row.classification_method} />
  {/snippet}

  {#snippet actions()}
    <a class="btn" {href}>Open</a>
  {/snippet}

  {#snippet secondary()}
    <!-- Local only. Dismissing drops the proposal from this list and changes nothing in
         Gmail: the archive and trash actions live on the entry page, behind their own
         confirmation, because they leave Sjel. -->
    <button
      class="btn"
      type="button"
      disabled={busy}
      aria-label="Dismiss mail proposal"
      use:tip={"Dismiss"}
      onclick={dismiss}
    >
      {#if busy}<Icon name="loader" size={13} />{:else}<Icon name="close" size={13} />{/if}
    </button>
  {/snippet}
</ListRow>
