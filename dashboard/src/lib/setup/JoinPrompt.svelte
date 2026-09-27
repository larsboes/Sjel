<script lang="ts">
  // On the Mac: a phone that tapped "Find my Mac" waits here to be allowed. Shown on any page of
  // the dashboard, so the owner does not have to open Devices first. The code must match the
  // phone's screen (lib/devices.ts, "Guided pairing").
  import { onMount } from "svelte";
  import Overlay from "../Overlay.svelte";
  import { request } from "../api";
  import { comparisonCode } from "../connection/transports";
  import { joinCode, joins, type JoinRequest } from "../devices";

  let waiting = $state<JoinRequest[]>([]);
  let certificateCode = $state<string | null>(null);
  let busy = $state(false);
  let note = $state<string | null>(null);
  /** Closed without an answer: not shown again in this tab. The request expires by itself. */
  let dismissed = $state<string[]>([]);
  const current = $derived(waiting.find((join) => !dismissed.includes(join.id)) ?? null);

  async function refresh() {
    try {
      waiting = await joins.pending();
    } catch {
      waiting = [];
    }
  }

  onMount(() => {
    void (async () => {
      try {
        const lan = await request<{ enabled: boolean; lan?: { fingerprint: string } }>(
          "/sjel-status/api/sjel-status/lan",
        );
        certificateCode = lan.enabled && lan.lan ? comparisonCode(lan.lan.fingerprint) : null;
      } catch {
        certificateCode = null;
      }
      await refresh();
    })();
    const timer = setInterval(() => void refresh(), 3000);
    return () => clearInterval(timer);
  });

  async function decide(join: JoinRequest, allow: boolean) {
    busy = true;
    try {
      await (allow ? joins.approve(join.id) : joins.deny(join.id));
      note = allow ? `${join.label} is added.` : `${join.label} was not allowed.`;
      setTimeout(() => (note = null), 4000);
    } catch (e) {
      note = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
      await refresh();
    }
  }
</script>

{#if current && certificateCode}
  <Overlay title="Allow this iPhone?" onClose={() => (dismissed = [...dismissed, current.id])} {busy} width="440px">
    <p><strong>{current.label}</strong> wants to join Sjel on this Mac.</p>
    <p>Allow it only if the iPhone shows this code:</p>
    <p class="code">{joinCode(certificateCode, current.key_code)}</p>
    <div class="actions">
      <button type="button" class="primary" disabled={busy} onclick={() => void decide(current, true)}>Allow</button>
      <button type="button" disabled={busy} onclick={() => void decide(current, false)}>Don't allow</button>
    </div>
  </Overlay>
{:else if note}
  <p class="note" role="status">{note}</p>
{/if}

<style>
  .code {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 1.15rem;
    letter-spacing: 0.04em;
    text-align: center;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
  }
  .primary {
    font-weight: 600;
  }
  .note {
    position: fixed;
    bottom: 1rem;
    right: 1rem;
    padding: 0.5rem 0.75rem;
    border-radius: 8px;
    background: var(--surface, #fff);
    border: 1px solid var(--border, #ddd);
    z-index: 50;
  }
</style>
