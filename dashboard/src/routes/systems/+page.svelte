<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import { axonStatus, macmon, type MacmonSample, type StorageReport } from "$lib/api";
  import { formatBytes, storageView } from "$lib/systems/storage";

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
  });
</script>

<PageHeader
  badge="Local machine"
  title="Systems"
  desc="What this computer is doing now: temperature, power, and utilisation."
/>

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
          <span title="CPU">CPU {watts(sample.cpu_power)}</span>
          <span title="GPU">GPU {watts(sample.gpu_power)}</span>
          <span title="RAM">RAM {watts(sample.ram_power)}</span>
          <span title="System (remainder)">Sys {watts(sample.sys_power)}</span>
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
          <div class="bar-fill" style="width: {sample.cpu_usage_pct * 100}%"></div>
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
        <span class="mem-pressure" title="Swap use as an indicator of memory pressure">
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
          <div class="bar-fill mem" style="width: {sample.memory.ram_usage / sample.memory.ram_total * 100}%"></div>
        </div>
        <div class="mem-row swap">
          <span>Swap</span>
          <span class="mono">{bytes(sample.memory.swap_usage)} / {bytes(sample.memory.swap_total)}</span>
        </div>
        <div class="bar-track">
          <div class="bar-fill swap" style="width: {sample.memory.swap_usage / sample.memory.swap_total * 100}%"></div>
        </div>
      </div>

      <!-- Hover popover: what is consuming memory. -->
      <div class="mem-hover">
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
                  <span class="proc-bar" style="width:{Math.min(100, p.rss_mb / 8)}%"></span>
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
          style="width: {view.usedPct}%"
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
                  <span class="mono" title={row.name}>{row.name}</span>
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
              <span class="stor-name mono" title={row.path}>{row.path}</span>
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
    transition: width 0.5s ease;
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

  /* ── Hover popover: who is eating memory ───────────────────── */
  .mem-hover {
    display: none;
    position: absolute;
    top: calc(100% + 0.5rem);
    left: 0;
    right: 0;
    z-index: 20;
    padding: 0.85rem 1rem;
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    box-shadow: var(--card-shadow-hover);
  }

  .mem-card:hover .mem-hover,
  .mem-card:focus-within .mem-hover {
    display: block;
  }

  /* Keep hover stable: don't disappear when cursor moves from card to popover */
  .mem-card:hover .mem-hover:hover {
    display: block;
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

  .proc-bar {
    height: 100%;
    border-radius: 999px;
    background: var(--primary-soft);
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
</style>
