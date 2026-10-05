<script lang="ts">
  import { tip } from "$lib/tip";
  // The agent's reach on this machine (ISA F10): one mode per capability, the writes that
  // wait for the owner, and the latest calls. The gate in each capability reads the same
  // policy on its next request, so a change here needs no restart.
  import { onDestroy, onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import { axonStatus, type AgentMode, type AgentView } from "$lib/api";

  let view = $state<AgentView | null>(null);
  let err = $state<string | null>(null);
  let busy = $state<string | null>(null);
  let timer: ReturnType<typeof setInterval> | undefined;

  const LABELS: Record<AgentMode, string> = {
    off: "Off",
    "read-only": "Read only",
    ask: "Ask",
    auto: "Auto",
  };

  async function load() {
    try {
      view = await axonStatus.agent();
      err = null;
    } catch (e) {
      err = e instanceof Error ? e.message : String(e);
    }
  }

  async function setMode(capability: string, mode: AgentMode) {
    busy = capability;
    try {
      await axonStatus.setAgentMode(capability, mode);
      await load();
    } catch (e) {
      err = e instanceof Error ? e.message : String(e);
    } finally {
      busy = null;
    }
  }

  async function decide(id: string, allow: boolean) {
    busy = id;
    try {
      await axonStatus.decideAgentWrite(id, allow);
      await load();
    } catch (e) {
      err = e instanceof Error ? e.message : String(e);
    } finally {
      busy = null;
    }
  }

  function when(at: number): string {
    return new Date(at * 1000).toLocaleString();
  }

  onMount(() => {
    load();
    // Pending writes are what a person waits on, so the list refreshes while the page is open.
    timer = setInterval(load, 5000);
  });
  onDestroy(() => clearInterval(timer));
</script>

<section class="agent">
  <h2 class="section-head">
    <Icon name="sparkles" size={14} />
    Agent
    {#if view}
      <span class="state">{view.enrolled ? "enrolled" : "not enrolled"}</span>
    {/if}
  </h2>

  {#if err}
    <p class="err"><Icon name="alert" size={14} /> {err}</p>
  {/if}

  {#if !view}
    <p class="hint"><Icon name="loader" size={14} /> reading the agent policy…</p>
  {:else}
    {#if view.pending.length > 0}
      <h3 class="col-head">Waiting for you</h3>
      <ul class="list">
        {#each view.pending as approval (approval.id)}
          <li class="pending">
            <span class="what">
              <span class="mono">{approval.method} {approval.path}</span>
              <span class="dim">{approval.capability} · {when(approval.created_at)}</span>
              {#if approval.preview}<span class="preview mono">{approval.preview}</span>{/if}
            </span>
            <span class="actions">
              <button disabled={busy === approval.id} onclick={() => decide(approval.id, true)}>Allow</button>
              <button disabled={busy === approval.id} onclick={() => decide(approval.id, false)}>Deny</button>
            </span>
          </li>
        {/each}
      </ul>
    {/if}

    <h3 class="col-head">What an agent may do</h3>
    {#if view.capabilities.length === 0}
      <p class="hint">No capability has started an agent gate on this machine yet.</p>
    {:else}
      <ul class="list">
        {#each view.capabilities as cap (cap.capability)}
          <li class="cap">
            <span class="what">
              <span class="mono">{cap.capability}</span>
              {#if cap.confirm.length > 0}
                <span class="dim" use:tip={cap.confirm.join("\n")}>
                  {cap.confirm.length} action{cap.confirm.length === 1 ? "" : "s"} always ask
                </span>
              {/if}
            </span>
            <select
              aria-label="Agent mode for {cap.capability}"
              value={cap.mode}
              disabled={busy === cap.capability}
              onchange={(e) => setMode(cap.capability, e.currentTarget.value as AgentMode)}
            >
              {#each view.modes as mode (mode)}
                <option value={mode}>{LABELS[mode]}</option>
              {/each}
            </select>
          </li>
        {/each}
      </ul>
      <p class="hint">
        Auto lets an agent change things; an action that leaves Sjel or cannot be undone still asks.
        Ask waits for you here or in the menu bar.
      </p>
    {/if}

    <h3 class="col-head">Latest calls</h3>
    {#if view.calls.length === 0}
      <p class="hint">No agent call yet.</p>
    {:else}
      <ul class="list calls">
        {#each view.calls.slice(0, 20) as call, i (i)}
          <li class="call mono">
            <span class="dim">{when(call.at)}</span>
            <span>{call.capability}</span>
            <span>{call.method} {call.path}</span>
            <span>{call.status}</span>
            <span class="dim">{call.decision}</span>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</section>

<style>
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
  }

  .col-head {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-secondary);
    margin: 0.75rem 0 0.4rem;
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .cap,
  .pending {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    padding: 0.4rem 0.6rem;
    border: 1px solid var(--card-border);
    border-radius: 6px;
    font-size: var(--text-sm);
  }

  .what {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    min-width: 0;
  }

  .preview {
    font-size: var(--text-2xs);
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .actions {
    display: flex;
    gap: 0.35rem;
  }

  .call {
    display: grid;
    grid-template-columns: 11rem 7rem 1fr 3rem 6rem;
    gap: 0.5rem;
    font-size: var(--text-2xs);
  }

  .dim {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .hint {
    font-size: var(--text-xs);
    color: var(--text-tertiary);
    margin: 0.4rem 0;
  }

  .err {
    font-size: var(--text-xs);
    color: var(--danger);
  }
</style>
