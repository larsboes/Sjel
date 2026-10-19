<script lang="ts">
  import PageHeader from "$lib/PageHeader.svelte";
  import MarkdownDocument from "$lib/feed/MarkdownDocument.svelte";
  import Icon from "$lib/Icon.svelte";
  import { link } from "$lib/nav";
  import { resolveDocLink } from "$lib/docs/content";

  let { data } = $props();
  const doc = $derived(data.doc);
  const sections = $derived(data.sections);
</script>

<svelte:head>
  <title>{doc.title} · Sjel Documentation</title>
</svelte:head>

<div class="docs-layout">
  <aside class="docs-sidebar">
    <div class="sidebar-header">
      <a class="all-docs" href={link("/docs")}>
        <Icon name="arrow-left" size={14} />
        <span>All Documentation</span>
      </a>
    </div>

    <nav class="nav-tree" aria-label="Documentation sections">
      {#each sections as section (section.id)}
        <div class="nav-group">
          <span class="group-title">{section.label}</span>
          <ul class="nav-list">
            {#each section.docs as item (item.slug)}
              {@const href = link(item.slug === "index" ? "/docs/README" : `/docs/${item.slug}`)}
              {@const isCurrent = item.slug === doc.slug}
              <li>
                <a
                  {href}
                  class:active={isCurrent}
                  aria-current={isCurrent ? "page" : undefined}
                >
                  {item.title}
                </a>
              </li>
            {/each}
          </ul>
        </div>
      {/each}
    </nav>
  </aside>

  <main class="docs-main">
    <div class="doc-header-row">
      <PageHeader
        badge="Specification"
        title={doc.title}
      />
    </div>

    <article class="card prose">
      <MarkdownDocument
        content={doc.body}
        resolveLink={(href) => resolveDocLink(href, doc.file, link)}
      />
    </article>

    <div class="doc-footer">
      <p class="source-link">
        Source:
        <a
          href={`https://github.com/larsboes/Sjel/blob/main/${doc.file}`}
          target="_blank"
          rel="noreferrer"
        >
          <code>{doc.file}</code>
        </a>
      </p>
    </div>
  </main>
</div>

<style>
  .docs-layout {
    display: grid;
    grid-template-columns: 16rem 1fr;
    gap: 2rem;
    align-items: start;
    max-width: 72rem;
  }

  .docs-sidebar {
    position: sticky;
    top: 1rem;
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
    padding-right: 1rem;
    border-right: 1px solid var(--card-border);
    max-height: calc(100vh - 4rem);
    overflow-y: auto;
  }

  .sidebar-header {
    padding-bottom: 0.5rem;
    border-bottom: 1px solid var(--card-border);
  }

  .all-docs {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    font-size: var(--text-sm);
    color: var(--text-secondary);
    text-decoration: none;
    font-weight: 500;
  }

  .all-docs:hover {
    color: var(--primary);
  }

  .nav-tree {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }

  .group-title {
    display: block;
    font-size: var(--text-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-tertiary);
    margin-bottom: 0.5rem;
  }

  .nav-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .nav-list a {
    display: block;
    padding: 0.375rem 0.625rem;
    font-size: var(--text-sm);
    color: var(--text-secondary);
    text-decoration: none;
    border-radius: var(--radius-sm);
    transition: background var(--motion-fast) var(--ease-out), color var(--motion-fast) var(--ease-out);
  }

  .nav-list a:hover {
    background: var(--surface);
    color: var(--text-primary);
  }

  .nav-list a.active {
    background: var(--card-bg);
    color: var(--primary);
    font-weight: 600;
  }

  .docs-main {
    min-width: 0;
  }

  .doc-header-row {
    margin-bottom: 1rem;
  }

  .prose {
    padding: 2rem;
    line-height: 1.6;
  }

  .doc-footer {
    margin-top: 1.5rem;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .source-link a {
    color: var(--primary);
    text-decoration: none;
  }

  .source-link a:hover {
    text-decoration: underline;
  }

  @media (max-width: 768px) {
    .docs-layout {
      grid-template-columns: 1fr;
    }

    .docs-sidebar {
      position: static;
      border-right: none;
      border-bottom: 1px solid var(--card-border);
      padding-bottom: 1.5rem;
      max-height: none;
    }
  }
</style>
