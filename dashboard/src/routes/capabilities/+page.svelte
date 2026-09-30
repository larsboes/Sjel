<script lang="ts">
  import { link } from "$lib/nav";
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import { axonStatus, hasPanel, type CapabilityView, type BackupStatus } from "$lib/api";
  import { capabilities } from "$lib/capabilities.svelte";

  let busy = $state<string | null>(null);
  let error = $state<string | null>(null);

  // Keyed by capability. Only the few that declare a backup contract appear, so most
  // cards look up nothing and render no backup row at all.
  let backups = $state<Record<string, BackupStatus>>({});
  // Which capability is mid-confirmation, when its contract says a run holds it down.
  let confirming = $state<string | null>(null);

  // Polled rather than pushed, on the same 5s beat as the capability list. That poll is
  // also what makes a slow run survive a page refresh: the run state lives in
  // sjel-status, so a reload re-reads it instead of losing it.
  async function loadBackups(): Promise<void> {
    try {
      const r = await axonStatus.backups();
      backups = Object.fromEntries(r.backups.map((b) => [b.capability, b]));
    } catch {
      // sjel-status being unreachable is already reported by the capabilities poll;
      // a second copy of the same message helps nobody.
    }
  }

  onMount(() => {
    const unsubscribe = capabilities.subscribe(5_000);
    void loadBackups();
    const timer = setInterval(() => void loadBackups(), 5_000);
    return () => {
      unsubscribe?.();
      clearInterval(timer);
    };
  });

  async function requestBackup(name: string): Promise<void> {
    confirming = null;
    error = null;
    try {
      await axonStatus.backup(name);
    } catch (e) {
      error = `backup ${name}: ${e instanceof Error ? e.message : String(e)}`;
    }
    // Refresh either way: a rejected duplicate still means a run is in flight, and the
    // surface should show that rather than only the error.
    await loadBackups();
  }

  /** Coarse on purpose — "6 days ago" is the decision, the exact minute is not. */
  function age(seconds: number | null): string {
    if (seconds === null) return "never";
    const days = Math.floor(seconds / 86_400);
    if (days >= 1) return `${days}d ago`;
    const hours = Math.floor(seconds / 3_600);
    if (hours >= 1) return `${hours}h ago`;
    return `${Math.max(1, Math.floor(seconds / 60))}m ago`;
  }

  /** What the badge says, spelled out rather than left as a bare state word. */
  function backupLabel(b: BackupStatus): string {
    if (b.state === "never") return "never backed up";
    if (b.state === "unknown") return `backed up ${age(b.age_seconds)} · no cadence declared`;
    return `backed up ${age(b.age_seconds)}`;
  }

  async function act(name: string, action: "start" | "stop"): Promise<void> {
    busy = name;
    error = null;
    try {
      await axonStatus[action](name);
      await capabilities.refresh();
    } catch (e) {
      error = `${action} ${name}: ${e instanceof Error ? e.message : String(e)}`;
    } finally {
      busy = null;
    }
  }

  // Three states, not two. A container with no declared health surface is unknown from
  // here, and painting that as "stopped" would be a lie with a colour attached.
  function state_(c: CapabilityView): { dot: string; label: string } {
    if (c.up === null) return { dot: "unknown", label: "unknown" };
    return c.up ? { dot: "up", label: "running" } : { dot: "down", label: "stopped" };
  }
</script>

<PageHeader
  badge="Machine"
  title="Capabilities"
  desc="What this machine has enabled, what is running, and what starts on demand."
/>

