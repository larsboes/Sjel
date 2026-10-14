<script lang="ts">
  /** Profiles, priority, id and source of one item. */
  import type { ItemMeta } from "./model";

  let { item = $bindable(), profiles }: { item: ItemMeta; profiles: string[] } = $props();

  const chosen = $derived(item.profiles ?? []);
  function toggle(p: string): void {
    item.profiles = chosen.includes(p) ? chosen.filter((x) => x !== p) : [...chosen, p];
  }

  /** The level outside any per-profile override. */
  const base = $derived(typeof item.priority === "object" ? (item.priority.default ?? 1) : (item.priority ?? 1));
  const overrides = $derived(
    typeof item.priority === "object" ? Object.entries(item.priority).filter(([k]) => k !== "default") : [],
  );
  function setPriority(baseLevel: number, list: [string, number][]): void {
    if (list.length === 0) item.priority = baseLevel === 1 ? undefined : baseLevel;
    else item.priority = { default: baseLevel, ...Object.fromEntries(list) };
  }
</script>

<div class="meta">
  <span class="lbl">Profiles</span>
  <div class="chips">
    {#each profiles.filter((p) => p !== "default") as p (p)}
      <button class="chip" class:on={chosen.includes(p)} onclick={() => toggle(p)}>{p}</button>
    {/each}
    {#if chosen.length === 0}<span class="hint">all profiles</span>{/if}
  </div>
  <span class="lbl">Priority</span>
  <div class="chips">
    <select aria-label="Priority" value={base} onchange={(e) => setPriority(+e.currentTarget.value, overrides)}>
      <option value={1}>1 keep</option><option value={2}>2</option><option value={3}>3 drop first</option>
    </select>
    {#each overrides as [p, lvl], i (p)}
      <span class="ov">
        {p}
        <select aria-label="Priority in {p}" value={lvl}
          onchange={(e) => setPriority(base, overrides.map((o, j) => (j === i ? [p, +e.currentTarget.value] : o)))}>
          <option value={1}>1</option><option value={2}>2</option><option value={3}>3</option>
        </select>
        <button class="x" aria-label="Remove override" onclick={() => setPriority(base, overrides.filter((_, j) => j !== i))}>×</button>
      </span>
    {/each}
    <select aria-label="Add per-profile priority" value=""
      onchange={(e) => { const p = e.currentTarget.value; if (p) setPriority(base, [...overrides, [p, base]]); e.currentTarget.value = ""; }}>
      <option value="">+ per profile</option>
      {#each profiles.filter((p) => p !== "default" && !overrides.some(([o]) => o === p)) as p (p)}<option>{p}</option>{/each}
    </select>
  </div>
  <span class="lbl">Id · source</span>
  <div class="chips">
    <input class="id" aria-label="Id" placeholder="id (for pin/hide)" bind:value={() => item.id ?? "", (v) => (item.id = v)} />
    <input class="src" aria-label="Source" placeholder="source of this fact" bind:value={() => item.source ?? "", (v) => (item.source = v)} />
  </div>
</div>

<style>
  .meta { display: grid; grid-template-columns: 7rem 1fr; gap: 0.25rem 0.3rem; margin: 0.3rem 0 0; font-size: var(--text-xs); }
  .lbl { color: var(--text-tertiary); padding-top: 0.15rem; }
  .chips { display: flex; flex-wrap: wrap; gap: 0.25rem; align-items: center; }
  .chip { font-size: 0.68rem; padding: 0.05rem 0.4rem; border-radius: var(--radius-sm); border: 1px solid var(--card-border); background: none; color: var(--text-secondary); cursor: pointer; }
  .chip.on { background: var(--success-soft); color: var(--success); border-color: transparent; }
  .ov { display: inline-flex; gap: 0.2rem; align-items: center; }
  .x { border: 0; background: none; cursor: pointer; color: var(--text-tertiary); }
  .hint { color: var(--text-tertiary); }
  .id { width: 9rem; }
  .src { flex: 1; min-width: 12rem; }
</style>
