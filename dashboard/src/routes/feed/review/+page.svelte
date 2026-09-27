<script lang="ts">
  import { link } from "$lib/nav";
  import { onMount } from "svelte";
  import FeedNav from "$lib/feed/FeedNav.svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import {
    axonStatus,
    comms,
    type FeedQualityFlag,
    type FeedQualityRefresh,
  } from "$lib/api";

  interface ReviewItem {
    feedId: string;
    title: string | null;
    url: string;
    status: FeedQualityFlag["status"];
    contentStatus: FeedQualityFlag["content_status"];
    derivedAt: string;
    flags: FeedQualityFlag[];
  }

  const SIGNAL_LABEL: Record<string, string> = {
    content_status: "Content",
    extraction_path: "Extraction",
    retention: "Retention",
    boilerplate_leakage: "Boilerplate",
    summary_attempts: "Summary retries",
    ranking_basis: "Ranking basis",
  };

  let flags = $state<FeedQualityFlag[]>([]);
  let loading = $state(true);
  let refreshing = $state(false);
  let error = $state<string | null>(null);
  let lastRefresh = $state<FeedQualityRefresh | null>(null);

  const items = $derived.by(() => {
    const grouped = new Map<string, ReviewItem>();
    for (const flag of flags) {
      const current = grouped.get(flag.feed_id);
      if (current) {
        current.flags.push(flag);
      } else {
        grouped.set(flag.feed_id, {
          feedId: flag.feed_id,
          title: flag.title,
          url: flag.url,
          status: flag.status,
          contentStatus: flag.content_status,
          derivedAt: flag.derived_at,
          flags: [flag],
        });
      }
    }
    return [...grouped.values()];
  });

  async function loadQueue() {
    flags = await comms.qualityFlags();
  }

  async function refreshQueue() {
    refreshing = true;
    error = null;
    try {
      lastRefresh = await comms.refreshQualityFlags();
      await loadQueue();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : "Quality refresh failed.";
    } finally {
      refreshing = false;
    }
  }

  onMount(() => {
    void (async () => {
      try {
        await axonStatus.start("comms").catch(() => undefined);
        await loadQueue();
      } catch (cause) {
        error = cause instanceof Error ? cause.message : "Review queue unavailable.";
      } finally {
        loading = false;
      }
    })();
  });

  function formatTime(value: string): string {
    const date = new Date(value);
    return Number.isNaN(date.getTime())
      ? value
      : new Intl.DateTimeFormat("en-GB", {
          dateStyle: "medium",
          timeStyle: "short",
        }).format(date);
  }
</script>

<svelte:head><title>Feed review · Sjel</title></svelte:head>

<PageHeader
  badge="Feed"
  title="Computed review queue"
  desc="Inspect deterministic warnings from stored pipeline evidence. Flags suggest review; they never start enrichment or change a Feed decision."
/>

<FeedNav active="review" />

<section class="queue-head" aria-labelledby="queue-title">
  <div>
    <p class="eyebrow mono">Quality signals</p>
    <h2 id="queue-title">{items.length} {items.length === 1 ? "item" : "items"} to inspect</h2>
    <p>
      The refresh checks at most 500 entries from the last ten years against the public
      extraction-corpus envelope and the stored stage ledger.
    </p>
  </div>
  <button type="button" onclick={refreshQueue} disabled={refreshing || loading}>
    <Icon name={refreshing ? "loader" : "refresh"} size={14} />
    {refreshing ? "Computing…" : "Refresh computed flags"}
  </button>
</section>

