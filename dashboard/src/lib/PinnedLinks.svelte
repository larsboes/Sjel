<script lang="ts">
  /**
   * Operator-pinned links, straight from the overlay's links.toml via sjel-status.
   *
   * The shell owns the card; the overlay owns the entries. A deployment that pins
   * nothing renders nothing — no empty-state prose for a purely optional surface.
   * Read-only, outbound: these are the operator's own services (a tailnet page, a
   * family-node UI), so they open in a new tab and never route through nav.ts.
   */
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import RailSection from "$lib/rail/RailSection.svelte";
  import { axonStatus, type PinnedLink } from "$lib/api";

  let links = $state<PinnedLink[]>([]);

  onMount(async () => {
    try {
      links = (await axonStatus.links()).links;
    } catch {
      // The card is decoration: an unreachable sjel-status already renders loudly in
      // the surfaces that own health, so this stays quiet and empty.
    }
  });
</script>

{#if links.length > 0}
  <RailSection label="Pinned links" count={links.length} open>
    <ul>
      {#each links as pinned (pinned.url)}
        <li>
          <a href={pinned.url} target="_blank" rel="noreferrer">
            <span class="name">{pinned.name}</span>
            {#if pinned.note}<small>{pinned.note}</small>{/if}
            <Icon name="external" size={12} />
          </a>
        </li>
      {/each}
    </ul>
  </RailSection>
{/if}

<style>
  /* No box, and no heading of its own. This sits inside the rail, which is already a
     pane, and the section around it is `RailSection` — the same disclosure, chevron and
     count every other rail section uses. It kept its own `<h2>` and rule until
     2026-09-07, which is why the rail had two heading sizes in one column. */
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 0.35rem;
  }
  a {
    display: flex;
    align-items: baseline;
    gap: 0.45rem;
    text-decoration: none;
    color: inherit;
    padding: 0.25rem 0.3rem;
    border-radius: 6px;
  }
  a:hover {
    background: var(--nav-hover);
  }

  a:focus-visible {
    outline: 2px solid var(--focus-ring);
  }
  /* One line per link in a narrow rail: the name keeps its line and the note gives way. */
  .name {
    flex-shrink: 0;
    white-space: nowrap;
    font-weight: 600;
    font-size: var(--text-sm);
  }
  small {
    color: var(--text-tertiary);
    font-size: var(--text-xs);
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
