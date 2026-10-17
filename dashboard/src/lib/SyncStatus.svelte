<script lang="ts">
  // The app's sync status line and conflicts view (sync step 2, PRD §10 A5). Rendered only
  // inside the Tauri app: the web shell has no device store. The words and the field diff are
  // in `./sync-status.ts`; the store and the flush rules are in `src-tauri/src/sync.rs`.
  import { onMount } from "svelte";
  import Overlay from "./Overlay.svelte";
  import DataTable, { type Column } from "./ui/DataTable.svelte";
  import Freshness from "./ui/Freshness.svelte";
  import { poll } from "./ui/poll.svelte";
  import {
    bridgeErrorText,
    inTauri,
    syncEntries,
    syncFlush,
    syncResolve,
    syncStatus,
    type OutboxEntry,
    type SyncStatus,
  } from "./mac-bridge";
  import {
    clockTime,
    fieldDiff,
    needsReview,
    showValue,
    statusLine,
    SYNC_CHANGED_EVENT,
    type FieldDiff,
  } from "./sync-status";

  /** How often the line reads the store. Reading is local and cheap; sending is not done here. */
  const POLL_MS = 5_000;

  const shown = inTauri();
  const polled = poll({
    intervalMs: POLL_MS,
    fetch: async () => {
      const next = await syncStatus();
      announce(polled.data, next);
      return next;
    },
  });
  let entries = $state<OutboxEntry[]>([]);
  let open = $state(false);
  let busy = $state(false);
  /** A refused write. A failed read is the poller's, and the line goes stale instead. */
  let actionError = $state<string | null>(null);

  const status = $derived(polled.data);
  const error = $derived(polled.error ?? actionError);
  const line = $derived(status ? statusLine(status) : null);
  const review = $derived(status ? needsReview(status) : false);
  const waiting = $derived((status?.pending ?? 0) > 0);

  /** Fires the change event when the outbox counts move, so the Interior page reloads. */
  function announce(before: SyncStatus | null, next: SyncStatus) {
    if (
      before &&
      (before.pending !== next.pending ||
        before.conflicts !== next.conflicts ||
        before.failed !== next.failed)
    ) {
      window.dispatchEvent(new CustomEvent(SYNC_CHANGED_EVENT));
    }
  }

  /** A write's answer is the newest state on this surface, so it is taken as an arrival. */
  function take(next: SyncStatus) {
    announce(polled.data, next);
    polled.adopt(next);
  }

  async function flush() {
    busy = true;
    actionError = null;
    try {
      take(await syncFlush());
      if (open) entries = await syncEntries();
    } catch (e) {
      actionError = bridgeErrorText(e);
    } finally {
      busy = false;
    }
  }

  async function resolve(id: number, action: "keep_mine" | "discard") {
    busy = true;
    actionError = null;
    try {
      take(await syncResolve(id, action));
      entries = await syncEntries();
      window.dispatchEvent(new CustomEvent(SYNC_CHANGED_EVENT));
    } catch (e) {
      actionError = bridgeErrorText(e);
    } finally {
      busy = false;
    }
  }

  async function showEntries() {
    open = true;
    actionError = null;
    try {
      entries = await syncEntries();
    } catch (e) {
      actionError = bridgeErrorText(e);
    }
  }

  onMount(() => {
    if (!shown) return;
    void flush();
    const stop = polled.start();
    // Back in the foreground: send what waits (item 6 of the step-2 design). The read is the
    // poller's, which catches up on return by itself; this one is about sending.
    const onVisible = () => {
      if (document.visibilityState === "visible") void flush();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", onVisible);
    };
  });

  // Every column wraps. This is the one table in the app where a value is read and compared
  // against the one beside it, and an ellipsis here would hide the difference it exists for.
  const diffColumns: Column<FieldDiff>[] = [
    { id: "field", label: "Field", width: "8rem", wrap: true, cell: fieldNameCell },
    { id: "mine", label: "Yours", wrap: true, cell: mineCell },
    { id: "theirs", label: "On the canonical node", wrap: true, cell: theirsCell },
  ];
</script>