{#if capabilities.offline}
  <p class="offline">
    <Icon name="wifi-off" />
    sjel-status is unavailable. Start it with:
    <code>tools/service-runner.sh start sjel-status</code>
  </p>
{:else}
  {#if error}
    <p class="error">{error}</p>
  {/if}

  <ul>
    {#each capabilities.items as c (c.name)}
      {@const s = state_(c)}
      {@const b = backups[c.name]}
      <li class="card">
        <div class="left">
          <span class="dot {s.dot}"></span>
          <div class="ident">
            <p class="name">
              {c.name}
              {#if c.scope === "spine"}<span class="tag mono">spine</span>{/if}
              {#if c.autostart === "true"}<span class="tag mono">autostart</span>{/if}
            </p>
            <p class="sub mono">
              {c.kind}{c.port ? ` · :${c.port}` : ""} · {s.label}
            </p>
            {#if confirming === c.name}
              <!-- The impact, before the confirmation and derived from the contract:
                   `holds_service` is true because the manifest declares backup_sqlite,
                   not because this page knows anything about which service it is. -->
              <p class="impact">
                A backup takes a cold copy, so {c.name} stops for the duration and comes
                back on its own.
              </p>
            {/if}
            {#if b?.run?.state === "failed"}
              <!-- Terminal and readable. A locked vault is the normal way this fails, and
                   the operator needs backup.sh's own words to know it was the vault. -->
              <p class="failed">backup failed{b.run.detail ? `: ${b.run.detail}` : ""}</p>
            {:else if b?.attempt && b.attempt.exit_code !== 0 && b.attempt.exit_code !== null}
              {@const attemptAgeSeconds = b.attempt.started_epoch ? Math.max(0, Math.floor(Date.now() / 1000) - b.attempt.started_epoch) : Math.max(0, Math.floor((Date.now() - new Date(b.attempt.started_at).getTime()) / 1000))}
              <p class="failed">
                the last attempt FAILED {age(attemptAgeSeconds)} (exit {b.attempt.exit_code}){b.attempt.detail ? `: ${b.attempt.detail.split("\n")[0]}` : ""} · <a href={link("/backup")}>Backup ledger →</a>
              </p>
            {/if}
          </div>
        </div>

        <div class="actions">
          {#if b}
            <span class="backup {b.state}" title={b.contents ? `contents: ${b.contents}` : undefined}>
              {backupLabel(b)}
            </span>
            {#if b.run?.state === "running"}
              <span class="btn"><Icon name="loader" size={14} /> Backing up</span>
            {:else if confirming === c.name}
              <!-- Inline, not a confirm() dialog: a modal blocks the page, and the thing
                   worth reading is the impact sentence, which a dialog renders worst. -->
              <button class="btn danger" onclick={() => requestBackup(c.name)}>
                Stop {c.name} and back up
              </button>
              <button class="btn" onclick={() => (confirming = null)}>Cancel</button>
            {:else}
              <button
                class="btn"
                onclick={() => (b.holds_service ? (confirming = c.name) : requestBackup(c.name))}
              >
                <Icon name="database" size={12} /> Back up
              </button>
            {/if}
          {/if}
          {#if hasPanel(c) && c.up}
            <a class="btn" href={link("/projects")}>Project <Icon name="arrow-right" size={13} /></a>
          {/if}
          {#if c.up !== null}
            {#if busy === c.name}
              <span class="btn"><Icon name="loader" size={14} /></span>
            {:else if c.up}
              <button class="btn" onclick={() => act(c.name, "stop")}>
                <Icon name="square" size={12} /> Stop
              </button>
            {:else}
              <button class="btn" onclick={() => act(c.name, "start")}>
                <Icon name="play" size={12} /> Start
              </button>
            {/if}
          {/if}
        </div>
      </li>
    {/each}
  </ul>

  <p class="note">
    Only services marked <code>autostart</code> run without being requested. Everything else
    starts when you open it. See <a href={link("/backup")}>Backup management</a> for target policies, intervals, and attempt history.
  </p>
{/if}

<style>
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  li {
    padding: 0.75rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
  }

  .left {
    display: flex;
    align-items: center;
    gap: 0.65rem;
    min-width: 0;
  }

  .dot {
    height: 0.5rem;
    width: 0.5rem;
    border-radius: 999px;
    flex-shrink: 0;
    background-color: var(--text-tertiary);
  }

  .dot.up {
    background-color: var(--success);
  }

  .dot.down {
    background-color: var(--warning);
  }

  .ident {
    min-width: 0;
  }

  .name {
    margin: 0;
    font-size: 0.875rem;
    font-weight: 500;
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .sub {
    margin: 0.1rem 0 0;
    font-size: 0.625rem;
    color: var(--text-tertiary);
  }

  .impact,
  .failed {
    margin: 0.25rem 0 0;
    font-size: 0.625rem;
    max-width: 42ch;
  }

  .impact {
    color: var(--text-secondary);
  }

  .failed {
    color: var(--warning-ink);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    flex-shrink: 0;
  }

  /* Four states share one badge. `ok` and `unknown` stay tertiary — a backup that is
     fine is not news, and a capability that never declared a cadence must not be
     coloured as though Sjel had an opinion about it. */
  .backup {
    font-size: 0.625rem;
    color: var(--text-tertiary);
    white-space: nowrap;
  }

  .backup.due {
    color: var(--text-secondary);
  }

  .backup.overdue,
  .backup.never {
    color: var(--warning-ink);
  }

  .btn.danger {
    color: var(--warning-ink);
    border-color: var(--warning);
  }

  .offline,
  .error {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem;
    border-radius: var(--radius-md);
    background-color: var(--warning-soft);
    color: var(--warning-ink);
    font-size: var(--text-sm);
  }

  .note {
    margin-top: 1rem;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }
</style>
