<script lang="ts">
  import { calendar, type CalendarEntry } from "$lib/api";
  import { KINDS } from "./types";

  let {
    refresh,
    onChanged,
    onCount,
  }: {
    /** Raised by the import review after it creates a draft. */
    refresh: number;
    onChanged: () => Promise<void>;
    /** Reports the pending count to the rail, which shows it on the collapsed row.
     * The fetch stays here rather than moving up: the 90-day draft window is this
     * component's own concern, and the rail should not have to know it. */
    onCount: (count: number) => void;
  } = $props();

  function dateKey(date: Date): string {
    return [
      date.getFullYear(),
      String(date.getMonth() + 1).padStart(2, "0"),
      String(date.getDate()).padStart(2, "0"),
    ].join("-");
  }

  function plusDays(date: Date, days: number): string {
    const shifted = new Date(date);
    shifted.setDate(shifted.getDate() + days);
    return dateKey(shifted);
  }

  const today = new Date();
  const from = dateKey(today);
  const to = plusDays(today, 90);
  let drafts = $state<CalendarEntry[]>([]);
  let proposedKinds = $state<Record<string, string>>({});
  let loading = $state(false);
  let actingId = $state<string | null>(null);
  let error = $state("");
  let seenRefresh = -1;

  async function loadDrafts() {
    loading = true;
    error = "";
    try {
      drafts = await calendar.google.drafts(from, to);
    } catch (cause) {
      error = String(cause);
    } finally {
      loading = false;
    }
    onCount(drafts.length);
  }

  $effect(() => {
    if (refresh === seenRefresh) return;
    seenRefresh = refresh;
    void loadDrafts();
  });

  function kindFor(entry: CalendarEntry): string {
    return proposedKinds[entry.id] ?? entry.kind;
  }

  function setKind(entry: CalendarEntry, kind: string) {
    proposedKinds = { ...proposedKinds, [entry.id]: kind };
  }

  function when(entry: CalendarEntry): string {
    if (entry.all_day) return `${entry.starts_at} (all day)`;
    return `${entry.starts_at} – ${entry.ends_at}`;
  }

  async function adopt(entry: CalendarEntry) {
    actingId = entry.id;
    error = "";
    try {
      // Raising commitment is the durable adoption signal: a later Google
      // refresh can no longer overwrite this entry even if its kind stays busy.
      await calendar.entries.update(entry.id, {
        kind: kindFor(entry),
        commitment: "planned",
      });
      await Promise.all([loadDrafts(), onChanged()]);
    } catch (cause) {
      error = String(cause);
    } finally {
      actingId = null;
    }
  }

  async function remove(entry: CalendarEntry) {
    if (!window.confirm(`Remove “${entry.title}” from Sjel only? Google remains unchanged.`)) return;
    actingId = entry.id;
    error = "";
    try {
      await calendar.entries.delete(entry.id);
      await Promise.all([loadDrafts(), onChanged()]);
    } catch (cause) {
      error = String(cause);
    } finally {
      actingId = null;
    }
  }
</script>

<p class="hint">Adding an entry protects it from later Google updates. Removing it deletes only Sjel's copy.</p>

{#if error}<p class="message error" role="alert">{error}</p>{/if}

{#if loading}
  <p class="empty">Loading Google drafts…</p>
{:else if drafts.length === 0}
  <p class="empty">Nothing pending in the next 90 days.</p>
{:else}
  <div class="draft-list">
    {#each drafts as entry (entry.id)}
      <article class="draft">
        <div class="details">
          <strong>{entry.title}</strong>
          <p>{when(entry)}{#if entry.location} · {entry.location}{/if}</p>
        </div>
        <label>
          <span>As</span>
          <select value={kindFor(entry)} onchange={(event) => setKind(entry, event.currentTarget.value)} disabled={actingId !== null}>
            {#each KINDS as kind}
              <option value={kind.value}>{kind.label}</option>
            {/each}
          </select>
        </label>
        <div class="actions">
          <button class="btn btn-primary" onclick={() => adopt(entry)} disabled={actingId !== null}>
            {actingId === entry.id ? "Saving…" : "Add"}
          </button>
          <button class="btn btn-danger" onclick={() => remove(entry)} disabled={actingId !== null}>Remove</button>
        </div>
      </article>
    {/each}
  </div>
{/if}

<style>
  .hint { margin: 0 0 0.5rem; color: var(--text-secondary); font-size: var(--text-xs); line-height: 1.45; }
  .message { margin: 0 0 0.5rem; padding: 0.5rem 0.6rem; border-radius: var(--radius-sm); font-size: 0.78rem; }
  .error { color: var(--danger); background-color: var(--danger-soft); }
  .empty { margin: 0; color: var(--text-tertiary); font-size: 0.78rem; }
  .draft-list { display: flex; flex-direction: column; }
  .draft { padding: 0.6rem 0; border-top: 1px solid var(--card-border); }
  .details { min-width: 0; }
  .details strong { display: block; font-size: var(--text-sm); font-weight: 600; }
  .details p { margin: 0.15rem 0 0; color: var(--text-secondary); font-size: 0.72rem; }
  label { display: flex; align-items: center; gap: 0.4rem; margin-top: 0.45rem; color: var(--text-secondary); font-size: 0.72rem; }
  select { flex: 1; min-width: 0; padding: 0.25rem 0.35rem; border: 1px solid var(--input-border); border-radius: var(--radius-sm); background-color: var(--input-bg); color: var(--text-primary); font: inherit; font-size: 0.72rem; }
  .actions { display: flex; gap: 0.3rem; margin-top: 0.45rem; }
  .actions .btn { flex: 1; padding: 0.3rem 0.5rem; font-size: 0.72rem; }
</style>
