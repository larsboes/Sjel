<script lang="ts">
  import {
    calendar,
    type CalendarGoogleImportCandidate,
    type CalendarGoogleImportPreview,
  } from "$lib/api";

  let {
    onImported,
    onClose,
  }: {
    onImported: () => Promise<void>;
    onClose: () => void;
  } = $props();

  function dateKey(date: Date): string {
    return [
      date.getFullYear(),
      String(date.getMonth() + 1).padStart(2, "0"),
      String(date.getDate()).padStart(2, "0"),
    ].join("-");
  }

  function plusDays(date: Date, days: number): string {
    const next = new Date(date);
    next.setDate(next.getDate() + days);
    return dateKey(next);
  }

  const today = new Date();
  let from = $state(dateKey(today));
  let to = $state(plusDays(today, 30));
  let preview = $state<CalendarGoogleImportPreview | null>(null);
  let selected = $state<Set<string>>(new Set());
  let showNonCandidates = $state(false);
  let showDuplicatesOnly = $state(false);
  let search = $state("");
  let loading = $state(false);
  let importing = $state(false);
  let error = $state("");
  let notice = $state("");

  let candidates = $derived(preview?.candidates ?? []);
  let importable = $derived(candidates.filter((candidate) => candidate.status === "importable"));
  let duplicates = $derived(candidates.filter((candidate) => candidate.status === "likely-duplicate"));
  let visible = $derived(
    candidates.filter((candidate) => {
      if (!showNonCandidates && candidate.status !== "importable" && candidate.status !== "likely-duplicate") {
        return false;
      }
      if (showDuplicatesOnly && candidate.status !== "likely-duplicate") return false;
      const terms = `${candidate.title} ${candidate.location ?? ""}`.toLocaleLowerCase("en-GB");
      return terms.includes(search.trim().toLocaleLowerCase("en-GB"));
    }),
  );

  function isSelectable(candidate: CalendarGoogleImportCandidate): boolean {
    return candidate.status === "importable" || candidate.status === "likely-duplicate";
  }

  function labelFor(status: CalendarGoogleImportCandidate["status"]): string {
    return {
      importable: "Importable",
      "likely-duplicate": "Possible duplicate",
      "already-in-axon": "Already in Sjel",
      cancelled: "Cancelled in Google",
      invalid: "Not importable",
    }[status];
  }

  function when(candidate: CalendarGoogleImportCandidate): string {
    if (!candidate.starts_at) return "Time is missing or cannot be imported";
    if (candidate.all_day) return candidate.ends_at
      ? `${candidate.starts_at} – ${candidate.ends_at} (all day, exclusive end)`
      : `${candidate.starts_at} (all day)`;
    return candidate.ends_at ? `${candidate.starts_at} – ${candidate.ends_at}` : candidate.starts_at;
  }

  async function loadPreview() {
    loading = true;
    error = "";
    notice = "";
    try {
      preview = await calendar.google.previewImport(from, to);
      selected = new Set();
    } catch (cause) {
      error = String(cause);
      preview = null;
    } finally {
      loading = false;
    }
  }

  function invalidatePreview() {
    preview = null;
    selected = new Set();
    notice = "";
    error = "";
  }

  function setRange(days: number) {
    from = dateKey(today);
    to = plusDays(today, days);
    invalidatePreview();
    void loadPreview();
  }

  function toggle(candidate: CalendarGoogleImportCandidate) {
    if (!isSelectable(candidate)) return;
    const next = new Set(selected);
    if (next.has(candidate.google_event_id)) {
      next.delete(candidate.google_event_id);
    } else {
      // A cluster is a warning, not a verdict. Still, only one member can be
      // selected in one import, both here and again on the server.
      if (candidate.duplicate_group) {
        for (const other of candidates) {
          if (other.duplicate_group === candidate.duplicate_group) next.delete(other.google_event_id);
        }
      }
      next.add(candidate.google_event_id);
    }
    selected = next;
  }

  function selectRecommended() {
    // Duplicates are deliberately absent: "recommended" means the server did
    // not find an exact-title-and-time twin, not merely that an event looks nice.
    selected = new Set(importable.map((candidate) => candidate.google_event_id));
  }

  async function commit() {
    if (!preview || selected.size === 0) return;
    importing = true;
    error = "";
    notice = "";
    try {
      const chosen = candidates
        .filter((candidate) => selected.has(candidate.google_event_id))
        .map((candidate) => ({
          google_event_id: candidate.google_event_id,
          google_updated: candidate.google_updated,
        }));
      const report = await calendar.google.importSelected(from, to, chosen);
      notice = `${report.created} imported as drafts.`;
      selected = new Set();
      await onImported();
      // Reload rather than locally guessing which candidates are now in Sjel.
      preview = await calendar.google.previewImport(from, to);
    } catch (cause) {
      error = String(cause);
    } finally {
      importing = false;
    }
  }
