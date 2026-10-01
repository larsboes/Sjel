<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import ResearchTabs from "$lib/research/ResearchTabs.svelte";
  import { groupUpstreams, matches } from "$lib/research/content";
  import { registers } from "virtual:sjel-research";
  import { axonStatus } from "$lib/api";
  import { DEMO } from "$lib/demo";

  // Two registers, one page. systems.toml is what runs beside Sjel; upstreams.toml is every
  // project whose code or ideas Sjel took, or declined. Sjel replaces none of the first group.
  let query = $state("");

  const systems = $derived(registers.systems.filter((s) => matches(query, s.name, s.why, s.kind)));
  const groups = $derived(
    groupUpstreams(registers.upstreams.filter((u) => matches(query, u.name, u.summary, u.verdict, u.license))),
  );

  // Adding writes a `watch` row through sjel-status. The published demo has no backend, so the
  // form is absent there rather than failing on submit.
  const SUMMARY_MAX = 110;
  let addUrl = $state("");
  let addSummary = $state("");
  let addName = $state("");
  let adding = $state(false);
  let added = $state<string | null>(null);
  let addError = $state<string | null>(null);
  const summaryLength = $derived(addSummary.trim().length);

  async function addProject(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    adding = true;
    added = null;
    addError = null;
    try {
      const row = await axonStatus.watchUpstream(addUrl.trim(), addSummary.trim(), addName.trim() || undefined);
      added = row.name;
      addUrl = addSummary = addName = "";
    } catch (e) {
      addError = e instanceof Error ? e.message : String(e);
    } finally {
      adding = false;
    }
  }
</script>

<svelte:head><title>Projects · Sjel research</title></svelte:head>

<PageHeader
  badge="Research"
  title="Projects"
  desc="The software Sjel runs beside, builds on, learned from, is watching, or declined. Generated from systems.toml and upstreams.toml."
/>
<ResearchTabs current="projects" />

{#if !DEMO}
  <details class="card add">
    <summary><Icon name="plus" /> Add a project to watch</summary>
    <form onsubmit={addProject}>
      <label>
        URL
        <input type="url" required placeholder="https://github.com/owner/repo" bind:value={addUrl} />
      </label>
      <label>
        Summary
        <input type="text" required placeholder="What it is, and why it is worth a look" bind:value={addSummary} />
        <span class="counter" class:over={summaryLength > SUMMARY_MAX}>{summaryLength} / {SUMMARY_MAX}</span>
      </label>
      <label>
        Name <span class="optional">optional, from the URL when empty</span>
        <input type="text" pattern="[a-z0-9][a-z0-9\-]*" bind:value={addName} />
      </label>
      <button type="submit" disabled={adding || summaryLength > SUMMARY_MAX}>{adding ? "Adding…" : "Add"}</button>
    </form>
    {#if added}
      <p class="ok"><Icon name="check" /> Added <code>{added}</code> to upstreams.toml as <code>watch</code>. The list updates when the dev server reloads the file.</p>
    {/if}
    {#if addError}
      <p class="err"><Icon name="alert" /> {addError}</p>
    {/if}
  </details>
{/if}

<label class="search">
  <Icon name="search" />
  <input type="search" placeholder="Filter by name, summary, license…" bind:value={query} />
</label>

<section>
  <h2>Runs alongside <span class="count">{systems.length}</span></h2>
  <p class="blurb">Systems Sjel connects to and leaves in charge of their own job.</p>
  <ul class="rows">
    {#each systems as s (s.name)}
      <li class="card row">
        <div class="head">
          {#if s.url}
            <a href={s.url} target="_blank" rel="noreferrer">{s.name}</a>
          {:else}
            <span class="name">{s.name}</span>
          {/if}
          <span class="tag">{s.kind}</span>
          <span class="tag">{s.local ? "local" : "hosted"}</span>
        </div>
        <p>{s.why}</p>
      </li>
    {/each}
  </ul>
</section>

{#each groups as group (group.id)}
  {#if group.rows.length}
    <section>
      <h2>{group.label} <span class="count">{group.rows.length}</span></h2>
      {#if group.blurb}<p class="blurb">{group.blurb}</p>{/if}
      <ul class="rows">
        {#each group.rows as u (u.name)}
          <li class="card row">
            <div class="head">
              <a href={u.url} target="_blank" rel="noreferrer">{u.name}</a>
              <span class="tag">{u.verdict}</span>
              {#if u.license}<span class="tag">{u.license}</span>{/if}
            </div>
            <p>{u.summary}</p>
          </li>
        {/each}
      </ul>
    </section>
  {/if}
{/each}

<p class="source">
  The audit behind each verdict is the <code>why</code> field in
  <a href="https://github.com/larsboes/Sjel/blob/main/upstreams.toml" target="_blank" rel="noreferrer"
    ><code>upstreams.toml</code></a
  >.
</p>

<style>
  .add {
    max-width: 40rem;
    margin: 1rem 0 0;
    padding: 0.75rem 0.9rem;
  }
  .add summary {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    color: var(--text-primary);
    font-weight: 600;
    cursor: pointer;
  }
  .add form {
    display: grid;
    gap: 0.6rem;
    margin-top: 0.75rem;
  }
  .add label {
    display: grid;
    gap: 0.25rem;
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }
  .add input {
    padding: 0.45rem 0.6rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    background: var(--surface);
    color: var(--text-primary);
    font: inherit;
  }
  .optional,
  .counter {
    color: var(--text-secondary);
    font-size: var(--text-xs);
  }
  .counter.over {
    color: var(--danger, #c0392b);
  }
  .add button {
    justify-self: start;
    padding: 0.4rem 1rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    background: var(--primary);
    color: var(--on-primary, #fff);
    font: inherit;
    cursor: pointer;
  }
  .add button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .ok,
  .err {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin: 0.6rem 0 0;
    font-size: var(--text-sm);
  }
  .ok {
    color: var(--text-secondary);
  }
  .err {
    color: var(--danger, #c0392b);
  }
  .search {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    max-width: 28rem;
    margin: 1rem 0 1.5rem;
    padding: 0.5rem 0.75rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    background: var(--surface);
    color: var(--text-secondary);
  }
  .search input {
    flex: 1;
    border: 0;
    background: none;
    color: var(--text-primary);
    font: inherit;
    outline: none;
  }
  section {
    margin-bottom: 2rem;
  }
  h2 {
    margin: 0 0 0.25rem;
    font-size: var(--text-md);
    color: var(--text-primary);
  }
  .count,
  .blurb,
  .source {
    color: var(--text-secondary);
    font-weight: 400;
  }
  .blurb {
    margin: 0 0 0.75rem;
    font-size: var(--text-base);
  }
  .rows {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(18rem, 1fr));
    gap: 0.6rem;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    padding: 0.75rem 0.9rem;
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.4rem;
  }
  .head a,
  .name {
    margin-right: auto;
    color: var(--text-primary);
    font-weight: 600;
  }
  .tag {
    padding: 0 0.4rem;
    border: 1px solid var(--card-border);
    border-radius: 999px;
    color: var(--text-secondary);
    font-size: var(--text-xs);
  }
  .row p {
    margin: 0.35rem 0 0;
    color: var(--text-secondary);
    font-size: var(--text-sm);
    line-height: 1.5;
  }
  .source {
    font-size: var(--text-sm);
  }
  .source a {
    color: var(--primary);
  }
</style>
