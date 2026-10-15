<script lang="ts">
  // What the ingest gate did, read over HTTP from capabilities/media. The ledger's counts are
  // only half an answer: a refusal names the path and the reason, so the peek is where a run's
  // `refused` number becomes a list of files to look at.
  import { onMount } from "svelte";
  import DataTable, { type Column } from "$lib/ui/DataTable.svelte";
  import Chip, { type Tone } from "$lib/ui/Chip.svelte";
  import Section from "$lib/ui/Section.svelte";
  import SidePeek from "$lib/ui/SidePeek.svelte";
  import Icon from "$lib/Icon.svelte";
  import { tip } from "$lib/tip";
  import {
    media,
    type MediaDisposition,
    type MediaIngestDetail,
    type MediaIngestItem,
    type MediaIngestRun,
    type MediaStatus,
    type MediaVolume,
  } from "$lib/api";

  let status = $state<MediaStatus | null>(null);
  let runs = $state<MediaIngestRun[]>([]);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let reloading = $state(false);
  let openRun = $state<number | null>(null);
  let detail = $state<MediaIngestDetail | null>(null);
  let detailError = $state<string | null>(null);

  async function load(): Promise<void> {
    try {
      const [index, ingests] = await Promise.all([media.status(), media.ingests()]);
      status = index;
      runs = ingests.runs;
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
      reloading = false;
    }
  }

  async function reload(): Promise<void> {
    reloading = true;
    await load();
  }

  async function open(row: MediaIngestRun): Promise<void> {
    if (openRun === row.id) {
      openRun = null;
      return;
    }
    openRun = row.id;
    detail = null;
    detailError = null;
    try {
      detail = await media.ingest(row.id);
    } catch (e) {
      detailError = e instanceof Error ? e.message : String(e);
    }
  }

  onMount(load);

  const TONES: Record<MediaDisposition, Tone> = {
    imported: "success",
    duplicate: "muted",
    resumed: "accent",
    refused: "warning",
    failed: "danger",
  };

  /** Refusals and failures first: they are the rows a reader opens the peek to find. */
  const ATTENTION: MediaDisposition[] = ["failed", "refused"];

  const sortedItems = (items: MediaIngestItem[]): MediaIngestItem[] =>
    [...items].sort((a, b) => {
      const rank = (item: MediaIngestItem) => {
        const index = ATTENTION.indexOf(item.disposition);
        return index === -1 ? ATTENTION.length : index;
      };
      return rank(a) - rank(b) || a.relpath.localeCompare(b.relpath);
    });

  function bytes(value: number): string {
    if (value === 0) return "0 B";
    const units = ["B", "KiB", "MiB", "GiB", "TiB"];
    const i = Math.min(units.length - 1, Math.floor(Math.log(value) / Math.log(1024)));
    return `${(value / 1024 ** i).toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
  }

  function stamp(value: string | null): string {
    if (!value) return "—";
    const date = new Date(value.replace(" ", "T"));
    if (Number.isNaN(date.getTime())) return value;
    return date.toLocaleString("en-GB", {
      day: "2-digit",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  const runLabel = (run: MediaIngestRun): string => {
    const name = run.source.split("/").filter(Boolean).pop();
    return name ? `${name} · run ${run.id}` : `run ${run.id}`;
  };

  const volumeColumns: Column<MediaVolume>[] = [
    { id: "label", label: "Volume", cell: volumeLabelCell },
    { id: "uuid", label: "UUID", width: "10rem", cell: volumeUuidCell },
    { id: "locations", label: "Paths", width: "6rem", align: "end", cell: volumePathsCell },
    { id: "bytes", label: "Bytes", width: "7rem", align: "end", cell: volumeBytesCell },
    { id: "last_seen", label: "Last seen", width: "9rem", cell: volumeSeenCell },
  ];

  const runColumns: Column<MediaIngestRun>[] = [
    { id: "run", label: "Ingest", cell: runNameCell },
    { id: "staging", label: "Staging", width: "14rem", cell: runStagingCell },
    { id: "considered", label: "Seen", width: "5rem", align: "end", cell: runSeenCell },
    { id: "imported", label: "Imported", width: "6rem", align: "end", cell: runImportedCell },
    { id: "duplicates", label: "Duplicate", width: "6rem", align: "end", cell: runDuplicateCell },
    { id: "refused", label: "Refused", width: "6rem", align: "end", cell: runRefusedCell },
    { id: "failed", label: "Failed", width: "5rem", align: "end", cell: runFailedCell },
    { id: "finished", label: "Finished", width: "9rem", cell: runFinishedCell },
  ];
</script>

{#if error}
  <p class="error" role="alert">{error}</p>
{:else if loading}
  <p class="waiting"><Icon name="loader" size={14} /> Reading the ledger…</p>
{:else if status}
  <Section
    title="Index"
    meta={status.volumes.length === 1 ? "1 volume" : `${status.volumes.length} volumes`}
  >
    {#snippet actions()}
      <button type="button" class="refresh" onclick={() => void reload()} disabled={reloading}>
        <Icon name="refresh" size={13} /> {reloading ? "Reading…" : "Re-read"}
      </button>
    {/snippet}
    <div class="totals">
      <div class="total">
        <span class="value">{status.files.toLocaleString()}</span>
        <span class="label">digests</span>
      </div>
      <div class="total">
        <span class="value">{status.locations.toLocaleString()}</span>
        <span class="label">indexed paths</span>
      </div>
    </div>
    <DataTable
      rows={status.volumes}
      columns={volumeColumns}
      key={(row) => row.uuid}
      empty="No volume has been indexed yet. Run media index against a mounted library."
    />
  </Section>

  <Section title="Ingest runs" count={runs.length} meta={runs.length ? "newest first" : undefined}>
    <DataTable
      rows={runs}
      columns={runColumns}
      key={(row) => String(row.id)}
      onOpen={(row) => void open(row)}
      selected={openRun === null ? null : String(openRun)}
      empty="No ingest has run against this ledger yet."
    />
  </Section>

  {#if openRun !== null}
    {@const run = detail?.run ?? runs.find((candidate) => candidate.id === openRun)}
    <SidePeek
      title={run ? runLabel(run) : `run ${openRun}`}
      eyebrow="Ingest run"
      onClose={() => (openRun = null)}
    >
      {#if detailError}
        <p class="error" role="alert">{detailError}</p>
      {:else if !detail}
        <p class="waiting"><Icon name="loader" size={14} /> Reading the run…</p>
      {:else}
        <dl class="counts">
          <div><dt>seen</dt><dd>{detail.run.considered}</dd></div>
          <div><dt>imported</dt><dd>{detail.run.imported}</dd></div>
          <div><dt>duplicate</dt><dd>{detail.run.duplicates}</dd></div>
          <div><dt>refused</dt><dd class:alert={detail.run.refused > 0}>{detail.run.refused}</dd></div>
          <div><dt>failed</dt><dd class:alert={detail.run.failed > 0}>{detail.run.failed}</dd></div>
          <div><dt>resumed</dt><dd>{detail.run.resumed}</dd></div>
        </dl>
        {#if detail.items.length === 0}
          <p class="muted">This run recorded no paths.</p>
        {:else}
          <ul class="items">
            {#each sortedItems(detail.items) as item (item.relpath)}
              <li>
                <div class="item-head">
                  <Chip label={item.disposition} tone={TONES[item.disposition]} />
                  {#if item.verified}<Chip label="verified" tone="success" />{/if}
                  <span class="path" use:tip={item.relpath}>{item.relpath}</span>
                </div>
                <p class="reason">{item.reason}{item.imported_relpath ? ` → ${item.imported_relpath}` : ""}</p>
              </li>
            {/each}
          </ul>
        {/if}
      {/if}
    </SidePeek>
  {/if}
{/if}

{#snippet volumeLabelCell(row: MediaVolume)}
  <span class="strong">{row.label}</span>
{/snippet}
{#snippet volumeUuidCell(row: MediaVolume)}
  <span class="mono muted">{row.uuid}</span>
{/snippet}
{#snippet volumePathsCell(row: MediaVolume)}
  <span class="mono">{row.locations.toLocaleString()}</span>
{/snippet}
{#snippet volumeBytesCell(row: MediaVolume)}
  <span class="mono">{bytes(row.bytes)}</span>
{/snippet}
{#snippet volumeSeenCell(row: MediaVolume)}
  <span class="muted">{stamp(row.last_seen)}</span>
{/snippet}

{#snippet runNameCell(row: MediaIngestRun)}
  <span class="strong">{runLabel(row)}</span>
{/snippet}
{#snippet runStagingCell(row: MediaIngestRun)}
  <span class="mono muted" use:tip={row.staging}>{row.staging}</span>
{/snippet}
{#snippet runSeenCell(row: MediaIngestRun)}
  <span class="mono">{row.considered}</span>
{/snippet}
{#snippet runImportedCell(row: MediaIngestRun)}
  <span class="mono">{row.imported}</span>
{/snippet}
{#snippet runDuplicateCell(row: MediaIngestRun)}
  <span class="mono muted">{row.duplicates}</span>
{/snippet}
{#snippet runRefusedCell(row: MediaIngestRun)}
  {#if row.refused > 0}<Chip label={String(row.refused)} tone="warning" />{:else}<span class="mono muted">0</span>{/if}
{/snippet}
{#snippet runFailedCell(row: MediaIngestRun)}
  {#if row.failed > 0}<Chip label={String(row.failed)} tone="danger" />{:else}<span class="mono muted">0</span>{/if}
{/snippet}
{#snippet runFinishedCell(row: MediaIngestRun)}
  {#if row.finished}<span class="muted">{stamp(row.finished)}</span>{:else}<Chip label="running" tone="accent" />{/if}
{/snippet}

<style>
  .totals {
    display: flex;
    gap: var(--space-6);
    margin-bottom: var(--space-4);
  }

  .total {
    display: flex;
    flex-direction: column;
  }

  .total .value {
    font-family: var(--font-mono);
    font-size: var(--text-xl);
    font-weight: 650;
    line-height: var(--leading-tight);
  }

  .total .label {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .strong {
    font-weight: 550;
  }

  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }

  .muted {
    color: var(--text-tertiary);
  }

  .error {
    margin: 0;
    color: var(--danger);
    font-size: var(--text-sm);
  }

  .waiting {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .refresh {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--text-secondary);
    font-size: var(--text-xs);
  }

  .counts {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: var(--space-3);
    margin: 0 0 var(--space-4);
  }

  .counts dt {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .counts dd {
    margin: 0;
    font-family: var(--font-mono);
    font-size: var(--text-md);
  }

  .counts dd.alert {
    color: var(--warning-ink);
  }

  .items {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .item-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .path {
    overflow: hidden;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .reason {
    margin: var(--space-1) 0 0;
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }
</style>