</script>

<section class="review" aria-labelledby="google-import-title">
  <div class="heading">
    <div>
      <p class="eyebrow">Google Calendar</p>
      <h2 id="google-import-title">Review import</h2>
      <p>Google remains unchanged. Only selected, unchanged events become non-blocking Sjel drafts.</p>
    </div>
    <button class="btn" onclick={onClose} disabled={loading || importing}>Close</button>
  </div>

  <div class="range">
    <label>From <input type="date" bind:value={from} onchange={invalidatePreview} disabled={loading || importing} /></label>
    <label>To <input type="date" bind:value={to} onchange={invalidatePreview} disabled={loading || importing} /></label>
    <button class="btn primary" onclick={loadPreview} disabled={loading || importing}>
      {loading ? "Checking Google…" : "Load preview"}
    </button>
  </div>
  <div class="quick-ranges" aria-label="Quick ranges">
    <button class="text-button" onclick={() => setRange(7)} disabled={loading || importing}>7 days</button>
    <button class="text-button" onclick={() => setRange(30)} disabled={loading || importing}>30 days</button>
    <button class="text-button" onclick={() => setRange(90)} disabled={loading || importing}>90 days</button>
  </div>
  <p class="hint">Up to 90 days per review. A recurring event is not automatically a duplicate.</p>

  {#if error}
    <p class="message error" role="alert">{error}</p>
  {:else if notice}
    <p class="message success">{notice}</p>
  {/if}

  {#if preview}
    <div class="summary">
      <span>{preview.fetched} loaded from Google</span>
      <span>{importable.length} importable</span>
      <span>{duplicates.length} possible duplicates</span>
      {#if preview.at_event_limit}<span class="warning">Results may be truncated</span>{/if}
    </div>

    <div class="review-actions">
      <div class="filters">
        <label class="search">Search <input type="search" bind:value={search} placeholder="Title or location" /></label>
        <label class="check"><input type="checkbox" bind:checked={showDuplicatesOnly} /> Possible duplicates only</label>
        <label class="check"><input type="checkbox" bind:checked={showNonCandidates} /> Show existing and excluded events</label>
      </div>
      <div>
        <button class="btn" onclick={selectRecommended} disabled={importable.length === 0 || importing}>Select recommended</button>
        <button class="btn primary" onclick={commit} disabled={selected.size === 0 || importing}>
          {importing ? "Importing…" : `Import ${selected.size} as drafts`}
        </button>
      </div>
    </div>

    <div class="candidate-list" aria-label="Google import candidates">
      {#each visible as candidate (candidate.google_event_id)}
        <article class:muted={!isSelectable(candidate)} class:duplicate={candidate.status === "likely-duplicate"} class="candidate">
          <label class="select">
            <input
              type="checkbox"
              checked={selected.has(candidate.google_event_id)}
              disabled={!isSelectable(candidate) || importing}
              onchange={() => toggle(candidate)}
            />
          </label>
          <div class="details">
            <div class="title-row">
              <strong>{candidate.title}</strong>
              <span class="status {candidate.status}">{labelFor(candidate.status)}</span>
              {#if candidate.recurring_event_id}<span class="recurring">Recurring</span>{/if}
            </div>
            <p>{when(candidate)}{#if candidate.location} · {candidate.location}{/if}</p>
            {#if candidate.reason}<p class="reason">{candidate.reason}</p>{/if}
          </div>
          {#if candidate.html_link}
            <a href={candidate.html_link} target="_blank" rel="noreferrer">Open in Google</a>
          {/if}
        </article>
      {:else}
        <p class="empty">There are no importable candidates in this period.</p>
      {/each}
    </div>
  {/if}
</section>

<style>
  .review { margin: 0 0 20px; padding: 18px; border: 1px solid var(--card-border); border-radius: 12px; background: var(--card-bg); }
  .heading, .range, .review-actions, .summary, .title-row { display: flex; align-items: center; gap: 10px; }
  .heading, .review-actions { justify-content: space-between; align-items: flex-start; }
  .heading h2 { margin: 2px 0 4px; font-size: 1.1rem; }
  .heading p { margin: 0; color: var(--text-secondary); font-size: .86rem; max-width: 48rem; }
  .eyebrow { color: var(--primary) !important; font-size: .72rem !important; font-weight: 700; letter-spacing: .08em; text-transform: uppercase; }
  .range { margin-top: 14px; flex-wrap: wrap; }
  .range label, .check { display: inline-flex; align-items: center; gap: 6px; color: var(--text-secondary); font-size: .82rem; }
  input[type="date"] { border: 1px solid var(--card-border); border-radius: 6px; padding: 5px 7px; background: var(--surface); color: var(--text-primary); font: inherit; }
  .quick-ranges { display: flex; gap: 9px; margin-top: 7px; }
  .text-button { padding: 0; border: 0; background: transparent; color: var(--primary); font: inherit; font-size: .78rem; cursor: pointer; text-decoration: underline; }
  .hint, .reason { margin: 7px 0 0; color: var(--text-secondary); font-size: .78rem; }
  .message { margin: 12px 0 0; padding: 8px 10px; border-radius: 7px; font-size: .84rem; }
  .error { color: var(--danger); background-color: var(--danger-soft); }
  .success { color: var(--success); background-color: var(--success-soft); }
  .summary { margin-top: 14px; flex-wrap: wrap; color: var(--text-secondary); font-size: .8rem; }
  .summary span { padding: 4px 7px; border-radius: 999px; background: var(--surface); }
  .summary .warning { color: var(--warning-ink); background-color: var(--warning-soft); }
  .review-actions { margin: 14px 0 8px; flex-wrap: wrap; }
  .filters { display: flex; gap: 9px; align-items: center; flex-wrap: wrap; }
  .search { display: inline-flex; align-items: center; gap: 6px; color: var(--text-secondary); font-size: .82rem; }
  input[type="search"] { width: 11rem; border: 1px solid var(--card-border); border-radius: 6px; padding: 5px 7px; background: var(--surface); color: var(--text-primary); font: inherit; }
  .review-actions > div { display: flex; gap: 7px; flex-wrap: wrap; }
  .candidate-list { border-top: 1px solid var(--card-border); max-height: 32rem; overflow: auto; }
  .candidate { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 10px; align-items: start; padding: 10px 3px; border-bottom: 1px solid var(--card-border); }
  .candidate.duplicate { background: var(--warning-soft); }
  .candidate.muted { opacity: .7; }
  .select { padding-top: 2px; }
  .details { min-width: 0; }
  .title-row { flex-wrap: wrap; }
  .details strong { font-size: .9rem; }
  .details p { margin: 3px 0 0; color: var(--text-secondary); font-size: .8rem; }
  .status, .recurring { padding: 2px 6px; border-radius: 999px; font-size: .7rem; background: var(--surface); color: var(--text-secondary); }
  .status.importable { color: var(--success); background-color: var(--success-soft); }
  .status.likely-duplicate { color: var(--warning-ink); background-color: var(--warning-soft); }
  .status.invalid, .status.cancelled { color: var(--danger); background-color: var(--danger-soft); }
  .recurring { color: var(--primary); background-color: var(--primary-soft); }
  .candidate a { color: var(--primary); font-size: .78rem; white-space: nowrap; }
  .empty { color: var(--text-secondary); font-size: .86rem; padding: 16px 0; }
  @media (max-width: 640px) {
    .heading { display: block; }
    .heading .btn { margin-top: 10px; }
    .candidate { grid-template-columns: auto minmax(0, 1fr); }
    .candidate a { grid-column: 2; }
  }
</style>
