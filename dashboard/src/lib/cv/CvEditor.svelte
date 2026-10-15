<script lang="ts">
  /**
   * Edit the master or one application, as a form or as raw YAML. Both save through `cv`, which
   * refuses what does not parse or render and commits what it accepts in the overlay's git.
   */
  import { axonStatus, type CvView } from "$lib/api";
  import Chip from "$lib/ui/Chip.svelte";
  import ApplicationForm from "./ApplicationForm.svelte";
  import MasterForm from "./MasterForm.svelte";
  import { clean, itemIds, type Application, type Master } from "./model";

  let { view, onsaved }: { view: CvView; onsaved: () => void } = $props();

  /** "" is the master, otherwise an application slug; null before anything is open. */
  let file = $state<string | null>(null);
  let tab = $state<"form" | "yaml">("form");
  let model = $state<Master | Application | null>(null);
  let yaml = $state("");
  let snapshot = $state("");
  let master = $state<Master | null>(null);
  let busy = $state(false);
  let note = $state<string | null>(null);
  let failed = $state(false);

  const current = () => (tab === "form" ? JSON.stringify(model) : yaml);
  const dirty = $derived(file !== null && current() !== snapshot);

  async function open(slug: string, as = tab): Promise<void> {
    if (dirty && !confirm("Discard unsaved changes?")) return;
    note = null;
    try {
      if (as === "form") {
        const loaded = await axonStatus.cvModel<Master | Application>(slug);
        // Empty overrides exist in the form so they can be typed into; clean() drops them on save.
        if (slug !== "") {
          const a = loaded as Application;
          a.title ??= { en: "" };
          a.summary ??= { en: "" };
        }
        model = loaded;
        if (slug !== "") master ??= await axonStatus.cvModel<Master>("");
        else master = model as Master;
      } else {
        yaml = (await axonStatus.cvFile(slug)).content;
      }
      file = slug;
      tab = as;
      snapshot = current();
    } catch (e) {
      failed = true;
      note = e instanceof Error ? e.message : String(e);
    }
  }

  async function save(): Promise<void> {
    if (file === null) return;
    busy = true;
    note = null;
    try {
      const result = tab === "form"
        ? await axonStatus.saveCvModel(file, clean($state.snapshot(model)))
        : await axonStatus.saveCvFile(file, yaml);
      failed = false;
      note = result.detail;
      await open(file, tab);
      onsaved();
    } catch (e) {
      failed = true;
      note = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="bar">
  <select aria-label="File to edit" value={file ?? "-"} onchange={(e) => open(e.currentTarget.value)}>
    {#if file === null}<option value="-" disabled>Choose a file…</option>{/if}
    <option value="">Master CV</option>
    {#each view.applications as a (a.slug)}<option value={a.slug}>Application: {a.company} · {a.role}</option>{/each}
  </select>
  {#if file !== null}
    <span class="tabs">
      <button class:on={tab === "form"} onclick={() => open(file!, "form")}>Form</button>
      <button class:on={tab === "yaml"} onclick={() => open(file!, "yaml")}>YAML</button>
    </span>
    <button class="btn btn-soft" disabled={busy || !dirty} onclick={save}>Save</button>
    <button class="btn btn-outline" disabled={busy || !dirty} onclick={() => open(file!, tab)}>Revert</button>
    {#if dirty}<Chip label="unsaved" tone="warning" />{/if}
  {:else}
    <button class="btn btn-soft" onclick={() => open("")}>Open master</button>
  {/if}
</div>
{#if note}<pre class="note" class:fail={failed}>{note}</pre>{/if}

{#if file !== null && tab === "form" && model}
  <div class="form">
    {#if file === ""}
      <MasterForm bind:master={model as Master} profiles={view.profiles} />
    {:else}
      <ApplicationForm bind:app={model as Application} profiles={view.profiles} ids={master ? itemIds(master) : []} />
    {/if}
  </div>
{:else if file !== null && tab === "yaml"}
  <p class="hint">Fields: <code>capabilities/cv/master_cv.schema.yaml</code>.</p>
  <textarea aria-label="YAML" spellcheck="false" bind:value={yaml}></textarea>
{/if}

<style>
  .bar { display: flex; flex-wrap: wrap; gap: 0.35rem; align-items: center; }
  .tabs { display: inline-flex; border: 1px solid var(--card-border); border-radius: var(--radius-sm); overflow: hidden; }
  .tabs button { border: 0; background: none; padding: 0.15rem 0.6rem; font-size: 0.72rem; cursor: pointer; color: var(--text-secondary); }
  .tabs button.on { background: var(--input-bg); color: var(--text-primary); font-weight: 600; }
  .note { font-size: 0.72rem; white-space: pre-wrap; margin: 0.4rem 0 0; }
  .note.fail { color: var(--warning-ink); }
  .form { margin: 0.5rem 0 0; }
  .hint { font-size: var(--text-xs); color: var(--text-secondary); margin: 0.4rem 0 0; }
  textarea { width: 100%; min-height: 60vh; margin: 0.4rem 0 0; font-family: var(--font-mono); font-size: 0.72rem; line-height: 1.45; tab-size: 2; }
  .form :global(input), .form :global(select), .form :global(textarea), .bar select {
    font-size: var(--text-xs); padding: 0.15rem 0.35rem; background: var(--input-bg); color: var(--text-primary);
    border: 1px solid var(--card-border); border-radius: var(--radius-sm);
  }
</style>
