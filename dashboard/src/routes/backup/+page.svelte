<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import {
    axonStatus,
    type BackupStatus,
    type BackupTargetView,
    type BackupRunAttempt,
  } from "$lib/api";

  let loading = $state(true);
  let error = $state<string | null>(null);
  let backups = $state<BackupStatus[]>([]);
  let targets = $state<BackupTargetView[]>([]);
  let runs = $state<BackupRunAttempt[]>([]);
  let runningCount = $state(0);

  // Per-target UI state
  let verifying = $state<Record<string, boolean>>({});
  let verifyMessage = $state<Record<string, { verdict: string; detail: string } | null>>({});
  let targetIntervals = $state<Record<string, number | null>>({});
  let savingPolicy = $state<Record<string, boolean>>({});
  let customIntervalInput = $state<Record<string, string>>({});
  let isCustom = $state<Record<string, boolean>>({});

  // Per-capability UI state
  let selectedTarget = $state<Record<string, string>>({});
  let confirming = $state<string | null>(null);
  let backingUp = $state<Record<string, boolean>>({});

  function timeAgo(seconds: number | null): string {
    if (seconds === null || seconds === undefined) return "never";
    if (seconds < 60) return `${Math.max(0, Math.floor(seconds))}s ago`;
    const mins = Math.floor(seconds / 60);
    if (mins < 60) return `${mins}m ago`;
    const hours = Math.floor(seconds / 3600);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(seconds / 86400);
    return `${days}d ago`;
  }

  function attemptAge(attempt: BackupRunAttempt): number {
    if (attempt.started_epoch) {
      const nowEpoch = Math.floor(Date.now() / 1000);
      return Math.max(0, nowEpoch - attempt.started_epoch);
    }
    if (attempt.started_at) {
      const started = new Date(attempt.started_at).getTime();
      return Math.max(0, Math.floor((Date.now() - started) / 1000));
    }
    return 0;
  }

  function formatBytes(bytes: number | null): string {
    if (bytes === null || bytes === undefined) return "—";
    if (bytes === 0) return "0 B";
    const units = ["B", "KB", "MB", "GB", "TB"];
    const i = Math.floor(Math.log(bytes) / Math.log(1024));
    return `${(bytes / Math.pow(1024, i)).toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
  }

  function formatTime(iso: string | null): string {
    if (!iso) return "—";
    const d = new Date(iso);
    if (isNaN(d.getTime())) return iso;
    return d.toLocaleString("en-GB", {
      month: "short",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  }

  async function reloadData(showLoading = true): Promise<void> {
    if (showLoading && loading) loading = true;
    try {
      const [backupsRes, targetsRes, runsRes] = await Promise.all([
        axonStatus.backups(),
        axonStatus.backupTargets(),
        axonStatus.backupRuns(50),
      ]);
      backups = backupsRes.backups;
      targets = targetsRes.targets;
      runs = runsRes.runs;
      runningCount = runsRes.running;

      // Sync interval state for each target
      for (const t of targets) {
        if (targetIntervals[t.id] === undefined) {
          targetIntervals[t.id] = t.interval_hours;
          const knownPresets = [null, 12, 24, 48, 168];
          if (t.interval_hours !== null && !knownPresets.includes(t.interval_hours)) {
            isCustom[t.id] = true;
            customIntervalInput[t.id] = String(t.interval_hours);
          }
        }
      }
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void reloadData(true);
    const interval = setInterval(() => void reloadData(false), 5_000);
    return () => clearInterval(interval);
  });

  async function handleVerifyTarget(targetId: string): Promise<void> {
    verifying[targetId] = true;
    verifyMessage[targetId] = null;
    try {
      const res = await axonStatus.verifyBackupTarget(targetId);
      verifyMessage[targetId] = { verdict: res.verdict, detail: res.detail };
      await reloadData(false);
    } catch (e) {
      verifyMessage[targetId] = {
        verdict: "failed",
        detail: e instanceof Error ? e.message : String(e),
      };
    } finally {
      verifying[targetId] = false;
    }
  }

  async function handleSavePolicy(targetId: string): Promise<void> {
    savingPolicy[targetId] = true;
    try {
      let hours: number | null = targetIntervals[targetId];
      if (isCustom[targetId]) {
        const parsed = parseInt(customIntervalInput[targetId], 10);
        hours = isNaN(parsed) || parsed <= 0 ? null : parsed;
        targetIntervals[targetId] = hours;
      }
      await axonStatus.setBackupPolicy(targetId, hours);
      await reloadData(false);
    } catch (e) {
      error = `save policy: ${e instanceof Error ? e.message : String(e)}`;
    } finally {
      savingPolicy[targetId] = false;
    }
  }

  async function handleBackup(name: string): Promise<void> {
    confirming = null;
    backingUp[name] = true;
    error = null;
    try {
      const target = selectedTarget[name] || undefined;
      await axonStatus.backup(name, target);
    } catch (e) {
      error = `backup ${name}: ${e instanceof Error ? e.message : String(e)}`;
    } finally {
      backingUp[name] = false;
      await reloadData(false);
    }
  }

  function stateBadgeClass(state: string): string {
    if (state === "fresh") return "badge-ok";
    if (state === "advised") return "badge-info";
    if (state === "stale") return "badge-warn";
    if (state === "failed") return "badge-err";
    return "badge-dim";
  }
</script>

<PageHeader
  badge="Operations"
  title="Backup"
  desc="Every capability's backup contract, declared targets, interval policy, and attempt history."
/>

{#if error}
  <div class="error-banner">
    <Icon name="alert" size={16} />
    <span>{error}</span>
  </div>
{/if}

<!-- ─── Section 1: Backup Targets & Scheduled Cadence ──────────────────────── -->
<section class="section">
  <div class="section-head">
    <div class="head-titles">
      <h2>Backup Targets & Scheduled Cadence</h2>
      <p class="section-desc">
        Configured backup targets, rehearsal verification status, and autonomous sweep intervals.
      </p>
    </div>
  </div>

  {#if loading && targets.length === 0}
    <p class="loading"><Icon name="loader" size={14} /> Loading backup targets…</p>
  {:else if targets.length === 0}
    <div class="card empty-card">
      <p>No backup targets are declared on this machine.</p>
    </div>
  {:else}
    <div class="targets-grid">
      {#each targets as target (target.id)}
        {@const currentHours = targetIntervals[target.id] ?? null}
        {@const changed = currentHours !== target.interval_hours}
        <div class="card target-card">
          <div class="target-top">
            <div class="target-ident">
              <span class="target-name">{target.id}</span>
              <span class="tag mono">{target.kind}</span>
              {#if target.present === "true"}
                <span class="badge badge-ok">present</span>
              {:else if target.present === "false"}
                <span class="badge badge-err">missing</span>
              {:else}
                <span class="badge badge-dim">{target.present}</span>
              {/if}
            </div>

            <button
              class="btn btn-outline btn-sm"
              disabled={verifying[target.id]}
              onclick={() => handleVerifyTarget(target.id)}
              title="Run rehearsal verification"
            >
              {#if verifying[target.id]}
                <Icon name="loader" size={13} /> Verifying…
              {:else}
                <Icon name="refresh" size={13} /> Verify target
              {/if}
            </button>
          </div>

          <div class="target-meta mono">
            <span class="meta-label">Path:</span>
            <span class="meta-val">{target.path}</span>
            {#if target.host}
              <span class="meta-label">Host:</span>
              <span class="meta-val">{target.host}</span>
            {/if}
          </div>

          <!-- Rehearsal verdict -->
          {#if verifyMessage[target.id]}
            {@const vm = verifyMessage[target.id]!}
            <div class="verdict-banner" class:verdict-ok={vm.verdict === "verified"} class:verdict-err={vm.verdict !== "verified"}>
              <strong>Verification {vm.verdict}:</strong> {vm.detail}
            </div>
          {:else if target.verified_verdict}
            <div class="verdict-banner" class:verdict-ok={target.verified_verdict === "verified"} class:verdict-err={target.verified_verdict === "failed"}>
              <span class="verdict-status">Verified {target.verified_verdict}:</span>
              <span class="verdict-detail">{target.verified_detail ?? "rehearsal pass"}</span>
              {#if target.verified_at}
                <span class="verdict-time mono">({formatTime(target.verified_at)})</span>
              {/if}
            </div>
          {/if}

          <!-- Interval Control -->
          <div class="policy-control">
            <div class="policy-label">
              <Icon name="clock" size={13} />
              <span>Cadence:</span>
            </div>

            <div class="policy-actions">
              <select
                class="input policy-select"
                aria-label="Cadence for target {target.id}"
                value={isCustom[target.id] ? "custom" : (targetIntervals[target.id] ?? "off")}
                onchange={(e) => {
                  const val = (e.target as HTMLSelectElement).value;
                  if (val === "custom") {
                    isCustom[target.id] = true;
                    if (!customIntervalInput[target.id]) {
                      customIntervalInput[target.id] = String(target.interval_hours ?? 24);
                    }
                  } else {
                    isCustom[target.id] = false;
                    targetIntervals[target.id] = val === "off" ? null : parseInt(val, 10);
                  }
                }}
              >
                <option value="off">Off (disabled)</option>
                <option value="12">12h</option>
                <option value="24">24h (daily)</option>
                <option value="48">48h (2 days)</option>
                <option value="168">168h (weekly)</option>
                <option value="custom">Custom…</option>
              </select>

              {#if isCustom[target.id]}
                <div class="custom-hours-wrap">
                  <input
                    type="number"
                    min="1"
                    aria-label="Custom cadence hours for target {target.id}"
                    class="input custom-hours-input mono"
                    placeholder="hours"
                    bind:value={customIntervalInput[target.id]}
                  />
                  <span class="hours-suffix">h</span>
                </div>
              {/if}

              {#if changed || isCustom[target.id]}
                <button
                  class="btn btn-primary btn-sm"
                  disabled={savingPolicy[target.id]}
                  onclick={() => handleSavePolicy(target.id)}
                >
                  {#if savingPolicy[target.id]}
                    <Icon name="loader" size={12} />
                  {:else}
                    Save
                  {/if}
                </button>
              {/if}
            </div>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</section>

<!-- ─── Section 2: Capability Contracts & Manual Runs ──────────────────────── -->
<section class="section">
  <div class="section-head">
    <div class="head-titles">
      <h2>Capability Contracts & Manual Runs</h2>
      <p class="section-desc">
        Declared backup contracts, receipt freshness, and manual trigger controls.
      </p>
    </div>
  </div>

  {#if loading && backups.length === 0}
    <p class="loading"><Icon name="loader" size={14} /> Loading capability contracts…</p>
  {:else if backups.length === 0}
    <div class="card empty-card">
      <p>No capabilities declare a backup contract on this machine.</p>
    </div>
  {:else}
    <ul class="contracts-list">
      {#each backups as b (b.capability)}
        {@const isRunning = backingUp[b.capability] || b.run?.state === "running"}
        {@const isConfirming = confirming === b.capability}
        {@const attemptFailed = b.attempt && b.attempt.exit_code !== 0 && b.attempt.exit_code !== null}
        <li class="card contract-card">
          <div class="contract-main">
            <div class="contract-header">
              <div class="contract-title">
                <span class="cap-name">{b.capability}</span>
                <span class="badge {stateBadgeClass(b.state)}">{b.state}</span>
                {#if b.holds_service}
                  <span class="tag mono holds-tag" title="Requires stopping service for cold copy">holds-service</span>
                {/if}
              </div>

              <div class="contract-receipt mono">
                {#if b.last_success}
                  <span>backed up {timeAgo(b.age_seconds)}</span>
                  {#if b.bytes}
                    <span class="dim">({formatBytes(b.bytes)})</span>
                  {/if}
                {:else}
                  <span class="dim">never backed up</span>
                {/if}
                {#if b.advise_days || b.stale_days}
                  <span class="cadence-rule">
                    [advise: {b.advise_days ?? "—"}d, stale: {b.stale_days ?? "—"}d]
                  </span>
                {/if}
              </div>
            </div>

            <!-- Last attempt failure finding (A fresh receipt can no longer mask a failing run) -->
            {#if attemptFailed}
              <div class="attempt-failure">
                <div class="attempt-failure-title">
                  <Icon name="alert" size={14} />
                  <strong>The last attempt FAILED {timeAgo(attemptAge(b.attempt!))} (exit {b.attempt!.exit_code})</strong>
                  <span class="tag mono">target: {b.attempt!.target}</span>
                </div>
                {#if b.attempt!.detail}
                  <p class="attempt-failure-detail mono">{b.attempt!.detail}</p>
                {/if}
                {#if b.attempt!.log_path}
                  <p class="attempt-failure-log mono">Log: {b.attempt!.log_path}</p>
                {/if}
              </div>
            {/if}

            {#if isConfirming}
              <div class="cold-copy-warning">
                <Icon name="alert" size={14} />
                <div class="warning-text">
                  A backup takes a cold copy, so <strong>{b.capability}</strong> stops for the duration
                  and restarts automatically once the archive is written.
                </div>
              </div>
            {/if}
          </div>

          <div class="contract-actions">
            {#if isRunning}
              <span class="btn btn-outline" aria-busy="true">
                <Icon name="loader" size={14} /> Backing up…
              </span>
            {:else if isConfirming}
              <div class="confirm-group">
                <button
                  class="btn btn-danger btn-sm"
                  onclick={() => handleBackup(b.capability)}
                >
                  Stop and back up
                </button>
                <button class="btn btn-outline btn-sm" onclick={() => (confirming = null)}>
                  Cancel
                </button>
              </div>
            {:else}
              <div class="trigger-group">
                {#if targets.length > 1}
                  <select
                    class="input target-select"
                    bind:value={selectedTarget[b.capability]}
                    aria-label="Target for {b.capability}"
                  >
                    <option value="">Default target</option>
                    {#each targets as t}
                      <option value={t.id}>{t.id} ({t.kind})</option>
                    {/each}
                  </select>
                {/if}

                <button
                  class="btn btn-outline btn-sm"
                  onclick={() => (b.holds_service ? (confirming = b.capability) : handleBackup(b.capability))}
                >
                  <Icon name="database" size={13} /> Back up now
                </button>
              </div>
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<!-- ─── Section 3: Recent Runs Ledger ──────────────────────────────────────── -->
<section class="section">
  <div class="section-head">
    <div class="head-titles">
      <h2>Recent Runs Ledger</h2>
      <p class="section-desc">
        Durable attempt ledger including failure exits, archive metrics, and command logs.
      </p>
    </div>
    {#if runningCount > 0}
      <span class="running-indicator">
        <Icon name="loader" size={13} /> {runningCount} in flight
      </span>
    {/if}
  </div>

  {#if loading && runs.length === 0}
    <p class="loading"><Icon name="loader" size={14} /> Loading run history…</p>
  {:else if runs.length === 0}
    <div class="card empty-card">
      <p>No recent backup runs recorded.</p>
    </div>
  {:else}
    <div class="card table-wrap">
      <table class="table">
        <thead>
          <tr>
            <th scope="col">Started</th>
            <th scope="col">Capability</th>
            <th scope="col">Target</th>
            <th scope="col">Duration</th>
            <th scope="col">Outcome</th>
            <th scope="col">Archive</th>
            <th scope="col">Detail & Log</th>
          </tr>
        </thead>
        <tbody>
          {#each runs as run (run.id ?? `${run.started_at}-${run.capability}`)}
            {@const isSuccess = run.exit_code === 0}
            {@const isRunActive = run.exit_code === null && run.finished_at === null}
            <tr>
              <td class="mono num num-cell">{formatTime(run.started_at)}</td>
              <td><strong>{run.capability}</strong></td>
              <td><span class="mono tag">{run.target}</span></td>
              <td class="mono num num-cell">
                {#if run.finished_epoch && run.started_epoch}
                  {Math.max(0, run.finished_epoch - run.started_epoch)}s
                {:else if isRunActive}
                  <span class="active-pulse"><Icon name="loader" size={11} /> active</span>
                {:else}
                  —
                {/if}
              </td>
              <td>
                {#if isSuccess}
                  <span class="badge badge-ok">exit 0</span>
                {:else if isRunActive}
                  <span class="badge badge-info"><Icon name="loader" size={11} /> running</span>
                {:else}
                  <span class="badge badge-err">exit {run.exit_code ?? "?"}</span>
                {/if}
              </td>
              <td class="mono">
                {#if run.archive}
                  <span class="archive-name" title="SHA-256: {run.archive.sha256}">
                    {run.archive.name}
                    <span class="dim">({formatBytes(run.archive.bytes)})</span>
                  </span>
                {:else}
                  <span class="dim">—</span>
                {/if}
              </td>
              <td class="detail-col">
                {#if run.detail}
                  <span class="detail-msg mono" class:err-msg={!isSuccess} title={run.detail}>
                    {run.detail}
                  </span>
                {/if}
                {#if run.log_path}
                  <span class="log-cell mono" title={run.log_path}>
                    {run.log_path.split("/").slice(-1)[0]}
                  </span>
                {/if}
                {#if !run.detail && !run.log_path}
                  <span class="dim">—</span>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</section>

<style>
  .section {
    margin-bottom: var(--space-8);
  }

  .section-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-4);
    margin-bottom: var(--space-4);
  }

  .head-titles h2 {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 600;
    line-height: var(--leading-tight);
    color: var(--text-primary);
  }

  .section-desc {
    margin: var(--space-1) 0 0;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .running-indicator {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: var(--text-xs);
    color: var(--primary);
    font-family: var(--font-mono);
  }

  .error-banner {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-md);
    background-color: var(--danger-soft);
    color: var(--danger);
    font-size: var(--text-sm);
    margin-bottom: var(--space-6);
  }

  .loading,
  .empty-card {
    padding: var(--space-6);
    text-align: center;
    color: var(--text-tertiary);
    font-size: var(--text-sm);
  }

  /* Targets Grid */
  .targets-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(20rem, 1fr));
    gap: var(--space-4);
  }

  .target-card {
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .target-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .target-ident {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .target-name {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-primary);
  }

  .target-meta {
    font-size: var(--text-2xs);
    color: var(--text-secondary);
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.2rem var(--space-2);
    word-break: break-all;
  }

  .meta-label {
    color: var(--text-tertiary);
  }

  .verdict-banner {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    line-height: var(--leading-normal);
  }

  .verdict-ok {
    background-color: var(--success-soft, color-mix(in srgb, var(--success) 12%, transparent));
    color: var(--success);
  }

  .verdict-err {
    background-color: var(--danger-soft);
    color: var(--danger);
  }

  .policy-control {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding-top: var(--space-2);
    border-top: 1px solid var(--card-border);
  }

  .policy-label {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .policy-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .policy-select {
    width: auto;
    padding: 0.25rem 0.5rem;
    font-size: var(--text-xs);
  }

  .custom-hours-wrap {
    display: flex;
    align-items: center;
    gap: 0.2rem;
  }

  .custom-hours-input {
    width: 4.5rem;
    padding: 0.25rem 0.4rem;
    font-size: var(--text-xs);
  }

  .hours-suffix {
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  /* Contracts List */
  .contracts-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .contract-card {
    padding: var(--space-4);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
  }

  @media (max-width: 48rem) {
    .contract-card {
      flex-direction: column;
      align-items: stretch;
    }
  }

  .contract-main {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
    flex: 1;
  }

  .contract-header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  .contract-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .cap-name {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-primary);
  }

  .contract-receipt {
    font-size: var(--text-xs);
    color: var(--text-secondary);
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: center;
  }

  .cadence-rule {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .holds-tag {
    font-size: var(--text-2xs);
    color: var(--warning-ink);
    background-color: var(--warning-soft);
  }

  .attempt-failure {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background-color: var(--danger-soft);
    color: var(--danger);
    font-size: var(--text-xs);
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }

  .attempt-failure-title {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .attempt-failure-detail {
    margin: 0;
    font-size: var(--text-2xs);
    word-break: break-all;
    white-space: pre-wrap;
    opacity: 0.9;
  }

  .attempt-failure-log {
    margin: 0;
    font-size: var(--text-2xs);
    opacity: 0.75;
  }

  .cold-copy-warning {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--warning-ink);
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background-color: var(--warning-soft);
  }

  .contract-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  .trigger-group,
  .confirm-group {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .target-select {
    width: auto;
    font-size: var(--text-xs);
    padding: 0.3rem 0.5rem;
  }

  /* Badges */
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.125rem 0.45rem;
    border-radius: var(--radius-sm);
    font-size: var(--text-2xs);
    font-weight: 600;
    line-height: 1.4;
  }

  .badge-ok {
    background-color: var(--success-soft, color-mix(in srgb, var(--success) 12%, transparent));
    color: var(--success);
  }

  .badge-warn {
    background-color: var(--warning-soft);
    color: var(--warning-ink);
  }

  .badge-err {
    background-color: var(--danger-soft);
    color: var(--danger);
  }

  .badge-info {
    background-color: var(--primary-soft);
    color: var(--primary);
  }

  .badge-dim {
    background-color: var(--card-border);
    color: var(--text-tertiary);
  }

  .btn-sm {
    padding: 0.25rem 0.55rem;
    font-size: var(--text-xs);
  }

  /* Runs Ledger Table */
  .table-wrap {
    overflow-x: auto;
  }

  .num-cell {
    white-space: nowrap;
    font-size: var(--text-xs);
  }

  .archive-name {
    font-size: var(--text-xs);
    white-space: nowrap;
  }

  .dim {
    color: var(--text-tertiary);
  }

  .detail-col {
    max-width: 22rem;
  }

  .detail-msg {
    display: block;
    font-size: var(--text-2xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 22rem;
  }

  .err-msg {
    color: var(--danger);
  }

  .log-cell {
    display: block;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .active-pulse {
    color: var(--primary);
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
  }
</style>
