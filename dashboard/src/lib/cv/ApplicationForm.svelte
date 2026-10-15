<script lang="ts">
  /** One application: what to build for it, overrides, pin and hide, and notes. */
  import type { Application } from "./model";
  import LocField from "./LocField.svelte";

  let { app = $bindable(), profiles, ids }: { app: Application; profiles: string[]; ids: string[] } = $props();

  function toggle(key: "pin" | "hide", id: string): void {
    const list = app[key] ?? [];
    app[key] = list.includes(id) ? list.filter((x) => x !== id) : [...list, id];
    const other = key === "pin" ? "hide" : "pin";
    if (app[key]!.includes(id)) app[other] = (app[other] ?? []).filter((x) => x !== id);
  }
  const state = (id: string) => (app.pin?.includes(id) ? "pin" : app.hide?.includes(id) ? "hide" : "auto");
</script>

<div class="grid">
  <label>Company <input bind:value={app.company} /></label>
  <label>Role <input bind:value={app.role} /></label>
  <label>Posting URL <input bind:value={app.url} /></label>
  <label>Profile <select bind:value={app.profile}>{#each profiles as p (p)}<option>{p}</option>{/each}</select></label>
  <label>Language <select bind:value={app.lang}><option>en</option><option>de</option></select></label>
</div>

<h5>Overrides <span class="hint">blank = the profile's own text</span></h5>
<LocField bind:value={app.title!} label="Title" />
<LocField bind:value={app.summary!} label="Summary" multiline />

<h5>Pin and hide <span class="hint">pin = always in, hide = never in, auto = by priority</span></h5>
<div class="ids">
  {#each ids as id (id)}
    {@const s = state(id)}
    <span class="id {s}">
      {id}
      <button class:on={s === "pin"} onclick={() => toggle("pin", id)}>pin</button>
      <button class:on={s === "hide"} onclick={() => toggle("hide", id)}>hide</button>
    </span>
  {:else}
    <span class="hint">No item in the master has an id yet. Give one in the master form.</span>
  {/each}
</div>

<h5>Notes <span class="hint">fit, gaps, contacts, interview preparation</span></h5>
<textarea aria-label="Notes" bind:value={() => app.notes ?? "", (v) => (app.notes = v)}></textarea>

<h5>Status log</h5>
<ul class="log">
  {#each app.log as entry, i (i)}<li>{entry.date} · {entry.status}{entry.note ? ` · ${entry.note}` : ""}</li>{/each}
</ul>

<style>
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(12rem, 1fr)); gap: 0.35rem; }
  label { display: flex; flex-direction: column; gap: 0.1rem; font-size: var(--text-xs); color: var(--text-secondary); }
  h5 { font-size: var(--text-xs); margin: 0.7rem 0 0.25rem; color: var(--text-secondary); }
  .hint { font-weight: normal; color: var(--text-tertiary); font-size: var(--text-xs); }
  .ids { display: flex; flex-wrap: wrap; gap: 0.3rem; font-size: 0.7rem; }
  .id { display: inline-flex; gap: 0.2rem; align-items: center; padding: 0.05rem 0.3rem; border: 1px solid var(--card-border); border-radius: var(--radius-sm); }
  .id.pin { border-color: var(--success); }
  .id.hide { opacity: 0.55; text-decoration: line-through; }
  .id button { font-size: 0.62rem; border: 0; background: none; cursor: pointer; color: var(--text-tertiary); }
  .id button.on { color: var(--text-primary); font-weight: 600; }
  textarea { width: 100%; min-height: 6rem; }
  .log { font-size: var(--text-xs); color: var(--text-secondary); margin: 0; padding-left: 1rem; }
</style>
