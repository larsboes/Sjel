<script lang="ts">
  import { onMount } from "svelte";
  import {
    calendar,
    type CalendarGoogleExportOptIn,
    type CalendarGoogleExportReport,
  } from "$lib/api";

  let {
    onSynced,
    onClose,
  }: {
    onSynced: () => Promise<void>;
    onClose: () => void;
  } = $props();

  let optIns = $state<CalendarGoogleExportOptIn[]>([]);
  let preview = $state<CalendarGoogleExportReport | null>(null);
  let loading = $state(true);
  let pushing = $state(false);
  let error = $state("");
  let notice = $state("");

  async function load() {
    loading = true;
    error = "";
    try {
      optIns = await calendar.google.exports();
    } catch (cause) {
      error = String(cause);
    } finally {
      loading = false;
    }
  }

  onMount(() => { void load(); });

  async function makePreview() {
    loading = true;
    error = "";
    notice = "";
    try {
      preview = await calendar.google.previewExport();
    } catch (cause) {
      error = String(cause);
      preview = null;
    } finally {
      loading = false;
    }
  }

  async function push() {
    if (!preview || preview.pushed.length === 0) return;
    if (!window.confirm(`Send ${preview.pushed.length} entries to Google now?`)) return;
    pushing = true;
    error = "";
    notice = "";
    try {
      const report = await calendar.google.export();
      preview = report;
      notice = `${report.inserted} created, ${report.patched} updated.`;
      await onSynced();
      await load();
    } catch (cause) {
      error = String(cause);
    } finally {
      pushing = false;
    }
  }
</script>

<section class="review" aria-labelledby="google-export-title">
  <div class="heading">
    <div>
      <p class="eyebrow">Google Calendar</p>
      <h2 id="google-export-title">Review export</h2>
      <p>Only individually approved Sjel entries are included. The preview does not change Google.</p>
    </div>
    <button class="btn" onclick={onClose} disabled={loading || pushing}>Close</button>
  </div>

  {#if error}
    <p class="message error" role="alert">{error}</p>
  {:else if notice}
    <p class="message success">{notice}</p>
  {/if}

  <div class="summary">
    <span>{optIns.length} individually approved</span>
    <span>{optIns.filter((entry) => entry.google_event_id).length} already linked to Google</span>
  </div>

  <div class="actions">
    <button class="btn primary" onclick={makePreview} disabled={loading || pushing || optIns.length === 0}>
      {loading ? "Loading…" : "Create preview"}
    </button>
  </div>

  {#if !loading && optIns.length === 0}
    <p class="empty">No entry is approved for Google yet. Open an Sjel entry and enable export there.</p>
  {/if}

  {#if preview}
    <div class="preview">
      <p>{preview.pushed.length} entries would be sent.</p>
      <ul>
        {#each preview.pushed as entry (entry.entry_id)}
          <li><strong>{entry.title}</strong> — {entry.operation === "inserted" ? "create" : "update"}</li>
        {/each}
      </ul>
      {#if preview.skipped.length > 0}
        <p class="skipped">{preview.skipped.length} skipped: {preview.skipped.map((item) => item.reason).join(" · ")}</p>
      {/if}
      {#if preview.dry_run}
        <button class="btn danger" onclick={push} disabled={pushing || preview.pushed.length === 0}>
          {pushing ? "Sending…" : `Send ${preview.pushed.length} to Google`}
        </button>
      {/if}
    </div>
  {/if}
</section>

<style>
  .review { margin: 0 0 20px; padding: 18px; border: 1px solid var(--card-border); border-radius: 12px; background: var(--card-bg); }
  .heading, .summary, .actions { display: flex; align-items: center; gap: 10px; }
  .heading { justify-content: space-between; align-items: flex-start; }
  .heading h2 { margin: 2px 0 4px; font-size: 1.1rem; }
  .heading p { margin: 0; color: var(--text-secondary); font-size: .86rem; max-width: 48rem; }
  .eyebrow { color: var(--primary) !important; font-size: .72rem !important; font-weight: 700; letter-spacing: .08em; text-transform: uppercase; }
  .summary { margin-top: 14px; flex-wrap: wrap; color: var(--text-secondary); font-size: .8rem; }
  .summary span { padding: 4px 7px; border-radius: 999px; background: var(--surface); }
  .actions { margin-top: 14px; }
  .btn { padding: 7px 12px; border: 0; border-radius: 7px; background: var(--surface); color: var(--text-primary); font: inherit; font-size: .82rem; cursor: pointer; }
  .btn:disabled { opacity: .5; cursor: wait; }
  .primary { background: var(--primary); color: #fff; }
  .danger { background: var(--danger); color: var(--text-inverse); }
  .message { margin: 12px 0 0; padding: 8px 10px; border-radius: 7px; font-size: .84rem; }
  .error { color: var(--danger); background-color: var(--danger-soft); }
  .success { color: var(--success); background-color: var(--success-soft); }
  .preview { margin-top: 16px; padding-top: 14px; border-top: 1px solid var(--card-border); }
  .preview p { margin: 0 0 8px; color: var(--text-secondary); font-size: .84rem; }
  ul { margin: 0 0 14px; padding-left: 20px; color: var(--text-secondary); font-size: .84rem; }
  li + li { margin-top: 4px; }
  .skipped { color: var(--warning-ink) !important; }
  .empty { margin: 14px 0 0; color: var(--text-secondary); font-size: .86rem; }
  @media (max-width: 640px) { .heading { display: block; } .heading .btn { margin-top: 10px; } }
</style>
