<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import { axonStatus, panelUrl, type CapabilityView } from "$lib/api";
  import { capabilities } from "$lib/capabilities.svelte";
  import { titleCase, link } from "$lib/nav";

  const copy: Record<string, { title: string; description: string; kind: string; icon: "graduation" | "server" }> = {
    server: {
      title: "Home-Server & Local AI",
      description: "Procurement, privacy, Obsidian sync, and Sjel's path to a private production system.",
      kind: "Infrastructure plan",
      icon: "server",
    },
  };

  let busy = $state<string | null>(null);
  let error = $state<string | null>(null);

  onMount(() => capabilities.subscribe());

  function projectCopy(project: CapabilityView) {
    return (
      copy[project.name] ?? {
        title: titleCase(project.name),
        description: "Standalone Sjel interface.",
        kind: "Project",
        icon: "graduation" as const,
      }
    );
  }

  async function start(project: CapabilityView): Promise<void> {
    busy = project.name;
    error = null;
    try {
      await axonStatus.start(project.name);
      await capabilities.refresh();
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      busy = null;
    }
  }
</script>

<PageHeader
  badge="Projects"
  title="Standalone sites"
  desc="Projects run separately from the dashboard and start only when you need them."
/>

{#if error}
  <p class="error"><Icon name="alert" size={15} /> {error}</p>
{/if}

{#if capabilities.loading}
  <p class="empty">Reading projects from the registry…</p>
{:else if capabilities.panels.length === 0}
  <p class="empty">No project site is enabled on this machine.</p>
{:else}
  <ul>
    {#each capabilities.panels as project (project.name)}
      {@const text = projectCopy(project)}
      <li class="card">
        <div class="project-mark"><Icon name={text.icon} size={20} /></div>
        <div class="copy">
          <span class="kind">{text.kind}</span>
          <h2>{text.title}</h2>
          <p>{text.description}</p>
          <span class="state mono">
            <span class="dot" class:up={project.up === true}></span>
            {project.up === true ? "running" : project.up === false ? "off" : "status unknown"}
          </span>
        </div>
        <div class="actions">
          {#if project.up}
            <a class="btn btn-primary" href={panelUrl(project)} target="_blank" rel="noreferrer">
              Open site <Icon name="external" size={13} />
            </a>
          {:else}
            <button
              class="btn btn-primary"
              type="button"
              disabled={busy === project.name}
              onclick={() => void start(project)}
            >
              {#if busy === project.name}
                <Icon name="loader" size={14} /> starting…
              {:else}
                <Icon name="play" size={12} /> Start
              {/if}
            </button>
          {/if}
        </div>
      </li>
    {/each}
  </ul>
{/if}

<p class="note">
  The dashboard does not embed these sites. Start and stop them from
  <a href={link("/capabilities")}>Capabilities</a>.
</p>

<style>
  ul {
    display: grid;
    gap: 0.75rem;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.9rem;
    padding: 1rem;
  }

  .project-mark {
    display: grid;
    place-items: center;
    width: 2.5rem;
    height: 2.5rem;
    border-radius: var(--radius-md);
    background: var(--primary-soft);
    color: var(--primary);
  }

  .copy {
    min-width: 0;
  }

  .kind {
    color: var(--primary);
    font-size: 0.625rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  h2 {
    margin: 0.1rem 0 0;
    font-size: 1rem;
  }

  p {
    margin: 0.25rem 0 0;
    color: var(--text-secondary);
    font-size: var(--text-xs);
  }

  .state {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    margin-top: 0.55rem;
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .dot {
    width: 0.4rem;
    height: 0.4rem;
    border-radius: 50%;
    background: var(--text-tertiary);
  }

  .dot.up {
    background: var(--success);
  }

  .actions {
    grid-column: 1 / -1;
  }

  .error,
  .empty {
    margin: 0 0 1rem;
    padding: 0.85rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
    font-size: var(--text-xs);
  }

  .error {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    color: var(--danger);
    background: var(--danger-soft);
  }

  .note {
    margin-top: 1rem;
    color: var(--text-tertiary);
  }

  .note a {
    color: var(--primary);
  }

  @media (width >= 42rem) {
    li {
      grid-template-columns: auto 1fr auto;
      align-items: center;
    }

    .actions {
      grid-column: auto;
    }
  }
</style>
