<script lang="ts">
  // What Sjel has put into each agent harness on this machine.
  //
  // The whole page is one GET. sjel-status shells `tools/harnesses status --json`, which
  // is the same code path the CLI and the session hook read, so this view and the terminal
  // can never disagree about what is deployed.
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import { axonStatus, type PacksView, type HarnessView } from "$lib/api";

  let view = $state<PacksView | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);

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
</script>

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
  <table>
    <thead>
      <tr>
        <th scope="col">Pack</th>
        <th scope="col">Skill</th>
        {#each shown as harness (harness.id)}<th scope="col">{harness.label}</th>{/each}
      </tr>
    </thead>
    <tbody>
      {#each rows as row (row.pack + "/" + row.skill)}
        <tr>
          <td class="pack">{row.pack}</td>
          <td class="skill">{row.skill}</td>
          {#each shown as harness (harness.id)}
            {@const status = statusOf(harness, row.pack, row.skill)}
            <td class="cell" class:attention={needsAttention(status)}>
              {status === "current" ? "·" : status === "not-deployed" ? "" : status}
            </td>
          {/each}
        </tr>
      {/each}
    </tbody>
  </table>

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

  /* ── The orphan card ────────────────────────────────────────── */
  .alarm { padding: 0.8rem 0.9rem; margin: 0 0 1rem; border-color: var(--warning); }
  .alarm-title { display: flex; align-items: center; gap: 0.45rem; font-size: 0.85rem; font-weight: 600; margin: 0 0 0.35rem; color: var(--warning-ink); }
  .alarm-body { font-size: 0.78rem; color: var(--text-secondary); margin: 0; }

  /* ── Matrix ─────────────────────────────────────────────────── */
  h2 { font-size: 0.9rem; margin: 1.4rem 0 0.5rem; }

  table { width: 100%; border-collapse: collapse; font-size: 0.78rem; }
  th { text-align: left; font-weight: 600; color: var(--text-secondary); padding: 0.3rem 0.5rem; border-bottom: 1px solid var(--rule); }
  td { padding: 0.28rem 0.5rem; border-bottom: 1px solid var(--rule); }
  .pack { color: var(--text-tertiary); }
  .skill { font-weight: 500; }
  .cell { font-family: var(--font-mono); color: var(--text-secondary); }
  .cell.attention { color: var(--warning-ink); font-weight: 600; }

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
