<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import { tip } from "$lib/tip";
  import PageHeader from "$lib/PageHeader.svelte";
  import { axonStatus, macmon, type MacmonSample, type StorageReport, type UpdatesReport } from "$lib/api";
  import { formatBytes, storageView } from "$lib/systems/storage";
  import AgentPanel from "$lib/systems/AgentPanel.svelte";
  import RuntimeProfile from "$lib/systems/RuntimeProfile.svelte";
  import { applySummary, auditNeedsAttention, updatesView, versionLabel } from "$lib/systems/updates";

  let macmonState = $state<"checking" | "up" | "down">("checking");
  let sample = $state<MacmonSample | null>(null);
  let error = $state<string | null>(null);
  let pollTimer: ReturnType<typeof setInterval> | undefined;
  let topProcs = $state<Array<{ pid: number; rss_mb: number; name: string }>>([]);
  let procErr = $state(false);

  // Storage is fetched once, not on macmon's 3 s beat: `report` runs a `du` walk per class
  // and takes seconds, which is a cost a poll would pay forever. Null while in flight.
  let storage = $state<StorageReport | null>(null);
  let storageErr = $state<string | null>(null);
  const view = $derived(storage ? storageView(storage) : null);

  // Updates, fetched on demand rather than on macmon's 3 s beat for a stronger reason than
  // storage's: `report` asks three registries and one crate registry per crate, so polling it
  // would be a dozen network calls every three seconds. Null while in flight.
  let updates = $state<UpdatesReport | null>(null);
  let updatesErr = $state<string | null>(null);
  let applyErr = $state<string | null>(null);
  let applyTimer: ReturnType<typeof setInterval> | undefined;
  const updView = $derived(updates ? updatesView(updates) : null);
  const updSummary = $derived(updView ? applySummary(updView) : "");

  /** Refetch the report, and stop polling once nothing is running. */
  function loadUpdates() {
    axonStatus
      .updates()
      .then((d) => {
        updates = d;
        updatesErr = null;
        if (d.lastApply?.state !== "running" && applyTimer) {
          clearInterval(applyTimer);
          applyTimer = undefined;
        }
      })
      .catch((e) => { updatesErr = e instanceof Error ? e.message : String(e); });
  }

  /**
   * Start one class moving. The route answers 202 — started, not finished — because the
   * cargo and host-patch steps compile for minutes, and a request held that long would time
   * out in the browser while the install succeeded. So the poll below IS the progress: the
   * tool writes a receipt and every report carries it as `lastApply`.
   */
  async function applyClass(className: string) {
    applyErr = null;
    try {
      await axonStatus.updatesApply(className);
    } catch (e) {
      applyErr = e instanceof Error ? e.message : String(e);
      return;
    }
    loadUpdates();
    if (applyTimer) clearInterval(applyTimer);
    applyTimer = setInterval(loadUpdates, 4_000);
  }

  /** °C → CSS class name */
  function tempClass(celsius: number): string {
    if (celsius >= 80) return "hot";
    if (celsius >= 60) return "warm";
    return "cool";
  }

  /** Bytes → human-readable. `gb` is macmon's unit: its fields are already gigabytes. */
  function bytes(gb: number): string {
    return (gb / 1024 / 1024 / 1024).toFixed(1) + " GB";
  }

  /** Fraction 0-1 → % */
  function pct(v: number): string {
    return (v * 100).toFixed(1) + "%";
  }

  /** Power in watts */
  function watts(v: number): string {
    return v.toFixed(2) + " W";
  }

  /** Sum of top processes' RSS for the "bekannt" total. */
  const topRssTotal = $derived(topProcs.reduce((s, p) => s + p.rss_mb, 0));

  onMount(() => {
    // Fetch top memory consumers once; cold data is fine on this page since macmon
    // gives the live totals and these names change slowly.
    fetch("/api/top-processes")
      .then((r) => { if (r.ok) return r.json(); throw new Error(); })
      .then((d) => { topProcs = d; procErr = false; })
      .catch(() => { procErr = true; });

    // Served by sjel-status, not by macmon, so it answers even when macmon is down —
    // which is the state a full disk is hardest to see in.
    axonStatus
      .storage()
      .then((d) => { storage = d; storageErr = null; })
      .catch((e) => { storageErr = e instanceof Error ? e.message : String(e); });

    loadUpdates();

    const poll = () => {
      macmon
        .json()
        .then((d) => {
          sample = d;
          macmonState = "up";
          error = null;
        })
        .catch((e) => {
          macmonState = "down";
          error = e instanceof Error ? e.message : String(e);
        });
    };

    poll();
    pollTimer = setInterval(poll, 3_000);
  });

  onDestroy(() => {
    if (pollTimer) clearInterval(pollTimer);
    if (applyTimer) clearInterval(applyTimer);
  });
