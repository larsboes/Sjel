<script lang="ts">
  import PageTabs from "$lib/PageTabs.svelte";
  import { link } from "$lib/nav";
  import { benchmarks, entries, registers } from "virtual:sjel-research";

  let { current }: { current: "articles" | "projects" | "benchmarks" } = $props();

  const items = [
    { id: "articles", label: "Articles", count: entries.filter((e) => e.file !== "README.md").length, href: link("/research") },
    {
      id: "projects",
      label: "Projects",
      count: registers.systems.length + registers.upstreams.length,
      href: link("/research/projects"),
    },
    { id: "benchmarks", label: "Benchmarks", count: benchmarks.length, href: link("/research/benchmarks") },
  ];
</script>

<PageTabs {items} value={current} label="Research sections" />
