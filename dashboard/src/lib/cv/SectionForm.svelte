<script lang="ts">
  /** One free section: title, intro, label/meta/detail entries, bullets and prose. */
  import type { Section } from "./model";
  import ItemMeta from "./ItemMeta.svelte";
  import ListTools from "./ListTools.svelte";
  import LocField from "./LocField.svelte";
  import TextField from "./TextField.svelte";

  let { section = $bindable(), profiles }: { section: Section; profiles: string[] } = $props();
</script>

<LocField bind:value={section.title} label="Title" />
<ItemMeta bind:item={section} {profiles} />

<h5>Intro</h5>
{#if section.intro}
  <div class="item">
    <LocField bind:value={section.intro} label="Intro" multiline />
    <ItemMeta bind:item={section.intro} {profiles} />
    <button class="btn btn-outline" onclick={() => (section.intro = undefined)}>Remove intro</button>
  </div>
{:else}
  <button class="btn btn-outline" onclick={() => (section.intro = { en: "" })}>+ Intro</button>
{/if}

<h5>Entries <span class="hint">label · right column · detail line</span></h5>
{#each section.entries ?? [] as _, i (i)}
  {@const entries = section.entries ?? []}
  <div class="item">
    <div class="head"><strong>{typeof entries[i].label === "string" ? entries[i].label : entries[i].label.en}</strong>
      <ListTools bind:list={() => section.entries ?? [], (v) => (section.entries = v as typeof entries)} index={i} what="entry" />
    </div>
    <TextField bind:value={entries[i].label} label="Label" />
    <TextField bind:value={entries[i].meta} label="Right column" />
    {#if entries[i].detail}
      <LocField bind:value={entries[i].detail!} label="Detail" />
    {:else}
      <button class="btn btn-outline" onclick={() => (entries[i].detail = { en: "" })}>+ Detail</button>
    {/if}
    <ItemMeta bind:item={entries[i]} {profiles} />
  </div>
{/each}
<button class="btn btn-outline" onclick={() => (section.entries = [...(section.entries ?? []), { label: "", priority: 2 }])}>+ Entry</button>

<h5>Bullets <span class="hint">bold label, then text</span></h5>
{#each section.bullets ?? [] as _, i (i)}
  {@const bullets = section.bullets ?? []}
  <div class="item">
    <div class="head"><span class="hint">Bullet {i + 1}</span>
      <ListTools bind:list={() => section.bullets ?? [], (v) => (section.bullets = v as typeof bullets)} index={i} what="bullet" />
    </div>
    <TextField bind:value={bullets[i].label} label="Bold label" />
    <LocField bind:value={bullets[i].text} label="Text" />
    <ItemMeta bind:item={bullets[i]} {profiles} />
  </div>
{/each}
<button class="btn btn-outline" onclick={() => (section.bullets = [...(section.bullets ?? []), { text: { en: "" } }])}>+ Bullet</button>

<h5>Prose</h5>
{#if section.prose}
  <LocField bind:value={section.prose} label="Paragraph" multiline />
  <button class="btn btn-outline" onclick={() => (section.prose = undefined)}>Remove paragraph</button>
{:else}
  <button class="btn btn-outline" onclick={() => (section.prose = { en: "" })}>+ Paragraph</button>
{/if}

<style>
  h5 { font-size: var(--text-xs); margin: 0.7rem 0 0.25rem; color: var(--text-secondary); }
  .item { border-left: 2px solid var(--card-border); padding: 0.2rem 0 0.4rem 0.6rem; margin: 0.4rem 0; }
  .head { display: flex; align-items: center; gap: 0.4rem; font-size: 0.78rem; }
  .hint { font-weight: normal; color: var(--text-tertiary); font-size: var(--text-xs); }
</style>
