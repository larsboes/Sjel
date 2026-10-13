<script lang="ts">
  // What Sjel has put into each agent harness on this machine, and the three buttons that
  // change it.
  //
  // The read is one GET. sjel-status shells `tools/harnesses status --json`, which is the same
  // code path the CLI and the session hook read, so this view and the terminal cannot disagree
  // about what is deployed. The write is a POST to the same path, which shells the launcher
  // that owns the verb — so a refusal here is the tool's own reason, shown as it wrote it.
  //
  // An agent cannot reach either: sjel-status admits no agent token. An agent that wants a
  // deploy runs `tools/harnesses` in its own shell, under its own permission gate.
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import { axonStatus, type PacksView, type HarnessView } from "$lib/api";
  import Chip from "$lib/ui/Chip.svelte";
  import Collection from "$lib/ui/Collection.svelte";
  import type { Column } from "$lib/ui/DataTable.svelte";
  import type { Field } from "$lib/ui/collection";

  type Row = { pack: string; skill: string };

  let view = $state<PacksView | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);

  /** Which card or cell is mid-write, so the whole page locks rather than racing itself. */
  let busy = $state<string | null>(null);
  /** The launcher's own output after a write, and its one-line refusal when it declines. */
  let notice = $state<string | null>(null);
  let writeError = $state<string | null>(null);
  /** The Pack each harness card has selected for deploy and remove. */
  let chosen = $state<Record<string, string>>({});

  const shown = $derived(view?.harnesses.filter((h) => h.installed) ?? []);

  /** A harness nobody installed that still holds deployed copies — the condition this page exists for. */
  const orphaned = $derived(
    (view?.harnesses ?? []).filter((h) => !h.installed && deployed(h).length > 0),
  );

  const rows = $derived.by(() => {
    const keys: { pack: string; skill: string }[] = [];
    const seen = new Set<string>();
    for (const harness of shown) {
      for (const unit of harness.units) {
        const key = `${unit.pack}/${unit.skill}`;
        if (seen.has(key)) continue;
        seen.add(key);
        keys.push({ pack: unit.pack, skill: unit.skill });
      }
    }
    return keys;
  });

  function deployed(harness: HarnessView) {
    return harness.units.filter((u) => u.status !== "not-deployed");
  }

  function statusOf(harness: HarnessView, pack: string, skill: string): string {
    return harness.units.find((u) => u.pack === pack && u.skill === skill)?.status ?? "not-deployed";
  }

  /** Anything that is neither current nor absent needs a decision from a human. */
  function needsAttention(status: string): boolean {
    return status !== "current" && status !== "not-deployed";
  }

  const attention = (row: Row) => shown.some((h) => needsAttention(statusOf(h, row.pack, row.skill)));

  // One column per installed harness, so the field list follows what is on this machine.
  const fields: Field<Row>[] = $derived([
    { id: "pack", label: "Pack", kind: "select", width: "8rem", value: (r) => r.pack, cell: packCell },
    { id: "skill", label: "Skill", value: (r) => r.skill, cell: skillCell },
    {
      id: "state", label: "State", kind: "select", column: false,
      value: (r) => (attention(r) ? "attention" : "ok"),
      display: (v) => (v === "attention" ? "Needs a decision" : "In order"),
      tone: (v) => (v === "attention" ? "warning" : "muted"),
    },
    ...shown.map((h): Field<Row> => ({
      id: `h:${h.id}`, label: h.label, width: "7rem", value: (r) => statusOf(h, r.pack, r.skill), cell: harnessCell,
    })),
  ]);

  async function load(): Promise<void> {
    try {
      view = await axonStatus.packs();
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void load();
    // Slow beat: deployments are a human action, not a stream.
    const timer = setInterval(() => void load(), 30_000);
    return () => clearInterval(timer);
  });

  /** Every Pack the matrix knows, in the order the table groups them. */
  const allPacks = $derived([...new Set(rows.map((r) => r.pack))]);

  /**
   * Ask sjel-status to change a harness, then re-read the matrix whatever happened.
   *
   * The re-read is the point: the ledger just changed, and a page that only updated its own
   * button would be guessing at what the launcher did. `detail` is the launcher's own words,
   * because it knows which units it moved and this page does not.
   */
  async function write(
    action: "deploy" | "sync" | "remove",
    harness: string,
    packs: string[],
  ): Promise<void> {
    busy = `${harness}:${packs.join(",")}`;
    notice = null;
    writeError = null;
    try {
      const result = await axonStatus.writePack({ action, harness, packs });
      notice = `${action} ${result.harness} ${packs.join(", ")} — ${result.detail || "done"}`;
    } catch (e) {
      // A refusal from the launcher is the answer, not a broken page: it names the drifted
      // copy, or the Pack that already owns the destination.
      writeError = `${action} ${harness}: ${e instanceof Error ? e.message : String(e)}`;
    } finally {
      busy = null;
      await load();
    }
  }

  /** The Packs a harness holds, deduped: the units are per skill, the verbs take Pack names. */
  function heldPacks(harness: HarnessView): string[] {
    return [...new Set(deployed(harness).map((u) => u.pack))];
  }

  /** The selection a harness card is acting on, defaulting to the first Pack in the table. */
  function pick(harness: HarnessView): string {
    return chosen[harness.id] ?? allPacks[0] ?? "";
  }
