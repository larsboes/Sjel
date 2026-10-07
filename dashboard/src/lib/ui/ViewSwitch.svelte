<script lang="ts" generics="V extends string">
  // Several views over one dataset (table, board, timeline), Notion's database pattern.
  import type { ComponentProps } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import { tip } from "$lib/tip";

  let {
    views,
    value = $bindable(),
    label = "View",
  }: {
    views: { id: V; label: string; icon: ComponentProps<typeof Icon>["name"] }[];
    value: V;
    label?: string;
  } = $props();
</script>

<div class="switch" role="radiogroup" aria-label={label}>
  {#each views as view (view.id)}
    <button
      type="button"
      role="radio"
      aria-checked={value === view.id}
      class:active={value === view.id}
      onclick={() => (value = view.id)}
      use:tip={view.label}
    >
      <Icon name={view.icon} size={13} />
      <span>{view.label}</span>
    </button>
  {/each}
</div>

<style>
  .switch {
    display: inline-flex;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    padding: 1px;
    gap: 1px;
  }

  button {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    height: 1.5rem;
    padding: 0 0.5rem;
    border: 0;
    border-radius: calc(var(--radius-sm) - 1px);
    background: none;
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    cursor: pointer;
  }

  button:hover {
    color: var(--text-primary);
  }

  button.active {
    background: var(--card-bg);
    color: var(--text-primary);
  }

  button:focus-visible {
    outline: 2px solid var(--focus-ring);
  }

  @media (max-width: 640px) {
    span {
      display: none;
    }
  }
</style>
