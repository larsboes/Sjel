<script lang="ts">
  import { onMount } from "svelte";
  import { modal } from "$lib/modal";
  import { KINDS } from "./types";
  import type { CalendarNewRhythm } from "$lib/api";

  let {
    onSave,
    onClose,
  }: {
    onSave: (data: CalendarNewRhythm) => Promise<void>;
    onClose: () => void;
  } = $props();

  let kind = $state("work_onsite");
  let title = $state("");
  let location = $state("");
  let weekdays = $state<string[]>(["tu", "we", "th"]);
  let allDay = $state(true);
  let startTime = $state("09:00");
  let endTime = $state("17:00");
  let validFrom = $state("");
  let validUntil = $state("");
  let saving = $state(false);
  let error = $state("");

  const WEEKDAY_OPTS: Array<{ value: string; label: string }> = [
    { value: "mo", label: "Mo" },
    { value: "tu", label: "Di" },
    { value: "we", label: "Mi" },
    { value: "th", label: "Do" },
    { value: "fr", label: "Fr" },
    { value: "sa", label: "Sa" },
    { value: "su", label: "So" },
  ];

  function toggleWd(val: string) {
    if (weekdays.includes(val)) {
      weekdays = weekdays.filter((d) => d !== val);
    } else {
      weekdays = [...weekdays, val];
    }
  }

  function todayString(): string {
    const d = new Date();
    return d.toISOString().slice(0, 10);
  }

  function nextMonthString(): string {
    const d = new Date();
    d.setMonth(d.getMonth() + 1);
    return d.toISOString().slice(0, 10);
  }

  onMount(() => {
    if (!validFrom) validFrom = todayString();
    if (!validUntil) validUntil = nextMonthString();
  });

  async function save() {
    if (!title.trim()) { error = "Title is required"; return; }
    if (weekdays.length === 0) { error = "Select at least one weekday"; return; }
    if (!validFrom || !validUntil) { error = "Validity period is required"; return; }
    error = "";
    saving = true;
    try {
      await onSave({
        kind,
        title: title.trim(),
        location: location.trim() || null,
        byweekday: weekdays,
        start_time: allDay ? null : startTime,
        end_time: allDay ? null : endTime,
        valid_from: validFrom,
        valid_until: validUntil,
      });
    } catch (e) {
      error = String(e);
    } finally {
      saving = false;
    }
  }
</script>

<div class="overlay">
  <button class="backdrop" aria-label="Close dialog" onclick={onClose}></button>
  <div
    class="sheet"
    use:modal={{ onClose, canClose: () => !saving }}
    role="dialog"
    aria-modal="true"
    aria-label="New rhythm"
    tabindex="-1"
  >
    <h2>New rhythm</h2>

    {#if error}
      <p class="error">{error}</p>
    {/if}

    <label>
      <span>Type</span>
      <select bind:value={kind}>
        {#each KINDS as k}
          <option value={k.value}>{k.label}</option>
        {/each}
      </select>
    </label>

    <label>
      <span>Title</span>
      <input type="text" bind:value={title} placeholder="e.g. office days in Bonn" />
    </label>

    <fieldset>
      <legend>Weekdays</legend>
      <div class="wd-picker">
        {#each WEEKDAY_OPTS as wd}
          <button
            class="wd-btn"
            class:selected={weekdays.includes(wd.value)}
            onclick={() => toggleWd(wd.value)}
          >
            {wd.label}
          </button>
        {/each}
      </div>
    </fieldset>

    <label>
      <span>Location</span>
      <input type="text" bind:value={location} placeholder="Optional" />
    </label>

    <label>
      <span>All day</span>
      <input type="checkbox" bind:checked={allDay} />
    </label>

    {#if !allDay}
      <div class="times">
        <label>
          <span>From</span>
          <input type="time" bind:value={startTime} />
        </label>
        <label>
          <span>To</span>
          <input type="time" bind:value={endTime} />
        </label>
      </div>
    {/if}

    <div class="dates">
      <label>
        <span>Valid from</span>
        <input type="date" bind:value={validFrom} />
      </label>
      <label>
        <span>until</span>
        <input type="date" bind:value={validUntil} />
      </label>
    </div>

    <div class="actions">
      <button class="btn secondary" onclick={onClose}>Cancel</button>
      <button class="btn primary" onclick={save} disabled={saving}>
        {saving ? "Creating..." : "Create rhythm"}
      </button>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
    padding: 20px;
  }

  .backdrop {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    border: 0;
    background: rgba(0,0,0,0.48);
    cursor: default;
    animation: fadeIn var(--motion-fast) ease-out;
  }

  .sheet {
    position: relative;
    background: var(--card-bg);
    border-radius: 12px;
    padding: 24px;
    width: min(480px, 90vw);
    max-height: 90vh;
    overflow-y: auto;
    box-shadow: 0 8px 32px rgba(0,0,0,0.35);
  }

  .sheet:focus { outline: none; }

  h2 { margin: 0 0 16px; font-size: 1.125rem; }

  label { display: flex; flex-direction: column; gap: 4px; margin-bottom: 12px; font-size: var(--text-sm); font-weight: 600; color: var(--text-secondary); }

  fieldset { min-width: 0; margin: 0 0 12px; padding: 0; border: 0; }
  legend { margin-bottom: 4px; padding: 0; font-size: var(--text-sm); font-weight: 600; color: var(--text-secondary); }

  select, input[type="text"], input[type="date"], input[type="time"] {
    padding: 8px 10px;
    border-radius: 6px;
    border: 1px solid var(--card-border);
    background: var(--card-bg);
    color: var(--text-primary);
    font-size: 0.875rem;
  }

  .wd-picker { display: flex; gap: 4px; }

  .wd-btn {
    width: 38px; height: 38px;
    border-radius: 8px;
    border: 1.5px solid var(--card-border);
    background: transparent;
    color: var(--text-primary);
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
    transition: background-color var(--motion-fast), border-color var(--motion-fast), color var(--motion-fast);
  }

  .wd-btn.selected { background: var(--primary); color: #fff; border-color: var(--primary); }

  .times { display: flex; gap: 12px; }
  .times label { flex: 1; }
  .dates { display: flex; gap: 12px; }
  .dates label { flex: 1; }

  .actions { display: flex; gap: 8px; justify-content: flex-end; margin-top: 16px; }

  .btn {
    padding: 8px 16px;
    border-radius: 8px;
    border: none;
    font-size: 0.875rem;
    cursor: pointer;
    transition: opacity var(--motion-fast);
  }
  .btn:disabled { opacity: 0.5; }
  .primary { background: var(--primary); color: #fff; }
  .secondary { background: var(--surface); color: var(--text-primary); }

  .error { color: var(--danger); font-size: var(--text-sm); margin-bottom: 8px; }

  @keyframes fadeIn { from { opacity: 0; } to { opacity: 1; } }
</style>
