<script lang="ts">
  import type { ComponentRenderProps } from '@json-render/svelte';
  let { children }: ComponentRenderProps = $props();
</script>
<div class="stack">{@render children?.()}</div>
<style>.stack { display: grid; gap: 1.5rem; }</style>
