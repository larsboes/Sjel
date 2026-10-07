<script lang="ts">
  // One property of a record, edited in place: it reads as text until clicked, then becomes
  // the field, and commits on Enter, blur or a choice. Escape drops the edit. Notion's
  // database property, so a record needs no separate edit form.
  import { tip } from "$lib/tip";

  type Option = { value: string; label: string };

  let {
    label,
    value,
    kind = "text",
    options = [],
    placeholder = "Empty",
    hint,
    readonly = false,
    onCommit,
  }: {
    label: string;
    value: string | null;
    kind?: "text" | "textarea" | "select" | "time";
    options?: Option[];
    placeholder?: string;
    hint?: string;
    readonly?: boolean;
    onCommit?: (value: string | null) => void;
  } = $props();

  let editing = $state(false);
  let field: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement | undefined = $state();

  const shown = $derived(
    kind === "select" ? (options.find((o) => o.value === (value ?? ""))?.label ?? value) : value,
  );

  function commit(next: string) {
    editing = false;
    const clean = next.trim() || null;
    if (clean !== value) onCommit?.(clean);
  }

  $effect(() => {
    if (editing) field?.focus();
  });
</script>

<div class="property">
  {#if hint}
    <span class="label" use:tip={hint}>{label}</span>
  {:else}
    <span class="label">{label}</span>
  {/if}
  {#if editing && kind === "select"}
    <select
      bind:this={field}
      aria-label={label}
      value={value ?? ""}
      onchange={(e) => commit(e.currentTarget.value)}
      onblur={() => (editing = false)}
      onkeydown={(e) => e.key === "Escape" && (editing = false)}
    >
      {#each options as option (option.value)}<option value={option.value}>{option.label}</option>{/each}
    </select>
  {:else if editing && kind === "textarea"}
    <textarea
      bind:this={field}
      aria-label={label}
      rows="3"
      value={value ?? ""}
      onblur={(e) => commit(e.currentTarget.value)}
      onkeydown={(e) => e.key === "Escape" && (editing = false)}
    ></textarea>
  {:else if editing}
    <input
      bind:this={field}
      aria-label={label}
      type={kind === "time" ? "time" : "text"}
      value={value ?? ""}
      onblur={(e) => commit(e.currentTarget.value)}
      onkeydown={(e) => {
        if (e.key === "Enter") commit(e.currentTarget.value);
        if (e.key === "Escape") editing = false;
      }}
    />
  {:else if readonly}
    <span class="value" class:empty={!shown}>{shown || placeholder}</span>
  {:else}
    <button type="button" class="value" class:empty={!shown} class:multiline={kind === "textarea"} onclick={() => (editing = true)}>
      {shown || placeholder}
    </button>
  {/if}
</div>

<style>
  .property {
    display: grid;
    grid-template-columns: 7rem 1fr;
    align-items: start;
    gap: var(--space-2);
    min-height: 1.75rem;
  }

  .label {
    padding-top: 0.3rem;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .value,
  input,
  select,
  textarea {
    font: inherit;
    font-size: var(--text-xs);
    color: var(--text-primary);
    text-align: left;
    width: 100%;
    min-width: 0;
    padding: 0.25rem 0.4rem;
    border-radius: var(--radius-sm);
    border: 1px solid transparent;
    background: none;
  }

  button.value {
    cursor: text;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  button.value.multiline {
    white-space: pre-wrap;
    line-height: var(--leading-normal);
  }

  button.value:hover {
    background: var(--nav-hover);
  }

  .empty {
    color: var(--text-tertiary);
  }

  input,
  select,
  textarea {
    background: var(--input-bg);
    border-color: var(--input-border);
  }

  textarea {
    resize: vertical;
  }

  input:focus,
  select:focus,
  textarea:focus {
    outline: 2px solid var(--focus-ring);
    outline-offset: -1px;
  }
</style>
