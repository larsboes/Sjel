<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import { axonStatus, hasPanel, panelUrl } from "$lib/api";
  import { capabilities } from "$lib/capabilities.svelte";
  import { titleCase, link } from "$lib/nav";

  const name = $derived(page.params.name ?? "");
  const capability = $derived(capabilities.byName(name));

  let starting = $state(false);
  let error = $state<string | null>(null);

  onMount(() => capabilities.subscribe());

  async function start(): Promise<void> {
    starting = true;
    error = null;
    try {
      await axonStatus.start(name);
      await capabilities.refresh();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      starting = false;
    }
  }
</script>

{#if !capability}
  <PageHeader badge="Panel" title={titleCase(name)} />
  <p class="muted">
    {#if capabilities.loading}Loading registry…{:else}No enabled capability has this name.{/if}
  </p>
{:else if !hasPanel(capability)}
  <PageHeader badge="Panel" title={titleCase(name)} />
  <p class="muted">{name} does not provide its own interface.</p>
{:else if !capability.up}
  <PageHeader badge="Panel" title={titleCase(name)} />
  <!-- Not an error state. On-demand is the design: nothing but the shell and
       sjel-status runs until you open something, and opening it starts it. -->
  <div class="card offer">
    <p class="lead">{name} is not running.</p>
    <p class="muted">It starts on demand. Nothing runs in the background until you open it.</p>
    {#if error}
      <p class="error"><Icon name="alert" size={14} /> {error}</p>
    {/if}
    <button class="btn btn-primary" onclick={start} disabled={starting}>
      {#if starting}
        <Icon name="loader" size={14} /> starting…
      {:else}
        <Icon name="play" size={12} /> Start {name}
      {/if}
    </button>
  </div>
{:else}
  <PageHeader
    badge="Standalone site"
    title={titleCase(name)}
    desc="This project runs separately from the dashboard and opens in its own tab."
  />
  <div class="card offer">
    <p class="lead">{name} is ready.</p>
    <p class="muted mono">{panelUrl(capability)}</p>
    <a class="btn btn-primary" href={panelUrl(capability)} target="_blank" rel="noreferrer">
      Open site <Icon name="external" size={13} />
    </a>
    <a class="back" href={link("/projects")}>Back to Projects</a>
  </div>
{/if}

<style>
  .offer {
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.6rem;
  }

  .lead {
    margin: 0;
    font-weight: 500;
  }

  .muted {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }

  .error {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin: 0;
    font-size: var(--text-sm);
    color: var(--warning-ink);
  }

  .back {
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  .back:hover {
    color: var(--primary);
  }
</style>
