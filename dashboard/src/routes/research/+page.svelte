<script lang="ts">
  import PageHeader from "$lib/PageHeader.svelte";
  import MarkdownDocument from "$lib/feed/MarkdownDocument.svelte";
  import ResearchTabs from "$lib/research/ResearchTabs.svelte";
  import { resolveResearchLink, toEntry } from "$lib/research/content";
  import { link } from "$lib/nav";
  import { entries } from "virtual:sjel-research";

  // research/README.md is the index: its entry list and its open questions are written by hand,
  // so this page renders them rather than generating a second list that could disagree.
  const readme = toEntry("README.md", entries.find((e) => e.file === "README.md")?.markdown ?? "# Research\n");
</script>

<svelte:head><title>Research · Sjel</title></svelte:head>

<PageHeader badge="Research" title={readme.title} desc="Why Sjel exists, the evidence behind it, and the questions still open." />
<ResearchTabs current="articles" />

<article class="card prose">
  <MarkdownDocument content={readme.body} resolveLink={(href) => resolveResearchLink(href, link)} />
</article>

<style>
  .prose {
    max-width: 46rem;
    padding: 1.5rem 1.75rem;
  }
</style>
