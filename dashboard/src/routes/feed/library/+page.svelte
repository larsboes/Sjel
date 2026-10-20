<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import {
    sjelStatus,
    comms,
    type CommsEvaluationStatus,
    type FeedEntry,
    type FeedStatus,
  } from "$lib/api";
  import PageHeader from "$lib/PageHeader.svelte";
  import FeedNav from "$lib/feed/FeedNav.svelte";
  import FeedMetricsBar from "$lib/feed/FeedMetricsBar.svelte";
  import FeedTripTimeline from "$lib/feed/FeedTripTimeline.svelte";
  import FeedItemPeek from "$lib/feed/FeedItemPeek.svelte";
  import Collection from "$lib/ui/Collection.svelte";
  import type { Field } from "$lib/ui/collection";
  import type { Tone } from "$lib/ui/Chip.svelte";
  import Icon from "$lib/Icon.svelte";
  import { tip } from "$lib/tip";
  import { FEED_KIND_LABEL } from "$lib/feed/FeedItemRow.svelte";

  let entries = $state<FeedEntry[]>([]);
  let modelStatus = $state<CommsEvaluationStatus | null>(null);
  let loading = $state(true);
  let offline = $state(false);
  let peek = $state<string | null>(null);
  let busy = $state<string | null>(null);

  const openEntry = $derived(entries.find((e) => e.id === peek) ?? null);

  const KIND_TONE: Record<string, Tone> = {
    youtube: "danger",
    article: "accent",
    podcast: "warning",
    github: "neutral",
    mail: "success",
    reddit: "accent",
    arxiv: "muted",
    instagram: "accent",
  };

  const STATUS_LABEL: Record<FeedStatus, string> = {
    new: "New",
    keeper: "Kept",
    dismissed: "Dismissed",
  };

  const fields: Field<FeedEntry>[] = [
    {
      id: "kind",
      label: "Type",
      kind: "select",
      width: "6.5rem",
      value: (r) => r.kind,
      display: (v) => FEED_KIND_LABEL[v] ?? v,
      tone: (v) => KIND_TONE[v] ?? "neutral",
    },
    {
      id: "title",
      label: "Title",
      value: (r) => r.title ?? r.url,
      cell: titleCell,
    },
    {
      id: "author",
      label: "Author",
      width: "9rem",
      value: (r) => r.author ?? "",
    },
    {
      id: "lens",
      label: "TELOS Lens",
      kind: "select",
      width: "8rem",
      value: (r) => r.relevance?.profile_label ?? "Unassigned",
      tone: () => "neutral",
    },
    {
      id: "score",
      label: "Fit",
      kind: "number",
      width: "5.5rem",
      align: "end",
      value: (r) => Math.round((r.evaluation?.overall_score ?? r.relevance?.score ?? 0) * 100),
      cell: scoreCell,
    },
    {
      id: "status",
      label: "Status",
      kind: "select",
      width: "6.5rem",
      value: (r) => r.status,
      display: (v) => STATUS_LABEL[v as FeedStatus] ?? v,
      tone: (v) => (v === "new" ? "accent" : v === "keeper" ? "success" : "muted"),
    },
    {
      id: "date",
      label: "Ingested",
      kind: "date",
      width: "7.5rem",
      value: (r) => r.created_at.slice(0, 10),
    },
    {
      id: "trip",
      label: "Trip",
      kind: "select",
      column: false,
      value: (r) => {
        const factor = r.evaluation?.factors.find((f) => f.key === "travel");
        return factor && factor.score > 0 ? (factor.context?.id ?? null) : null;
      },
    },
  ];

  async function load() {
    try {
      await sjelStatus.start("comms").catch(() => undefined);
      const [feed, status] = await Promise.all([
        comms.feed({ days: 3650, includeDismissed: true }),
        comms.evaluationStatus().catch(() => null),
      ]);
      entries = feed;
      modelStatus = status;
      offline = false;
    } catch {
      offline = true;
    } finally {
      loading = false;
    }
  }

  async function setStatus(id: string, next: FeedStatus): Promise<void> {
    if (busy) return;
    busy = id;
    try {
      await comms.setStatus(id, next);
      entries = entries.map((e) => (e.id === id ? { ...e, status: next } : e));
    } finally {
      busy = null;
    }
  }

  function filterByTrip(tripId: string) {
    const url = new URL(page.url);
    url.searchParams.set("feed.f.trip", tripId);
    void goto(url.toString(), { replaceState: true });
  }

  onMount(load);
</script>

