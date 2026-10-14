<script lang="ts">
  /** A `Text`: one string for both languages, or split into EN and DE. */
  import type { Localized, Text } from "./model";
  import LocField from "./LocField.svelte";

  let { value = $bindable(), label }: { value: Text | undefined; label: string } = $props();

  const split = () => (value = { en: typeof value === "string" ? value : "", de: "" });
  const join = () => (value = typeof value === "object" ? value.en : (value ?? ""));
</script>

{#if typeof value === "object"}
  <div class="row">
    <div class="grow"><LocField bind:value={value as Localized} {label} /></div>
    <button class="btn btn-outline" title="Same text in both languages" onclick={join}>1 lang</button>
  </div>
{:else}
  <div class="row">
    <span class="lbl">{label}</span>
    <input class="grow" aria-label={label} bind:value={() => value ?? "", (v) => (value = v)} />
    <button class="btn btn-outline" title="Separate EN and DE text" onclick={split}>EN/DE</button>
  </div>
{/if}

<style>
  .row { display: flex; gap: 0.3rem; align-items: start; margin: 0.2rem 0; }
  .grow { flex: 1; }
  .lbl { width: 7rem; flex: none; font-size: var(--text-xs); color: var(--text-secondary); padding-top: 0.2rem; }
</style>
