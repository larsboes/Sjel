<script lang="ts">
  // A record as a page of its own (2026-10-08): title, properties, then everything it
  // touches in other capabilities. The third layer after a row and its peek or inspector;
  // `lib/inspector/record.ts` says which records survive a reload.
  import { untrack } from "svelte";
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import StateLine from "$lib/StateLine.svelte";
  import Section from "$lib/ui/Section.svelte";
  import Property from "$lib/ui/Property.svelte";
  import { capabilities } from "$lib/capabilities.svelte";
  import ConnectionGroups from "$lib/inspector/ConnectionGroups.svelte";
  import { connectionsFor, deepLink, expand, type ConnectionGroup } from "$lib/inspector/connections";
  import type { InspectableItem } from "$lib/inspector/inspector.svelte";
  import { recordHref, recordView, resolveRecord } from "$lib/inspector/record";
  import { contextHref } from "$lib/context/context";

  const id = $derived(page.url.searchParams.get("id") ?? "");
  let item = $state<InspectableItem | null | undefined>(undefined);
  let groups = $state<ConnectionGroup[] | null>(null);

  $effect(() => {
    const current = id;
    let live = true;
    item = undefined;
    groups = null;
    void resolveRecord(current).then(async (found) => {
      if (!live) return;
      item = found;
      if (!found) return;
      // The registry re-polls; reading it tracked would refetch and blank the list.
      const registry = untrack(() => capabilities.items);
      const answer = await connectionsFor(found, registry);
      if (live) groups = answer;
    });
    return () => {
      live = false;
    };
  });

  async function follow(next: InspectableItem) {
    const href = recordHref(await expand(next));
    if (href) void goto(href);
  }

  const view = $derived(item ? recordView(item) : null);
  const home = $derived(item ? deepLink(item) : null);
  const around = $derived(item ? contextHref(item) : null);
</script>

{#if item === undefined}
  <StateLine state="loading" message="Reading the record…" />
{:else if item === null || !view}
  <StateLine state="error" message="This record cannot be read on its own. Open it from its list, then use Open as page." />
{:else}
  <PageHeader badge={view.kind} title={view.title}>
    {#snippet actions()}
      {#if around}
        <a class="open" href={around}><Icon name="compass" size={12} />Everything around it</a>
      {/if}
      {#if home}
        <a class="open" href={home}><Icon name="external" size={12} />Open where it lives</a>
      {/if}
    {/snippet}
  </PageHeader>

  <div class="record">
    <Section title="Properties">
      {#each view.properties as p (p.label)}
        <Property label={p.label} value={p.value} readonly />
      {/each}
    </Section>
    <Section title="Connected">
      <ConnectionGroups {groups} onfollow={(next) => void follow(next)} />
    </Section>
  </div>
{/if}

<style>
  .record {
    display: grid;
    gap: var(--space-6);
    max-width: 48rem;
  }

  .open {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: var(--text-2xs);
    color: var(--text-secondary);
  }

  .open:hover {
    color: var(--text-primary);
  }
</style>
