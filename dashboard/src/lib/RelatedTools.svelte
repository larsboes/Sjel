<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import { relatedToolsFor, type RelatedToolContext } from "$lib/related-tools";

  let {
    context,
    title,
    description,
  }: {
    context: RelatedToolContext;
    title: string;
    description: string;
  } = $props();

  const tools = $derived(relatedToolsFor(context));
</script>

<aside aria-label={title}>
  <details>
    <summary>
      <span class="mark"><Icon name="compass" size={15} /></span>
      <span class="summary-copy">
        <strong>{title}</strong>
        <small>{description}</small>
      </span>
      <span class="count">{tools.length} tools</span>
      <span class="chevron"><Icon name="arrow-right" size={14} /></span>
    </summary>

    <div class="tool-list">
      {#each tools as tool (tool.id)}
        <article>
          <div class="tool-copy">
            <div class="tool-heading">
              <h3>{tool.name}</h3>
              <span>{tool.kind}</span>
              <span>{tool.relation}</span>
            </div>
            <p>{tool.goodAt}</p>
            <small>{tool.boundary}</small>
          </div>
          <a href={tool.url} target="_blank" rel="noreferrer">
            {tool.action} <Icon name="external" size={12} />
          </a>
        </article>
      {/each}
    </div>
  </details>
</aside>

<style>
  aside {
    margin: 0.85rem 0 1.25rem;
  }

  details {
    border-block: 1px solid var(--card-border);
    background: color-mix(in srgb, var(--primary-soft) 30%, transparent);
  }

  summary {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto auto;
    gap: 0.75rem;
    align-items: center;
    min-height: 3.75rem;
    padding: 0.65rem 0.8rem;
    list-style: none;
    cursor: pointer;
  }

  summary::-webkit-details-marker {
    display: none;
  }

  .mark {
    display: grid;
    place-items: center;
    width: 1.8rem;
    height: 1.8rem;
    border: 1px solid color-mix(in srgb, var(--primary) 24%, var(--card-border));
    border-radius: var(--radius-sm);
    color: var(--primary);
  }

  .summary-copy {
    min-width: 0;
  }

  .summary-copy strong,
  .summary-copy small {
    display: block;
  }

  .summary-copy strong {
    font-size: var(--text-xs);
  }

  .summary-copy small {
    margin-top: 0.1rem;
    overflow: hidden;
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .count,
  .tool-heading span {
    padding: 0.12rem 0.4rem;
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text-tertiary);
    font-size: 0.625rem;
    font-weight: 600;
  }

  .chevron {
    color: var(--text-tertiary);
    transition: transform var(--motion-fast) ease;
  }

  details[open] .chevron {
    transform: rotate(90deg);
  }

  .tool-list {
    border-top: 1px solid var(--card-border);
  }

  article {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 1rem;
    align-items: center;
    padding: 0.8rem;
    background: var(--card-bg);
  }

  article + article {
    border-top: 1px solid var(--card-border);
  }

  .tool-copy {
    min-width: 0;
  }

  .tool-heading {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    align-items: center;
  }

  h3 {
    margin: 0 0.15rem 0 0;
    font-size: var(--text-sm);
  }

  p {
    margin: 0.25rem 0 0;
    color: var(--text-primary);
    font-size: var(--text-xs);
  }

  article small {
    display: block;
    margin-top: 0.2rem;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  article a {
    display: inline-flex;
    gap: 0.35rem;
    align-items: center;
    color: var(--primary);
    font-size: var(--text-2xs);
    font-weight: 600;
    white-space: nowrap;
  }

  article a:hover {
    text-decoration: underline;
    text-underline-offset: 0.2rem;
  }

  @media (width < 42rem) {
    .count {
      display: none;
    }

    article {
      grid-template-columns: 1fr;
      gap: 0.6rem;
    }

    article a {
      justify-self: start;
    }
  }
</style>
