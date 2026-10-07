<script lang="ts" module>
  /**
   * A tone says what the value means, never decoration: success = settled (booked, done),
   * warning = needs a decision, danger = broken, accent = chosen, muted = not active.
   * Map a value to a tone where the data is defined, not at the call site.
   */
  export type Tone = "neutral" | "accent" | "success" | "warning" | "danger" | "muted";
</script>

<script lang="ts">
  import { tip } from "$lib/tip";

  let { label, tone = "neutral", hint }: { label: string; tone?: Tone; hint?: string } = $props();
</script>

{#if hint}
  <span class="chip {tone}" use:tip={hint}>{label}</span>
{:else}
  <span class="chip {tone}">{label}</span>
{/if}

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    height: 1.25rem;
    padding: 0 0.4rem;
    border-radius: var(--radius-sm);
    font-size: var(--text-2xs);
    line-height: 1;
    white-space: nowrap;
    border: 1px solid transparent;
    background: var(--card-bg);
    color: var(--text-secondary);
  }

  .accent {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .success {
    background: var(--success-soft);
    color: var(--success);
  }

  .warning {
    background: var(--warning-soft);
    color: var(--warning-ink);
  }

  .danger {
    background: var(--danger-soft);
    color: var(--danger);
  }

  .muted {
    background: none;
    border-color: var(--card-border);
    color: var(--text-tertiary);
  }
</style>
