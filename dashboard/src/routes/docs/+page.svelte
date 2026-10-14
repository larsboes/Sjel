<script lang="ts">
  import PageHeader from "$lib/PageHeader.svelte";
  import Icon from "$lib/Icon.svelte";
  import { link } from "$lib/nav";
  import { entries } from "virtual:sjel-docs";
  import { toDocEntry, groupDocs, type DocEntry } from "$lib/docs/content";

  let filter = $state("");

  const allDocs = $derived(entries.map(toDocEntry));
  const filtered = $derived(
    filter.trim()
      ? allDocs.filter(
          (d) =>
            d.title.toLowerCase().includes(filter.toLowerCase()) ||
            d.summary.toLowerCase().includes(filter.toLowerCase()) ||
            d.slug.toLowerCase().includes(filter.toLowerCase()),
        )
      : allDocs,
  );

  const sections = $derived(groupDocs(filtered));
</script>

<svelte:head>
  <title>Documentation · Sjel</title>
</svelte:head>

<PageHeader
  badge="Technical Manual"
  title="Documentation"
  desc="Architecture, capability contracts, and operational doctrines for the Sjel operating environment."
/>

<div class="search-bar">
  <Icon name="search" size={16} />
  <input
    type="text"
    placeholder="Search specifications, contracts, architecture..."
    bind:value={filter}
    aria-label="Filter documentation"
  />
  {#if filter}
    <button class="clear" onclick={() => (filter = "")} aria-label="Clear filter">
      <Icon name="close" size={14} />
    </button>
  {/if}
</div>

<div class="doc-sections">
  {#each sections as section (section.id)}
    <section class="doc-section">
      <div class="section-header">
        <h2>{section.label}</h2>
        {#if section.description}
          <p class="section-desc">{section.description}</p>
        {/if}
      </div>

      <div class="card-grid">
        {#each section.docs as doc (doc.slug)}
          <a class="doc-card card" href={link(doc.slug === "index" ? "/docs/README" : `/docs/${doc.slug}`)}>
            <div class="card-top">
              <span class="category-tag">{doc.category}</span>
              <span class="file-path">{doc.file}</span>
            </div>
            <h3>{doc.title}</h3>
            {#if doc.summary}
              <p class="summary">{doc.summary}</p>
            {/if}
          </a>
        {/each}
      </div>
    </section>
  {/each}

  {#if sections.length === 0}
    <div class="empty card">
      <p>No documentation found matching "{filter}".</p>
    </div>
  {/if}
</div>

<style>
  .search-bar {
    display: flex;
    align-items: center;
    gap: 0.625rem;
    max-width: 44rem;
    margin: 0 0 2rem;
    padding: 0.625rem 0.875rem;
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
  }

  .search-bar input {
    flex: 1;
    background: transparent;
    border: none;
    color: var(--text-primary);
    font-size: var(--text-sm);
    outline: none;
  }

  .search-bar input::placeholder {
    color: var(--text-tertiary);
  }

  .clear {
    background: none;
    border: none;
    padding: 0.25rem;
    color: var(--text-tertiary);
    cursor: pointer;
    border-radius: var(--radius-sm);
  }

  .clear:hover {
    color: var(--text-primary);
  }

  .doc-sections {
    display: flex;
    flex-direction: column;
    gap: 2.5rem;
    max-width: 64rem;
  }

  .section-header {
    margin-bottom: 1rem;
  }

  .section-header h2 {
    font-size: var(--text-lg);
    font-weight: 600;
    margin: 0 0 0.25rem;
    color: var(--text-primary);
  }

  .section-desc {
    font-size: var(--text-sm);
    color: var(--text-secondary);
    margin: 0;
  }

  .card-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(18rem, 1fr));
    gap: 1rem;
  }

  .doc-card {
    display: flex;
    flex-direction: column;
    padding: 1.25rem;
    text-decoration: none;
    transition: transform var(--motion-fast) var(--ease-out), border-color var(--motion-fast) var(--ease-out);
  }

  .doc-card:hover {
    transform: translateY(-2px);
    border-color: var(--primary);
  }

  .card-top {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.5rem;
    font-size: var(--text-xs);
  }

  .category-tag {
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--primary);
  }

  .file-path {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
  }

  .doc-card h3 {
    margin: 0 0 0.5rem;
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text-primary);
  }

  .summary {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-secondary);
    line-height: 1.45;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .empty {
    padding: 2rem;
    text-align: center;
    color: var(--text-tertiary);
  }
</style>