{#snippet fieldNameCell(diff: FieldDiff)}
  <span class="mono">{diff.field}</span>
{/snippet}

{#snippet mineCell(diff: FieldDiff)}
  {showValue(diff.mine)}
{/snippet}

{#snippet theirsCell(diff: FieldDiff)}
  {showValue(diff.theirs)}
{/snippet}

{#if shown && line}
  <aside class="sync" class:offline={status?.offline} class:review role="status">
    <span class="line">{line}</span>
    <Freshness of={polled} />
    {#if waiting}
      <button type="button" class="link" disabled={busy} onclick={() => void flush()}>
        {busy ? "Sending…" : "Sync now"}
      </button>
    {/if}
    {#if review || waiting}
      <button type="button" class="link" onclick={() => void showEntries()}>
        {review ? "Review" : "Show"}
      </button>
    {/if}
  </aside>
{/if}

{#if shown && open}
  <Overlay title="Changes on this device" onClose={() => (open = false)} {busy} width="640px">
    {#if error}<p class="bad">{error}</p>{/if}
    {#if entries.length === 0}
      <p>Nothing waits. Every change reached the canonical node.</p>
    {/if}
    {#each entries as entry (entry.id)}
      <section class="entry" class:conflict={entry.state === "conflict"}>
        <header>
          <strong>{entry.item_id}</strong>
          <span class="state">{entry.state}</span>
          <span class="when">edited {clockTime(entry.updated_at)}</span>
        </header>
        {#if entry.state === "conflict"}
          <p class="hint">
            The canonical node changed this item after you read it. Nothing was overwritten. Pick one.
          </p>
          <DataTable
            rows={fieldDiff(entry)}
            columns={diffColumns}
            key={(diff) => diff.field}
            empty="No field differs: both hold the same values."
          />
          <div class="actions">
            <button type="button" disabled={busy} onclick={() => void resolve(entry.id, "keep_mine")}>
              Keep mine
            </button>
            <button type="button" disabled={busy} onclick={() => void resolve(entry.id, "discard")}>
              Discard mine
            </button>
          </div>
        {:else if entry.state === "failed"}
          <p class="bad">The canonical node refused this change: {entry.error ?? "no reason given"}</p>
          <div class="actions">
            <button type="button" disabled={busy} onclick={() => void resolve(entry.id, "discard")}>
              Discard mine
            </button>
          </div>
        {:else}
          <p class="hint">
            Waiting to be sent{entry.attempts > 0 ? ` (${entry.attempts} tries)` : ""}.
            {#if entry.error}Last error: {entry.error}{/if}
          </p>
          <ul class="fields">
            {#each fieldDiff({ body: entry.body, current: null }) as d (d.field)}
              <li><span class="mono">{d.field}</span>: {showValue(d.mine)}</li>
            {/each}
          </ul>
          <div class="actions">
            <button type="button" disabled={busy} onclick={() => void resolve(entry.id, "discard")}>
              Discard mine
            </button>
          </div>
        {/if}
      </section>
    {/each}
  </Overlay>
{/if}

<style>
  .sync {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem 0.9rem;
    align-items: center;
    padding: 0.4rem 1rem;
    font-size: var(--text-sm);
    border-bottom: 1px solid var(--border, rgba(128, 128, 128, 0.3));
    background: var(--surface-2, rgba(128, 128, 128, 0.12));
  }
  .sync.review {
    background: var(--warn-bg, rgba(200, 120, 0, 0.18));
  }
  .link {
    background: none;
    border: 0;
    padding: 0;
    color: inherit;
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }
  .entry {
    border-top: 1px solid var(--border, rgba(128, 128, 128, 0.3));
    padding: 0.6rem 0;
  }
  .entry header {
    display: flex;
    gap: 0.6rem;
    align-items: baseline;
  }
  .state,
  .when,
  .hint {
    opacity: 0.75;
    font-size: var(--text-sm);
  }
  .fields {
    margin: 0.3rem 0;
    padding-left: 1.1rem;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
  }
  .bad {
    color: var(--danger, #b00020);
  }
</style>