</script>

{#snippet packCell(row: Row)}<span class="pack">{row.pack}</span>{/snippet}
{#snippet skillCell(row: Row)}<span class="skill">{row.skill}</span>{/snippet}
{#snippet harnessCell(row: Row, column: Column<Row>)}
  {@const harness = shown.find((h) => `h:${h.id}` === column.id)}
  {@const status = harness ? statusOf(harness, row.pack, row.skill) : "not-deployed"}
  {#if harness && (status === "outdated" || status === "missing")}
    <!-- The fix for a Pack this harness already holds, offered where the defect is. A
         drifted copy gets no button: `sync` refuses it by design, and the decision there is
         `accept` or discarding the edit, which is not a one-click act. -->
    <button
      class="btn btn-soft cell-act"
      disabled={busy !== null}
      onclick={() => write(status === "missing" ? "deploy" : "sync", harness.id, [row.pack])}
    >
      {status === "missing" ? "Deploy" : "Sync"}
    </button>
  {:else if needsAttention(status)}<Chip label={status} tone="warning" />{:else if status === "current"}<span class="cell">·</span>{/if}
{/snippet}

<PageHeader badge="Packs" title="Packs across harnesses" />

{#if loading && !view}
  <p class="loading"><Icon name="loader" /> Reading every harness…</p>
{:else if error}
  <div class="card err-card">
    <p class="err"><Icon name="alert" /> Could not read the Pack state</p>
    <p class="err-hint">sjel-status answers this by running <code>tools/harnesses status --json</code>.</p>
    <p class="err-detail">{error}</p>
  </div>
{:else if view}
  <div class="harnesses">
    {#each view.harnesses as harness (harness.id)}
      <div class="card harness" class:absent={!harness.installed}>
        <div class="harness-head">
          <span class="harness-name">{harness.label}</span>
          <span class="pill" class:on={harness.installed}>
            {harness.installed ? "installed" : "absent"}
          </span>
        </div>
        <p class="meta">{harness.model === "registry" ? "reads the Pack source in place" : "copies, and can drift"}</p>
        <p class="counts">
          <strong>{deployed(harness).length}</strong> deployed
          {#if harness.unowned.length}· <strong>{harness.unowned.length}</strong> unowned{/if}
        </p>
        <p class="path">{harness.destination ?? harness.marker}</p>
        {#if harness.installed}
          <div class="acts">
            <!-- Every Pack this harness holds, named. `sync --all` is not used: it exists on
                 some launchers and not others, and the page knows the set anyway. -->
            <button
              class="btn btn-outline"
              disabled={busy !== null || heldPacks(harness).length === 0}
              onclick={() => write("sync", harness.id, heldPacks(harness))}
            >
              <Icon name="refresh" size={12} /> Sync all
            </button>
            <!-- Deploy and remove take a Pack name and cannot be inferred from the matrix,
                 which is per skill: `tools/harnesses` has no deploy verb, and the launcher
                 takes Pack names. -->
            <select
              aria-label={`Pack for ${harness.label}`}
              onchange={(e) => (chosen[harness.id] = (e.currentTarget as HTMLSelectElement).value)}
            >
              {#each allPacks as pack (pack)}
                <option value={pack} selected={pick(harness) === pack}>{pack}</option>
              {/each}
            </select>
            <button class="btn btn-soft" disabled={busy !== null} onclick={() => write("deploy", harness.id, [pick(harness)])}>
              Deploy
            </button>
            <button class="btn btn-danger" disabled={busy !== null} onclick={() => write("remove", harness.id, [pick(harness)])}>
              Remove
            </button>
          </div>
        {/if}
      </div>
    {/each}
  </div>

  {#each orphaned as harness (harness.id)}
    <div class="card alarm">
      <p class="alarm-title">
        <Icon name="alert" />
        {harness.label} is not installed, and {deployed(harness).length} units are deployed at its skill root
      </p>
      <p class="alarm-body">
        Nothing on this machine reads them.
        <code>{harness.cli} remove {[...new Set(deployed(harness).map((u) => u.pack))].join(" ")}</code>
      </p>
    </div>
  {/each}

  <h2>The matrix</h2>
  {#if notice}
    <div class="card outcome ok-outcome"><p class="outcome-text">{notice}</p></div>
  {/if}
  {#if writeError}
    <div class="card outcome err-outcome">
      <p class="outcome-text"><Icon name="alert" size={12} /> {writeError}</p>
    </div>
  {/if}
  <Collection id="matrix" {rows} {fields} key={(r) => `${r.pack}/${r.skill}`} title={(r) => r.skill} defaults={{ group: "pack" }} />

  {#each view.harnesses as harness (harness.id)}
    {#if harness.unowned.length}
      <h2>{harness.label}: at the destination, owned by no Pack</h2>
      <ul class="strays">
        {#each harness.unowned as stray (stray.name)}
          <li>
            <span class="kind kind-{stray.kind}">{stray.kind}</span>
            <span class="stray-name">{stray.name}</span>
            {#if stray.detail}<span class="meta">{stray.detail}</span>{/if}
          </li>
        {/each}
      </ul>
      {#if harness.unowned.some((s) => s.kind === "copy")}
        <p class="meta">
          A <code>copy</code> is the only promote candidate. An <code>external</code> or a
          <code>symlink</code> belongs to another installer, and copying one into a Pack forks it.
        </p>
      {/if}
    {/if}
  {/each}

  {#if view.unsupported.length}
    <h2>No adapter</h2>
    <ul class="strays">
      {#each view.unsupported as item (item.id)}
        <li><span class="kind kind-external">unsupported</span> <span class="stray-name">{item.label}</span>
          <span class="meta">{item.why}</span></li>
      {/each}
    </ul>
  {/if}

  <p class="measured">Measured {new Date(view.measuredAt).toLocaleTimeString()}</p>
{/if}

<style>
  /* ── Loading / error ────────────────────────────────────────── */
  .loading {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    font-size: 0.85rem;
    margin: 0 0 0.9rem;
  }

  .err-card { padding: 0.85rem 1rem; margin: 0 0 1rem; }
  .err { display: flex; align-items: center; gap: 0.45rem; font-size: 0.85rem; margin: 0 0 0.3rem; color: var(--warning-ink); }
  .err-hint { font-size: var(--text-xs); color: var(--text-secondary); margin: 0 0 0.15rem; }
  .err-detail { font-size: 0.7rem; color: var(--text-tertiary); margin: 0; }

  /* ── Harness cards ──────────────────────────────────────────── */
  .harnesses {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(15rem, 1fr));
    gap: 0.7rem;
    margin: 0 0 1.1rem;
  }

  .harness { padding: 0.8rem 0.9rem; }
  .harness.absent { opacity: 0.72; }

  .harness-head { display: flex; align-items: baseline; justify-content: space-between; gap: 0.5rem; }
  .harness-name { font-weight: 600; font-size: 0.9rem; }

  .pill {
    font-size: 0.65rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 0.1rem 0.4rem;
    border-radius: var(--radius-sm);
    background: var(--warning-soft);
    color: var(--warning-ink);
  }
  .pill.on { background: var(--success-soft); color: var(--success); }

  .counts { font-size: 0.8rem; margin: 0.35rem 0 0.2rem; color: var(--text-secondary); }
  .path { font-family: var(--font-mono); font-size: 0.68rem; color: var(--text-tertiary); margin: 0; word-break: break-all; }
  .meta { font-size: 0.72rem; color: var(--text-secondary); margin: 0.15rem 0 0; }

  /* ── The card's own controls ───────────────────────────────── */
  .acts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.3rem;
    margin: 0.55rem 0 0;
  }
  .acts select {
    font-size: 0.68rem;
    padding: 0.12rem 0.3rem;
    background: var(--input-bg);
    color: var(--text-primary);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
  }
  .acts .btn { font-size: 0.68rem; padding: 0.14rem 0.45rem; }

  /* One button per matrix cell that needs a decision, so it stays smaller than the card's. */
  .cell-act { font-size: 0.65rem; padding: 0.08rem 0.35rem; }

  /* ── What the last write did ───────────────────────────────── */
  .outcome { padding: 0.5rem 0.7rem; margin: 0 0 0.6rem; }
  .outcome-text {
    display: flex;
    align-items: baseline;
    gap: 0.35rem;
    font-size: var(--text-xs);
    margin: 0;
    white-space: pre-wrap;
  }
  .ok-outcome { border-color: var(--success); }
  .ok-outcome .outcome-text { color: var(--success); }
  .err-outcome { border-color: var(--warning); }
  .err-outcome .outcome-text { color: var(--warning-ink); }

  /* ── The orphan card ────────────────────────────────────────── */
  .alarm { padding: 0.8rem 0.9rem; margin: 0 0 1rem; border-color: var(--warning); }
  .alarm-title { display: flex; align-items: center; gap: 0.45rem; font-size: 0.85rem; font-weight: 600; margin: 0 0 0.35rem; color: var(--warning-ink); }
  .alarm-body { font-size: 0.78rem; color: var(--text-secondary); margin: 0; }

  /* ── Matrix ─────────────────────────────────────────────────── */
  h2 { font-size: 0.9rem; margin: 1.4rem 0 0.5rem; }

  .pack { color: var(--text-tertiary); }
  .skill { font-weight: 500; }
  .cell { font-family: var(--font-mono); color: var(--text-secondary); }

  /* ── Strays ─────────────────────────────────────────────────── */
  .strays { list-style: none; padding: 0; margin: 0; font-size: 0.78rem; }
  .strays li { display: flex; align-items: baseline; gap: 0.5rem; padding: 0.25rem 0; border-bottom: 1px solid var(--rule); flex-wrap: wrap; }

  .kind {
    font-family: var(--font-mono);
    font-size: 0.65rem;
    padding: 0.08rem 0.35rem;
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text-secondary);
  }
  .kind-copy { background: var(--warning-soft); color: var(--warning-ink); }
  .stray-name { font-weight: 500; }

  .measured { font-size: 0.7rem; color: var(--text-tertiary); margin: 1.2rem 0 0; }
</style>
