<script lang="ts">
  /** Move up, move down and remove for item `index` of `list`. */
  let { list = $bindable(), index, what }: { list: unknown[]; index: number; what: string } = $props();

  function move(by: number): void {
    const to = index + by;
    if (to < 0 || to >= list.length) return;
    const next = [...list];
    [next[index], next[to]] = [next[to], next[index]];
    list = next;
  }
</script>

<span class="tools">
  <button aria-label="Move {what} up" disabled={index === 0} onclick={() => move(-1)}>↑</button>
  <button aria-label="Move {what} down" disabled={index === list.length - 1} onclick={() => move(1)}>↓</button>
  <button aria-label="Remove {what}" onclick={() => confirm(`Remove this ${what}?`) && (list = list.filter((_, i) => i !== index))}>×</button>
</span>

<style>
  .tools { display: inline-flex; gap: 0.15rem; margin-left: auto; }
  button { border: 1px solid var(--card-border); background: none; border-radius: var(--radius-sm); cursor: pointer; color: var(--text-secondary); font-size: 0.7rem; padding: 0 0.35rem; }
  button:disabled { opacity: 0.35; cursor: default; }
</style>
