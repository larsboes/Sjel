<script lang="ts">
  import { tip } from "$lib/tip";
  import { onMount } from "svelte";
  import Overlay from "$lib/Overlay.svelte";
  import Icon from "$lib/Icon.svelte";
  import { link } from "$lib/nav";
  import {
    COMMITMENTS,
    KINDS,
    whenError,
    whenOf,
    whenPatch,
    type Commitment,
  } from "./types";
  import type {
    CalendarEntry,
    CalendarGoogleExportOptIn,
    CalendarNewEntry,
    CalendarUpdateEntry,
  } from "$lib/api";

  let {
    entry,
    draft,
    date,
    rangeEndDate,
    eyebrow,
    notice,
    googleExport,
    onSave,
    onGoogleExportChange,
    onDelete,
    onClose,
  }: {
    entry?: CalendarEntry;
    draft?: CalendarNewEntry;
    date?: string;
    rangeEndDate?: string;
    /** Overrides the kicker above the heading — a draft can come from
     * Discover or from an empty slot in a time view. */
    eyebrow?: string;
    notice?: string;
    googleExport?: CalendarGoogleExportOptIn;
    onSave: (data: CalendarNewEntry | CalendarUpdateEntry) => Promise<void>;
    onGoogleExportChange?: (entry: CalendarEntry, optedIn: boolean) => Promise<void>;
    onDelete?: () => Promise<void>;
    onClose: () => void;
  } = $props();

  let titleInput: HTMLInputElement;
  let kind = $state("busy");
  // Typing something in by hand means it is real; scouting's promotions are
  // the ones that arrive as mere possibilities. The wire default is
  // `possible`, so this form says what it means instead of inheriting it.
  let commitment = $state<Commitment>("committed");
  let title = $state("");
  let allDay = $state(true);
  let startDate = $state("");
  // Inclusive in the UI. The HTTP contract remains exclusive for all-day entries.
  let endDate = $state("");
  let startTime = $state("");
  let endTime = $state("");
  let location = $state("");
  let notes = $state("");
  let saving = $state(false);
  let updatingGoogleExport = $state(false);
  let googleExported = $state(false);
  let error = $state("");

  function initialise() {
    kind = entry?.kind ?? draft?.kind ?? "busy";
    commitment = entry?.commitment ?? "committed";
    title = entry?.title ?? draft?.title ?? "";
    // The inclusive/exclusive conversion lives in `whenOf` so this form and the
    // reader cannot disagree about which day an all-day entry ends on.
    const existing = entry ?? draft;
    const when = existing ? whenOf(existing) : null;
    allDay = when?.allDay ?? true;
    startDate = when?.startDate ?? date ?? "";
    endDate = when?.endDate ?? rangeEndDate ?? date ?? "";
    startTime = when && !when.allDay ? when.startTime : "09:00";
    endTime = when && !when.allDay ? when.endTime : "10:00";
    location = entry?.location ?? draft?.location ?? "";
    notes = entry?.notes ?? draft?.notes ?? "";
  }

  onMount(() => {
    initialise();
    googleExported = Boolean(googleExport);
    titleInput.focus();
  });

  function googleExportUnavailable(): string | null {
    if (!entry) return null;
    if (entry.source === "google") return "This entry came from Google and will not be exported back.";
    if (entry.rhythm_id) return "Rhythm instances are not exported to Google individually.";
    return null;
  }

  async function toggleGoogleExport() {
    if (!entry || !onGoogleExportChange || googleExportUnavailable()) return;
    const next = !googleExported;
    updatingGoogleExport = true;
    error = "";
    try {
      await onGoogleExportChange(entry, next);
      googleExported = next;
    } catch (cause) {
      error = String(cause);
    } finally {
      updatingGoogleExport = false;
    }
  }

  async function save() {
    if (!title.trim()) {
      error = "Title is required";
      return;
    }
    if (!startDate || !endDate) {
      error = "Date is required";
      return;
    }
    const whenProblem = whenError({ allDay, startDate, endDate, startTime, endTime });
    if (whenProblem) {
      error = whenProblem;
      return;
    }

    error = "";
    saving = true;
    try {
      const common = {
        kind,
        commitment,
        title: title.trim(),
        all_day: allDay,
        location: location.trim() || null,
        notes: notes.trim() || null,
        ...(!entry && draft
          ? {
              source: draft.source,
              external_id: draft.external_id,
              rhythm_id: draft.rhythm_id,
              payload: draft.payload,
            }
          : {}),
      };
      await onSave({
        ...common,
        ...whenPatch({ allDay, startDate, endDate, startTime, endTime }),
      });
    } catch (cause) {
      error = String(cause);
    } finally {
      saving = false;
    }
  }

  async function deleteEntry() {
    if (!onDelete || !window.confirm(`Delete “${title}”?`)) return;
    error = "";
    saving = true;
    try {
      await onDelete();
    } catch (cause) {
      error = String(cause);
      saving = false;
    }
  }
