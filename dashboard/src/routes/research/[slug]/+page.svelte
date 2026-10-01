<script lang="ts">
  import PageHeader from "$lib/PageHeader.svelte";
  import MarkdownDocument from "$lib/feed/MarkdownDocument.svelte";
  import { resolveResearchLink } from "$lib/research/content";
  import { link } from "$lib/nav";

  let { data } = $props();
  const entry = $derived(data.entry);
</script>

<svelte:head><title>{entry.title} · Sjel research</title></svelte:head>

<PageHeader badge="Research" title={entry.title} />
<p class="back"><a href={link("/research")}>All research</a></p>

<article class="card prose">
  <MarkdownDocument content={entry.body} resolveLink={(href) => resolveResearchLink(href, link)} />
</article>
<p class="source">
  Source: <a href={`https://github.com/larsboes/Sjel/blob/main/research/${entry.file}`} target="_blank" rel="noreferrer"
    ><code>research/{entry.file}</code></a
  >
</p>

<style>
  .prose {
    max-width: 46rem;
    padding: 1.5rem 1.75rem;
  }
  .back,
  .source {
    margin: 0 0 1rem;
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }
  .source {
    margin-top: 1rem;
  }
  a {
    color: var(--primary);
  }
</style>
