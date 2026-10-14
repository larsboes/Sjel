<script lang="ts">
  /** English and German text side by side. A blank German field renders the English text. */
  import type { Localized } from "./model";

  let { value = $bindable(), label, multiline = false }: {
    value: Localized;
    label: string;
    multiline?: boolean;
  } = $props();
</script>

<div class="loc">
  <span class="lbl">{label}</span>
  {#if multiline}
    <textarea aria-label="{label} EN" placeholder="EN" bind:value={value.en}></textarea>
    <textarea aria-label="{label} DE" placeholder="DE (blank = EN)" bind:value={() => value.de ?? "", (v) => (value.de = v)}></textarea>
  {:else}
    <input aria-label="{label} EN" placeholder="EN" bind:value={value.en} />
    <input aria-label="{label} DE" placeholder="DE (blank = EN)" bind:value={() => value.de ?? "", (v) => (value.de = v)} />
  {/if}
</div>

<style>
  .loc { display: grid; grid-template-columns: 7rem 1fr 1fr; gap: 0.3rem; align-items: start; margin: 0.2rem 0; }
  .lbl { font-size: var(--text-xs); color: var(--text-secondary); padding-top: 0.2rem; }
  textarea { min-height: 3.6rem; resize: vertical; }
</style>