</script>

<Overlay
  title={entry ? "Edit entry" : "New entry"}
  eyebrow={eyebrow ?? (entry ? "Calendar entry" : draft ? "From Discover" : rangeEndDate ? "Date range" : "Calendar entry")}
  busy={saving}
  {onClose}
>
  {#if entry?.rhythm_id}
    <p class="notice">Changes detach this entry from its rhythm. The rhythm itself remains unchanged.</p>
  {:else if notice}
    <p class="notice">{notice}</p>
  {/if}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <fieldset>
    <legend>Type</legend>
    <div class="kind-picker">
      {#each KINDS as option}
        <button
          type="button"
          class="kind-btn"
          class:selected={option.value === kind}
          style={`--kind-color: ${option.color}`}
          onclick={() => (kind = option.value)}
        >
          {option.label}
        </button>
      {/each}
    </div>
  </fieldset>

  <fieldset>
    <legend>Commitment</legend>
    <div class="kind-picker">
      {#each COMMITMENTS as option}
        <button
          type="button"
          class="kind-btn commitment-btn"
          class:selected={option.value === commitment}
          use:tip={option.hint}
          onclick={() => (commitment = option.value)}
        >
          {option.label}
        </button>
      {/each}
    </div>
    <p class="fieldnote">{COMMITMENTS.find((c) => c.value === commitment)?.hint}</p>
  </fieldset>

  <label>
    <span>Title</span>
    <input bind:this={titleInput} type="text" bind:value={title} placeholder="What for?" />
  </label>

  <label class="checkbox">
    <input type="checkbox" bind:checked={allDay} />
    <span>All day</span>
  </label>

  <div class="dates">
    <label>
      <span>Start</span>
      <input type="date" bind:value={startDate} />
      {#if !allDay}
        <input type="time" bind:value={startTime} />
      {/if}
    </label>
    <label>
      <span>{allDay ? "Last day" : "End"}</span>
      <input type="date" bind:value={endDate} min={startDate} />
      {#if !allDay}
        <input type="time" bind:value={endTime} />
      {/if}
    </label>
  </div>

  <label>
    <span>Location</span>
    <input type="text" bind:value={location} placeholder="Optional" />
    {#if location.trim()}
      <span class="location-links">
        <a class="location-link" href={link('/map')} target="_blank">
          <Icon name="map-pin" size={11} /> View on Map
        </a>
        <a class="location-link" href={link('/travel/connections')}>
          <Icon name="train" size={11} /> Check Transit
        </a>
      </span>
    {/if}
  </label>

  <label>
    <span>Note</span>
    <textarea bind:value={notes} rows="3" placeholder="Optional"></textarea>
  </label>

  {#if entry}
    <fieldset class="google-export">
      <legend>Google Calendar</legend>
      {#if googleExportUnavailable()}
        <p class="fieldnote">{googleExportUnavailable()}</p>
      {:else}
        <label class="checkbox">
          <input
            type="checkbox"
            checked={googleExported}
            disabled={updatingGoogleExport || saving}
            onchange={toggleGoogleExport}
          />
          <span>Approve for Google export</span>
        </label>
        <p class="fieldnote">
          {googleExported
            ? "In the export queue. The entry is sent only when you run Google sync."
            : "Not yet approved for Google."}
        </p>
      {/if}
    </fieldset>
  {/if}

  <div class="actions">
    {#if entry && onDelete}
      <button class="btn danger" onclick={deleteEntry} disabled={saving}>
        Delete
      </button>
    {/if}
    <div class="spacer"></div>
    <button class="btn secondary" onclick={onClose} disabled={saving}>Cancel</button>
    <button class="btn primary" onclick={save} disabled={saving}>
      {saving ? "Saving…" : "Save"}
    </button>
  </div>
</Overlay>

<style>
  /* Dialog chrome — backdrop, sheet, heading, close — lives in `$lib/Overlay.svelte`. */
  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    margin-bottom: 13px;
    color: var(--text-secondary);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  fieldset {
    min-width: 0;
    margin: 0 0 13px;
    padding: 0;
    border: 0;
  }

  legend {
    margin-bottom: 5px;
    padding: 0;
    color: var(--text-secondary);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .checkbox {
    flex-direction: row;
    align-items: center;
  }

  .kind-picker {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }

  .kind-btn {
    padding: 5px 10px;
    border: 1.5px solid var(--kind-color);
    border-radius: 6px;
    background: transparent;
    color: var(--kind-color);
    font-size: var(--text-xs);
    cursor: pointer;
  }

  /* Neutral rather than kind-coloured: this axis is orthogonal to what the
     entry is, and colouring it the same way would suggest otherwise. */
  .commitment-btn {
    --kind-color: var(--text-secondary);
  }

  .fieldnote {
    margin: 6px 0 0;
    color: var(--text-secondary);
    font-size: 0.78em;
  }

  .google-export {
    margin-top: 18px;
    padding-top: 13px;
    border-top: 1px solid var(--card-border);
  }

  .kind-btn.selected {
    background: var(--kind-color);
    color: #fff;
  }

  input[type="text"],
  input[type="date"],
  input[type="time"],
  textarea {
    box-sizing: border-box;
    width: 100%;
    padding: 8px 10px;
    border: 1px solid var(--card-border);
    border-radius: 6px;
    background: var(--card-bg);
    color: var(--text-primary);
    font: inherit;
    font-size: 0.875rem;
  }

  textarea {
    resize: vertical;
  }

  .dates {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
  }

  .notice {
    margin: -5px 0 15px;
    padding: 9px 11px;
    border-radius: 7px;
    background: var(--surface);
    color: var(--text-secondary);
    font-size: var(--text-xs);
    line-height: 1.45;
  }

  .actions {
    display: flex;
    gap: 8px;
    margin-top: 18px;
  }

  .spacer {
    flex: 1;
  }

  .btn {
    padding: 8px 16px;
    border: none;
    border-radius: 8px;
    font-size: 0.875rem;
    cursor: pointer;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: wait;
  }

  .primary {
    background: var(--primary);
    color: #fff;
  }

  .secondary {
    background: var(--surface);
    color: var(--text-primary);
  }

  .danger {
    background: var(--danger);
    color: var(--text-inverse);
  }

  .error {
    margin: 0 0 12px;
    color: var(--danger);
    font-size: var(--text-sm);
  }

  @media (max-width: 560px) {
    .dates {
      grid-template-columns: 1fr;
      gap: 0;
    }
  }

  .location-links {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: 0.25rem;
  }

  .location-link {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    font-size: var(--text-2xs);
    color: var(--primary);
    text-decoration: none;
    padding: 0.1rem 0.35rem;
    border-radius: var(--radius-sm);
    background-color: var(--surface);
    border: 1px solid var(--card-border);
  }

  .location-link:hover {
    background-color: var(--primary-soft);
  }
</style>
