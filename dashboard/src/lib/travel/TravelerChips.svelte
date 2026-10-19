<script lang="ts">
  import type { TripPlan } from "$lib/api";
  import Chip from "$lib/ui/Chip.svelte";
  import { tip } from "$lib/tip";
  import { inspectorStore } from "$lib/inspector/inspector.svelte";

  let { plan }: { plan: Pick<TripPlan, "travelers" | "traveler_links"> } = $props();
</script>

{#each plan.travelers as name, index (index)}
  {@const decision = plan.traveler_links?.[index]}
  {#if decision?.name === name && decision.status === "resolved" && decision.entity_id && /^ent:[^:]+$/.test(decision.entity_id)}
    <button
      type="button"
      class="traveler-chip"
      use:tip={`Inspect ${name}`}
      onclick={() => inspectorStore.inspectPerson({ id: decision.entity_id!, name })}
    ><Chip label={name} /></button>
  {:else}
    <Chip label={name} hint={decision?.status === "not_person" ? "Not a person" : "Person link not resolved"} />
  {/if}
{/each}

<style>
  .traveler-chip {
    display: inline-flex;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    cursor: pointer;
  }

  .traveler-chip:hover {
    outline: 1px solid var(--accent);
  }

  .traveler-chip:focus-visible {
    box-shadow: var(--focus-ring);
  }
</style>
