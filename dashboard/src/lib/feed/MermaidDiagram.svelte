<script lang="ts">
  /** Render a diagram lazily, with the source visible if parsing fails. */
  import { onMount } from "svelte";
  import { mermaidConfig } from "$lib/feed/mermaid-theme";

  let { source }: { source: string } = $props();
  const instanceId = $props.id();

  let svg = $state<string | null>(null);
  let error = $state<string | null>(null);
  let dark = $state(false);
  let sequence = 0;

  async function draw(diagram: string, darkMode: boolean) {
    const run = ++sequence;
    error = null;
    svg = null;
    try {
      const mermaid = (await import("mermaid")).default;
      mermaid.initialize(mermaidConfig(darkMode));
      // Multiple diagrams on one page must not share Mermaid's SVG definition IDs.
      const id = `sjel-mermaid-${instanceId.replace(/[^a-zA-Z0-9_-]/g, "")}-${run}`;
      const { svg: rendered } = await mermaid.render(id, diagram);
      if (run !== sequence) return; // a later run already superseded this one
      svg = rendered;
    } catch (cause) {
      if (run !== sequence) return;
      svg = null;
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  onMount(() => {
    const root = document.documentElement;
    dark = root.classList.contains("dark");
    const observer = new MutationObserver(() => {
      dark = root.classList.contains("dark");
    });
    observer.observe(root, { attributes: true, attributeFilter: ["class"] });
    return () => observer.disconnect();
  });

  $effect(() => {
    if (source) void draw(source, dark);
    return () => { sequence += 1; };
  });
</script>

<figure class="diagram" class:dark>
  {#if svg}
    <!-- Mermaid renders SVG under the strict security policy in mermaid-theme.ts. -->
    <!-- eslint-disable-next-line svelte/no-at-html-tags -->
    {@html svg}
  {:else if error}
    <p class="diagram-error">Mermaid could not draw this: {error}</p>
  {:else}
    <p class="diagram-pending">Drawing…</p>
  {/if}
  {#if error}
    <pre class="diagram-source">{source}</pre>
  {/if}
</figure>

<style>
  /* The figure sits on the palette's own paper in light mode and on the page's
     card in dark, so the node fills keep the same contrast either way without a
     white plate glaring out of a dark reader. */
  .diagram {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    margin: 0;
    padding: 1.25rem 1rem;
    border: 1px solid #ddd6cf; /* palette root: grid */
    border-radius: var(--radius-md);
    background: #fbfaf7; /* palette root: paper */
    /* A wide flowchart scrolls inside its own box rather than widening the
       reader column. */
    overflow-x: auto;
  }

  .diagram.dark {
    border-color: var(--card-border);
    background: var(--card-bg);
  }

  .diagram :global(svg) {
    max-width: 100%;
    height: auto;
  }

  .diagram-error {
    margin: 0;
    color: var(--danger);
    font-size: 0.875rem;
  }

  .diagram-pending {
    margin: 0;
    color: var(--text-tertiary);
    font-size: 0.875rem;
  }

  .diagram-source {
    margin: 0;
    padding: 0.75rem;
    border-radius: var(--radius-sm);
    background: var(--card-bg);
    color: var(--text-secondary);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    overflow-x: auto;
  }
</style>
