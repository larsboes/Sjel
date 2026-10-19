<script lang="ts">
  import { page } from "$app/state";
  import PageHeader from "$lib/PageHeader.svelte";
  import ProjectsBoard from "$lib/projects/ProjectsBoard.svelte";

  const selected = $derived(page.url.searchParams.get("project"));
  const open = $derived(page.url.searchParams.get("open"));
</script>

<PageHeader badge="Projects" title={selected ?? "Projects"} desc="Projects and tasks. Sjel's store owns them; every change renders its markdown file." />

<ProjectsBoard {selected} {open} />