{#if lastRefresh}
  <p class="refresh-result mono" aria-live="polite">
    Reviewed {lastRefresh.reviewed}; flagged {lastRefresh.flagged_items} items with
    {lastRefresh.flag_count} signals; provider calls {lastRefresh.provider_calls}.
  </p>
{/if}

{#if error}
  <p class="notice error"><Icon name="alert" size={14} /> {error}</p>
{:else if loading}
  <p class="notice"><Icon name="loader" size={14} /> Loading stored flags…</p>
{:else if items.length === 0}
  <section class="empty">
    <h2>No stored flags</h2>
    <p>Run the explicit refresh to compute the queue, or the current evidence is inside the thresholds.</p>
  </section>
{:else}
  <ol class="queue">
    {#each items as item (item.feedId)}
      <li>
        <article>
          <header>
            <div>
              <div class="meta mono">
                <span>{item.status}</span>
                <span>content {item.contentStatus}</span>
                <time datetime={item.derivedAt}>{formatTime(item.derivedAt)}</time>
              </div>
              <h2><a href={link(`/feed/${item.feedId}`)}>{item.title ?? item.url}</a></h2>
            </div>
            <a class="original" href={item.url} target="_blank" rel="noreferrer">
              Original <Icon name="external" size={13} />
            </a>
          </header>
          <ul class="signals">
            {#each item.flags as flag (`${flag.feed_id}:${flag.signal}`)}
              <li>
                <span class="signal mono">{SIGNAL_LABEL[flag.signal] ?? flag.signal}</span>
                <div>
                  <p>{flag.reason}</p>
                  <code>{flag.evidence}</code>
                </div>
              </li>
            {/each}
          </ul>
        </article>
      </li>
    {/each}
  </ol>
{/if}

<style>
  .queue-head {
    display: flex;
    align-items: end;
    justify-content: space-between;
    gap: 2rem;
    padding: 1.25rem 0 1.5rem;
    border-bottom: 1px solid var(--card-border);
  }

  .queue-head h2,
  article h2,
  .empty h2 {
    margin: 0;
  }

  .queue-head p:not(.eyebrow) {
    max-width: 62ch;
    margin: 0.45rem 0 0;
    color: var(--text-secondary);
    line-height: 1.55;
  }

  .eyebrow {
    margin: 0 0 0.35rem;
    color: var(--primary);
    font-size: 0.7rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  button {
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
    flex: 0 0 auto;
    padding: 0.65rem 0.85rem;
    border: 1px solid var(--primary);
    border-radius: 6px;
    background: transparent;
    color: var(--primary);
    font: inherit;
    font-size: var(--text-sm);
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.55;
    cursor: wait;
  }

  .refresh-result,
  .notice {
    margin: 1rem 0;
    color: var(--text-secondary);
    font-size: 0.78rem;
  }

  .notice {
    display: flex;
    align-items: center;
    gap: 0.45rem;
  }

  .notice.error {
    color: var(--danger, #b44343);
  }

  .queue {
    display: grid;
    gap: 1rem;
    margin: 1.25rem 0 3rem;
    padding: 0;
    list-style: none;
  }

  article,
  .empty {
    padding: 1.1rem 1.2rem;
    border: 1px solid var(--card-border);
    border-radius: 8px;
    background: var(--card-bg);
  }

  article > header {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
  }

  article h2 {
    margin-top: 0.35rem;
    font-size: 1rem;
    line-height: 1.35;
  }

  article h2 a {
    color: var(--text-primary);
    text-decoration: none;
  }

  article h2 a:hover,
  .original:hover {
    color: var(--primary);
  }

  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0.7rem;
    color: var(--text-tertiary);
    font-size: 0.68rem;
    text-transform: uppercase;
  }

  .original {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    flex: 0 0 auto;
    color: var(--text-tertiary);
    font-size: var(--text-xs);
    text-decoration: none;
  }

  .signals {
    display: grid;
    gap: 0.65rem;
    margin: 1rem 0 0;
    padding: 0;
    list-style: none;
  }

  .signals > li {
    display: grid;
    grid-template-columns: 8.5rem 1fr;
    gap: 0.8rem;
    padding-top: 0.65rem;
    border-top: 1px solid var(--card-border);
  }

  .signal {
    color: var(--primary);
    font-size: 0.68rem;
    text-transform: uppercase;
  }

  .signals p {
    margin: 0 0 0.25rem;
    color: var(--text-secondary);
    font-size: 0.82rem;
  }

  code {
    display: block;
    overflow-wrap: anywhere;
    color: var(--text-tertiary);
    font-size: 0.7rem;
    white-space: normal;
  }

  .empty {
    margin-top: 1.25rem;
  }

  .empty p {
    margin-bottom: 0;
    color: var(--text-secondary);
  }

  @media (max-width: 680px) {
    .queue-head,
    article > header {
      align-items: stretch;
      flex-direction: column;
    }

    .queue-head button {
      justify-content: center;
    }

    .signals > li {
      grid-template-columns: 1fr;
      gap: 0.3rem;
    }
  }
</style>
