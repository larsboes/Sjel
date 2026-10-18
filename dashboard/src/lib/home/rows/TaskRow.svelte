<script lang="ts">
  import { projects, type Task } from "../../api";
  import Icon from "../../Icon.svelte";
  import ListRow from "../../ListRow.svelte";
  import RowMeta from "../../RowMeta.svelte";
  import { dateLabel, metaParts } from "../format";
  import type { DecisionRowProps } from "../decisions";

  let { row, busy, act, id, current, tone, href, whyHere, dataClass }: DecisionRowProps<Task, Task[]> = $props();

  // A projects task has a dashboard page; a vault task still opens Obsidian and has no write.
  const owned = $derived(row.uri.startsWith("/"));
  const done = () =>
    act(() => projects.setDone(row.id, true).then(() => undefined), {
      dismiss: true,
      patch: (source) => source.filter((item) => item.id !== row.id),
    });



  const kind = $derived(metaParts("Task", row.due ? `due ${dateLabel(row.due)}` : null, ...row.projects));
  const overdue = $derived(row.due !== null && row.due.slice(0, 10) < new Date().toISOString().slice(0, 10));
</script>

<ListRow {id} {current} {tone}>
  {#snippet mark()}<Icon name="check" size={15} />{/snippet}

  <span class="row-kind">
    {#each kind as part}<span
      >{part}</span
    >{/each}
    {#if overdue}<span class="overdue">overdue</span>{/if}
  </span>
  <!-- A vault row links to the note, which Obsidian writes (PRD Q48). A projects row links
       to its project page and can be marked done here. -->
  <a class="row-title" {href}>{row.title}</a>
  {#if row.summary}<p class="row-text">{row.summary}</p>{/if}

  {#snippet meta()}<RowMeta {whyHere} {dataClass} candidateStatus="accepted" />{/snippet}

  {#snippet actions()}
    {#if owned}
      <button type="button" class="btn btn-soft" disabled={busy} onclick={done}>Done</button>
      <a class="btn btn-soft" {href}>Open project</a>
    {:else}
      <a class="btn btn-soft" {href}>Open note</a>
    {/if}
  {/snippet}
</ListRow>

<style>
  .overdue {
    margin-left: 0.35em;
    padding: 0 0.3em;
    border-radius: var(--radius-sm);
    background-color: var(--warning-soft);
    color: var(--warning-ink);
    font-weight: 600;
  }
</style>