{#snippet titleCell(r: FeedEntry)}
  <span class="feed-item-cell">
    <span class="entry-title">{r.title ?? r.url}</span>
    <a
      href={r.url}
      target="_blank"
      rel="noreferrer"
      class="ext-link"
      aria-label="Open source"
      use:tip={"Open source URL"}
      onclick={(e) => e.stopPropagation()}
    >
      <Icon name="external" size={11} />
    </a>
  </span>
{/snippet}

{#snippet scoreCell(r: FeedEntry)}
  {@const val = Math.round((r.evaluation?.overall_score ?? r.relevance?.score ?? 0) * 100)}
  <span class="score-num mono">{val}%</span>
{/snippet}

{#snippet rowActions(r: FeedEntry)}
  <div class="row-actions">
    {#if busy === r.id}
      <span class="spinner"><Icon name="loader" size={13} /></span>
    {:else if r.status === "new"}
      <button
        type="button"
        class="action-btn success"
        aria-label="Keep in Obsidian"
        use:tip={"Keep in Obsidian"}
        onclick={(e) => {
          e.stopPropagation();
          void setStatus(r.id, "keeper");
        }}
      >
        <Icon name="check" size={12} />
      </button>
      <button
        type="button"
        class="action-btn danger"
        aria-label="Dismiss"
        use:tip={"Dismiss"}
        onclick={(e) => {
          e.stopPropagation();
          void setStatus(r.id, "dismissed");
        }}
      >
        <Icon name="close" size={12} />
      </button>
    {:else if r.status === "keeper"}
      <button
        type="button"
        class="action-btn"
        aria-label="Mark New"
        use:tip={"Mark New"}
        onclick={(e) => {
          e.stopPropagation();
          void setStatus(r.id, "new");
        }}
      >
        <Icon name="arrow-right" size={12} />
      </button>
    {:else}
      <button
        type="button"
        class="action-btn"
        aria-label="Restore to New"
        use:tip={"Restore to New"}
        onclick={(e) => {
          e.stopPropagation();
          void setStatus(r.id, "new");
        }}
      >
        <Icon name="arrow-right" size={12} />
      </button>
    {/if}
  </div>
{/snippet}

<PageHeader
  badge="Feed"
  title="Library"
  desc="The complete Feed archive as a database. Group by media type, TELOS focus, or status across Table, Board, and Calendar views."
/>

<FeedNav active="library" />

{#if offline}
  <p class="notice error">
    <Icon name="wifi-off" />
    The feed library could not be loaded.
  </p>
{:else if loading}
  <p class="notice muted">
    <Icon name="loader" size={13} />
    Loading collection…
  </p>
{:else}
  <FeedMetricsBar {entries} />

  <FeedTripTimeline
    {entries}
    plans={modelStatus?.travel_context?.plans}
    onSelectTrip={filterByTrip}
  />

  <Collection
    id="feed"
    rows={entries}
    {fields}
    key={(r) => r.id}
    title={(r) => r.title ?? r.url}
    defaults={{ view: "table", group: "kind", filter: { status: ["new", "keeper"] } }}
    onOpen={(r) => (peek = r.id)}
    inactive={(r) => r.status === "dismissed"}
    selected={peek}
    actions={rowActions}
    empty="No feed entries match these filters."
  />

  {#if openEntry}
    <FeedItemPeek
      entry={openEntry}
      busy={busy === openEntry.id}
      onClose={() => (peek = null)}
      onSetStatus={setStatus}
    />
  {/if}
{/if}

<style>
  .notice {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    padding: var(--space-4) 0;
  }

  .notice.error {
    color: var(--warning-ink);
  }

  .notice.muted {
    color: var(--text-tertiary);
  }

  .feed-item-cell {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .entry-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-primary);
  }

  .ext-link {
    color: var(--text-tertiary);
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
  }

  .ext-link:hover {
    color: var(--accent);
  }

  .score-num {
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .row-actions {
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }

  .action-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    padding: 0;
    border-radius: var(--radius-sm);
    background: var(--surface);
    border: 1px solid var(--card-border);
    color: var(--text-secondary);
    cursor: pointer;
  }

  .action-btn:hover {
    color: var(--text-primary);
    background: var(--nav-hover);
  }

  .action-btn.success:hover {
    color: var(--success);
    border-color: var(--success);
  }

  .action-btn.danger:hover {
    color: var(--danger);
    border-color: var(--danger);
  }

  .spinner {
    display: inline-flex;
    align-items: center;
    color: var(--text-tertiary);
  }
</style>