</script>

<PageHeader
  badge="Local machine"
  title="Systems"
  desc="What this computer is doing now: temperature, power, and utilisation."
/>

<RuntimeProfile />

<!-- ─── macmon dashboard ──────────────────────────────────────────────────── -->

{#if macmonState === "checking"}
  <p class="loading"><Icon name="loader" size={14} /> collecting data…</p>
{:else if macmonState === "down"}
  <div class="card err-card">
    <p class="err">
      <Icon name="alert" size={14} />
      macmon unavailable
    </p>
    <p class="err-hint">
      Start <code class="mono">macmon serve --port 9911</code> to show live metrics on
      the Systems page.
    </p>
    {#if error}
      <p class="err-detail mono">{error}</p>
    {/if}
  </div>
{:else if sample}
  <div class="grid">
    <!-- Temperatur -->
    <div class="card metric">
      <span class="metric-head">
        <Icon name="thermometer" size={14} />
        Temperature
      </span>
      <div class="metric-body">
        <div class="temp-row">
          <span class="temp-value {tempClass(sample.temp.cpu_temp_avg)}">
            {sample.temp.cpu_temp_avg.toFixed(0)}°
          </span>
          <span class="temp-label">CPU</span>
        </div>
        <div class="temp-row">
          <span class="temp-value {tempClass(sample.temp.gpu_temp_avg)}">
            {sample.temp.gpu_temp_avg.toFixed(0)}°
          </span>
          <span class="temp-label">GPU</span>
        </div>
      </div>
    </div>

    <!-- Leistungsaufnahme -->
    <div class="card metric">
      <span class="metric-head">
        <Icon name="activity" size={14} />
        Power
      </span>
      <div class="metric-body watts">
        <div class="watts-total">{watts(sample.all_power)}</div>
        <div class="watts-breakdown">
          <span use:tip={"CPU"}>CPU {watts(sample.cpu_power)}</span>
          <span use:tip={"GPU"}>GPU {watts(sample.gpu_power)}</span>
          <span use:tip={"RAM"}>RAM {watts(sample.ram_power)}</span>
          <span use:tip={"System (remainder)"}>Sys {watts(sample.sys_power)}</span>
        </div>
      </div>
    </div>

    <!-- CPU Auslastung -->
    <div class="card metric">
      <span class="metric-head">
        <Icon name="cpu" size={14} />
        CPU
      </span>
      <div class="metric-body">
        <div class="usage-row">
          <span class="usage-pct">{pct(sample.cpu_usage_pct)}</span>
          <span class="usage-label">total</span>
        </div>
        <div class="bar-track">
          <div class="bar-fill" style="transform: scaleX({sample.cpu_usage_pct})"></div>
        </div>
        <div class="core-row">
          <span>P-Cores <b>{sample.pcpu_usage[0]} MHz</b> {pct(sample.pcpu_usage[1])}</span>
          <span>E-Cores <b>{sample.ecpu_usage[0]} MHz</b> {pct(sample.ecpu_usage[1])}</span>
        </div>
      </div>
    </div>

    <!-- Memory, with hover detail for swap consumers. -->
    <div class="card metric mem-card">
      <span class="metric-head">
        <Icon name="database" size={14} />
        Memory
        <span class="mem-pressure" use:tip={"Swap use as an indicator of memory pressure"}>
          {#if sample.memory.swap_usage > sample.memory.swap_total * 0.5}
            <Icon name="alert" size={11} />
          {/if}
        </span>
      </span>
      <div class="metric-body">
        <div class="mem-row">
          <span>RAM</span>
          <span class="mono">{bytes(sample.memory.ram_usage)} / {bytes(sample.memory.ram_total)}</span>
        </div>
        <div class="bar-track">
          <div class="bar-fill mem" style="transform: scaleX({sample.memory.ram_usage / sample.memory.ram_total})"></div>
        </div>
        <div class="mem-row swap">
          <span>Swap</span>
          <span class="mono">{bytes(sample.memory.swap_usage)} / {bytes(sample.memory.swap_total)}</span>
        </div>
        <div class="bar-track">
          <div class="bar-fill swap" style="transform: scaleX({sample.memory.swap_usage / sample.memory.swap_total || 0})"></div>
        </div>
      </div>

      <!-- What is consuming memory, one press away. It opened on :hover and :focus-within
           until 2026-10-05, and nothing in this card takes focus, so a keyboard or touch
           reader could never open it. A native popover answers all three. -->
      <button type="button" class="mem-detail" popovertarget="mem-top">
        Largest users
        <Icon name="chevron" size={11} />
      </button>

      <div id="mem-top" class="popover mem-hover" popover>
        <div class="mem-hover-head">
          <strong>Largest RAM users</strong>
          <span class="mono">{topRssTotal} MB across the largest processes</span>
        </div>
        {#if procErr}
          <p class="mem-hint">Process list unavailable</p>
        {:else if topProcs.length === 0}
          <p class="mem-hint">loading…</p>
        {:else}
          <ol class="proc-list">
            {#each topProcs.slice(0, 8) as p (p.pid)}
              <li>
                <span class="proc-name">{p.name}</span>
                <span class="proc-bar-wrap">
                  <span class="proc-bar" style="transform: scaleX({p.rss_mb / topProcs[0].rss_mb})"></span>
                </span>
                <span class="proc-rss mono">{p.rss_mb} MB</span>
              </li>
            {/each}
          </ol>
          {#if topProcs.length > 8}
            <p class="mem-hint">+ {topProcs.length - 8} more processes above 100 MB</p>
          {/if}
          <p class="mem-hint warn">
            <Icon name="alert" size={10} />
            macOS has used {bytes(sample.memory.swap_usage)} of
            {bytes(sample.memory.swap_total)} swap. {topRssTotal >= 6000
              ? "Active processes no longer fit in RAM."
              : "The system is compressing and moving rarely used pages to swap."}
          </p>
        {/if}
      </div>
    </div>
  </div>

  <p class="ts mono">
    <Icon name="clock" size={12} />
    Last updated: {new Date(sample.timestamp).toLocaleTimeString("en-GB")}
    (every 3 s)
  </p>
{/if}

<AgentPanel />

<!-- ─── Storage ──────────────────────────────────────────────────────────
     Its own section rather than a card in the metric grid above, and outside
     the macmon condition on purpose: this comes from sjel-status, and "what
     fills the disk" has to stay readable when macmon is down. ─────────────── -->
<section class="storage">
  <h2 class="section-head">
    <Icon name="hard-drive" size={14} />
    Storage
    {#if view}
      <span class="state {view.state}">{view.stateLabel}</span>
    {/if}
  </h2>

  {#if storageErr}
    <div class="card err-card">
      <p class="err">
        <Icon name="alert" size={14} />
        Storage report unavailable
      </p>
      <p class="err-detail mono">{storageErr}</p>
    </div>
  {:else if !view}
    <p class="loading"><Icon name="loader" size={14} /> measuring the disk…</p>
  {:else}
    <div class="vol">
      <div class="bar-track">
        <div
          class="bar-fill"
          class:warn={view.state === "warn"}
          class:crit={view.state === "critical"}
          style="transform: scaleX({view.usedPct / 100})"
        ></div>
      </div>
      <p class="vol-line">
        <span class="mono">{view.used} used of {view.total}</span>
        <span class="mono dim">{view.free} free</span>
      </p>
    </div>

    <div class="stor-cols">
      <div>
        <h3 class="col-head">Reclaimable by class</h3>
        {#if view.classes.length === 0}
          <p class="mem-hint">Nothing measured in any class.</p>
        {:else}
          <ul class="stor-list">
            {#each view.classes as row (row.name)}
              <li class="stor-row">
                <span class="stor-name">
                  <!-- title because the column is a fraction of the row and a class name is the
                       row's key: the tool knows headless-browser-payloads and
                       chrome-on-device-models, both longer than it holds. -->
                  <span class="mono" use:tip={row.name}>{row.name}</span>
                  {#if !row.applicable}<span class="tag">report-only</span>{/if}
                  {#if row.flagged}<span class="tag warn">over flag</span>{/if}
                </span>
                <span class="stor-bar">
                  <span
                    class="stor-bar-fill"
                    class:flag={row.flagged}
                    style="width: {Math.min(100, (row.bytes / view.classes[0].bytes) * 100)}%"
                  ></span>
                </span>
                <span class="stor-bytes mono">{formatBytes(row.bytes)}</span>
              </li>
            {/each}
          </ul>
          <p class="mem-hint">
            {view.reclaimableLabel} reclaimable — <code class="mono">sjel storage apply</code>
          </p>
        {/if}
      </div>

      <div>
        <h3 class="col-head">Protected — reported, never cleaned</h3>
        <ul class="stor-list">
          {#each view.protected as row (row.path)}
            <li class="stor-row protected">
              <span class="stor-name mono" use:tip={row.path}>{row.path}</span>
              <span class="stor-bytes mono">{formatBytes(row.bytes)}</span>
              <span class="stor-reason">{row.reason}</span>
            </li>
          {/each}
        </ul>
        {#if view.protectedUnmeasured > 0}
          <p class="mem-hint">
            {view.protectedUnmeasured} of these could not be read by the measuring user, so
            their size reads 0 MB.
          </p>
        {/if}
      </div>
    </div>
  {/if}
</section>

<!-- ─── Updates ──────────────────────────────────────────────────────────
     The same delegation shape as Storage above: `tools/updates` measures and
     sjel-status serves it, so the ownership table cannot drift from the tool's
     own. The Apply button is the only write on this page, and it answers 202 —
     the outcome arrives on the next poll as `lastApply`, which is why the panel
     shows what the last run did rather than a spinner that outlives it. ──────── -->
<section class="updates">
  <h2 class="section-head">
    <Icon name="boxes" size={14} />
    Updates
    {#if updView}
      <span class="state" class:warn={updView.stale > 0}>
        {updView.stale > 0 ? `${updView.stale} stale` : "nothing stale"}
      </span>
    {/if}
  </h2>

  {#if updatesErr}
    <div class="card err-card">
      <p class="err">
        <Icon name="alert" size={14} />
        Update report unavailable
      </p>
      <p class="err-detail mono">{updatesErr}</p>
    </div>
  {:else if !updView}
    <p class="loading"><Icon name="loader" size={14} /> asking three registries…</p>
  {:else}
    {#if updSummary}
      <p
        class="upd-summary"
        class:running={updView.busy}
        class:warn={auditNeedsAttention(updView.lastApply?.audit)}
      >
        {#if updView.busy}<Icon name="loader" size={12} />{/if}
        {updSummary}
      </p>
    {/if}
    {#if applyErr}
      <p class="err-detail mono">{applyErr}</p>
    {/if}

    {#each updView.groups as group (group.surface.id)}
      <div class="upd-group">
        <h3 class="col-head">
          <span use:tip={group.surface.why}>{group.surface.title}</span>
          <span class="dim mono">{group.surface.ownerDetail}</span>
          {#if group.actionable > 0 && !updView.busy}
            <button class="btn btn-soft" onclick={() => applyClass(group.surface.id)}>
              <Icon name="refresh" size={12} />
              Apply {group.actionable}
            </button>
          {/if}
        </h3>
        <ul class="upd-list">
          {#each group.rows as row (`${row.surface}-${row.name}`)}
            <li class="upd-row">
              <span class="upd-mark {row.status}">
                {#if row.status === "stale"}
                  <Icon name="alert" size={12} />
                {:else if row.status === "current"}
                  <Icon name="check" size={12} />
                {:else if row.status === "unknown"}
                  <span class="mono">?</span>
                {:else}
                  <span class="mono">·</span>
                {/if}
              </span>
              <span class="upd-name mono" use:tip={row.name}>{row.name}</span>
              <span class="upd-vers mono dim">{versionLabel(row)}</span>
              <span class="upd-note dim">{row.note}</span>
            </li>
          {/each}
        </ul>
      </div>
    {/each}

    {#if updView.unknown > 0}
      <p class="mem-hint">
        {updView.unknown} row(s) were not checked — not stale, but not confirmed current either.
      </p>
    {/if}
  {/if}
</section>

<style>
  /* ── Loading / error ────────────────────────────────────────── */
  .loading {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    font-size: 0.85rem;
    margin: 0 0 0.9rem;
  }

  .err-card {
    padding: 0.85rem 1rem;
    margin: 0 0 1rem;
  }

  .err {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    font-size: 0.85rem;
    margin: 0 0 0.3rem;
    color: var(--warning-ink);
  }

  .err-hint {
    font-size: var(--text-xs);
    color: var(--text-secondary);
    margin: 0 0 0.15rem;
  }

  .err-detail {
    font-size: 0.7rem;
    color: var(--text-tertiary);
    margin: 0;
  }

  /* ── Metric grid ────────────────────────────────────────────── */
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(17rem, 1fr));
    gap: 0.75rem;
    margin: 0 0 0.5rem;
  }

  .metric {
    padding: 0.85rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .metric-head {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-secondary);
  }

  .metric-body {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  /* ── Temperature ────────────────────────────────────────────── */
  .temp-row {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
  }

  .temp-value {
    font-size: 1.5rem;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    line-height: 1;
  }

  .temp-value.cool {
    color: var(--success);
  }

  .temp-value.warm {
    color: var(--warning-ink);
  }

  .temp-value.hot {
    color: var(--danger);
  }

  .temp-label {
    font-size: var(--text-xs);
    color: var(--text-tertiary);
    text-transform: uppercase;
    font-weight: 500;
  }

  /* ── Power ──────────────────────────────────────────────────── */
  .watts-total {
    font-size: 1.5rem;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    line-height: 1;
  }

  .watts-breakdown {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem 0.7rem;
    font-size: 0.72rem;
    color: var(--text-tertiary);
  }

  /* ── CPU / Usage bars ───────────────────────────────────────── */
  .usage-row {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
  }

  .usage-pct {
    font-size: 1.5rem;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    line-height: 1;
  }

  .usage-label {
    font-size: var(--text-xs);
    color: var(--text-tertiary);
    text-transform: uppercase;
    font-weight: 500;
  }

  .core-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem 0.9rem;
    font-size: 0.72rem;
    color: var(--text-secondary);
  }

  .core-row b {
    font-weight: 600;
    color: var(--text-primary);
  }

  /* ── Bar track ──────────────────────────────────────────────── */
  .bar-track {
    height: 0.35rem;
    border-radius: 999px;
    background-color: var(--surface);
    overflow: hidden;
  }

  .bar-fill {
    height: 100%;
    border-radius: 999px;
    background-color: var(--primary);
    /* scaleX, not width: macmon refreshes every 3 s, and a width transition re-lays-out
       the card each time. The track clips, so the scaled radius never shows. */
    transform-origin: left;
    transition: transform var(--motion-slow) var(--ease-out);
  }

  .bar-fill.mem {
    background-color: var(--primary);
  }

  .bar-fill.swap {
    background-color: var(--accent);
  }

  /* ── Memory ─────────────────────────────────────────────────── */
  .mem-card {
    position: relative;
  }

  .mem-pressure {
    margin-left: auto;
    display: flex;
    align-items: center;
    color: var(--warning-ink);
  }

  .mem-row {
    display: flex;
    justify-content: space-between;
    font-size: var(--text-sm);
  }

  .mem-row.swap {
    font-size: var(--text-xs);
    color: var(--text-tertiary);
    margin-top: 0.2rem;
  }

  /* ── Popover: who is eating memory ─────────────────────────── */
  .mem-detail {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    margin-top: var(--space-3);
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-tertiary);
    font: inherit;
    font-size: var(--text-xs);
    cursor: pointer;
    anchor-name: --mem-top;
  }

  .mem-detail:hover {
    color: var(--primary);
  }

  .mem-hover {
    position-anchor: --mem-top;
    width: min(26rem, calc(100vw - 2 * var(--space-4)));
    max-width: none;
  }

  .mem-hover-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 0.5rem;
    font-size: var(--text-xs);
    margin-bottom: 0.5rem;
    padding-bottom: 0.4rem;
    border-bottom: 1px solid var(--card-border);
  }

  .mem-hover-head span {
    font-size: 0.65rem;
    color: var(--text-tertiary);
  }

  .proc-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .proc-list li {
    display: grid;
    grid-template-columns: minmax(5rem, 1fr) 1.5fr 4rem;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.7rem;
  }

  .proc-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }

  .proc-bar-wrap {
    height: 0.35rem;
    border-radius: 999px;
    background: var(--surface);
    overflow: hidden;
  }

  /* Relative to the largest process, so the ranking reads at a glance. It was an inline
     span with a width until 2026-10-05, which rendered at 0x0: no bar ever showed. */
  .proc-bar {
    display: block;
    height: 100%;
    background: var(--primary);
    transform-origin: left;
  }

  .proc-rss {
    text-align: right;
    font-size: 0.65rem;
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .mem-hint {
    margin: 0.45rem 0 0;
    font-size: 0.65rem;
    color: var(--text-tertiary);
    line-height: 1.4;
  }

  .mem-hint.warn {
    display: flex;
    align-items: start;
    gap: 0.3rem;
    margin-top: 0.55rem;
    padding-top: 0.45rem;
    border-top: 1px solid var(--card-border);
    color: var(--text-secondary);
  }

  /* ── Timestamp ──────────────────────────────────────────────── */
  .ts {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    margin: 0 0 1.5rem;
  }

  /* ── Storage ────────────────────────────────────────────────── */
  .section-head {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-secondary);
    margin: 1.25rem 0 0.75rem;
  }

  .state {
    margin-left: auto;
    padding: 0.05rem 0.4rem;
    border: 1px solid var(--card-border);
    border-radius: 999px;
    color: var(--text-secondary);
  }

  .state.warn {
    color: var(--warning-ink);
    border-color: var(--warning-ink);
  }

  .state.critical {
    color: var(--danger);
    border-color: var(--danger);
  }

  .vol {
    margin: 0 0 1rem;
  }

  .bar-fill.warn {
    background-color: var(--warning-ink);
  }

  .bar-fill.crit {
    background-color: var(--danger);
  }

  .vol-line {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    font-size: var(--text-xs);
    color: var(--text-secondary);
    margin: 0.35rem 0 0;
  }

  .dim {
    color: var(--text-tertiary);
  }

  .stor-cols {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(19rem, 1fr));
    gap: 0.75rem 1.5rem;
  }

  .col-head {
    font-size: var(--text-2xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-tertiary);
    margin: 0 0 0.4rem;
  }

  .stor-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .stor-row {
    display: grid;
    grid-template-columns: minmax(7rem, 1.2fr) 1.5fr 4.5rem;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.72rem;
  }

  /* A protected path needs a second line for its reason; a class row does not. */
  .stor-row.protected {
    grid-template-columns: minmax(7rem, 1.2fr) 4.5rem;
    grid-template-areas: "name bytes" "reason reason";
    row-gap: 0.1rem;
  }

  .stor-row.protected .stor-name {
    grid-area: name;
  }

  .stor-row.protected .stor-bytes {
    grid-area: bytes;
  }

  .stor-name {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    min-width: 0;
  }

  .stor-name.mono,
  .stor-row > .stor-name > .mono {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .tag {
    flex: none;
    padding: 0.05rem 0.3rem;
    border: 1px solid var(--card-border);
    border-radius: 999px;
    font-size: 0.6rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-tertiary);
  }

  .tag.warn {
    color: var(--warning-ink);
    border-color: var(--warning-ink);
  }

  .stor-bar {
    height: 0.35rem;
    border-radius: 999px;
    background: var(--surface);
    overflow: hidden;
  }

  .stor-bar-fill {
    display: block;
    height: 100%;
    border-radius: 999px;
    background: var(--primary-soft);
  }

  .stor-bar-fill.flag {
    background: var(--warning-ink);
  }

  .stor-bytes {
    text-align: right;
    font-size: 0.68rem;
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .stor-reason {
    grid-area: reason;
    font-size: 0.65rem;
    color: var(--text-tertiary);
    line-height: 1.35;
  }

  /* ── Updates ────────────────────────────────────────────────────
     A class per group, rows inside it. The grid is name · version · note,
     with the note taking what is left — the notes carry the tool's own words
     (a receipt's age, why a pre-release was skipped) and are the reason this
     panel is worth reading rather than a count of stale packages. ────────── */
  .updates {
    margin-top: 1.4rem;
  }

  .upd-summary {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.72rem;
    color: var(--text-secondary);
    margin: 0 0 0.7rem;
  }

  .upd-summary.running {
    color: var(--text-primary);
  }

  /* The last apply's audit found something, or could not run. It colours the line that already
     carries the verdict rather than adding a second element, so a bad verdict is read in the
     same glance as what the apply did. */
  .upd-summary.warn {
    color: var(--warning-ink);
  }

  .upd-group {
    margin-bottom: 0.9rem;
  }

  .upd-group .col-head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .upd-group .col-head .btn {
    margin-left: auto;
    font-size: 0.68rem;
    padding: 0.15rem 0.5rem;
  }

  .upd-list {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .upd-row {
    display: grid;
    grid-template-columns: 1rem minmax(8rem, auto) minmax(6rem, auto) 1fr;
    align-items: baseline;
    gap: 0.6rem;
    padding: 0.3rem 0;
    border-top: 1px solid var(--rule);
    font-size: 0.72rem;
  }

  .upd-mark {
    display: flex;
    align-items: center;
    color: var(--text-tertiary);
  }

  .upd-mark.stale {
    color: var(--warning-ink);
  }

  .upd-mark.current {
    color: var(--success);
  }

  .upd-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .upd-vers {
    font-variant-numeric: tabular-nums;
  }

  .upd-note {
    font-size: 0.66rem;
    line-height: 1.35;
  }

  /* A narrow window is the phone, and four columns do not fit: the note wraps
     under the name rather than being clipped, because the note is the content. */
  @media (max-width: 640px) {
    .upd-row {
      grid-template-columns: 1rem 1fr auto;
    }

    .upd-note {
      grid-column: 2 / -1;
    }
  }
</style>
