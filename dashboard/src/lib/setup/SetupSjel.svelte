<script lang="ts">
  // The phone's first-launch setup: find the Mac, pin it and register this iPhone in one flow.
  // Scanning the Mac's QR code is the main path; "Find my Mac" asks to join and waits for the
  // owner to allow it on the Mac. Both are described in lib/devices.ts, "Guided pairing".
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import Overlay from "../Overlay.svelte";
  import { ApiError } from "../api";
  import { comparisonCode } from "../connection/transports";
  import {
    canClaimOnThisDevice,
    devices,
    getDeviceIdentity,
    joinCode,
    joins,
    keyCode,
    parsePairingQr,
  } from "../devices";
  import {
    bridgeErrorText,
    browseLocalNetwork,
    getConnectionSettings,
    setLocalEndpoint,
    type FoundNode,
  } from "../mac-bridge";
  import { setupSheet } from "./setup-state.svelte";

  type Step = "intro" | "working" | "pick" | "waiting" | "done";

  let step = $state<Step>("intro");
  let working = $state("");
  let error = $state<string | null>(null);
  let label = $state("iPhone");
  let nodes = $state<FoundNode[]>([]);
  let code = $state("");
  let macName = $state("");
  let poll: ReturnType<typeof setInterval> | null = null;

  onMount(() => {
    if (!canClaimOnThisDevice()) return;
    void (async () => {
      // Open by itself only when this iPhone has no way in yet: no address, or a node that does
      // not know its key. Offline or any other failure is not a reason to interrupt.
      try {
        const settings = await getConnectionSettings();
        if (!settings.canonical_base_url && !settings.local) {
          setupSheet.open = true;
          return;
        }
        await devices.me();
      } catch (e) {
        if (e instanceof ApiError && e.status === 401) setupSheet.open = true;
      }
    })();
    return stopPolling;
  });

  function stopPolling() {
    if (poll) clearInterval(poll);
    poll = null;
  }

  function fail(message: string) {
    stopPolling();
    error = message;
    step = "intro";
  }

  function hostName(host: string): string {
    return host.replace(/\.local$/, "");
  }

  async function register(claim: { challenge_id: string; code: string }) {
    const identity = await getDeviceIdentity();
    try {
      await devices.claim({
        ...claim,
        label: label.trim() || "iPhone",
        platform: identity.platform,
        algorithm: identity.algorithm,
        public_key: identity.public_key,
      });
    } catch (e) {
      // Scanning twice is not a failure: the key is already there.
      if (!(e instanceof ApiError && e.status === 409 && /already registered/.test(e.message))) throw e;
    }
  }

  async function scan() {
    error = null;
    step = "working";
    working = "Point the camera at the code on your Mac.";
    try {
      const result = await invoke<{ content: string }>("plugin:barcode-scanner|scan", {
        formats: ["QR_CODE"],
        windowed: false,
      });
      const qr = parsePairingQr(result.content);
      working = `Connecting to ${qr.host}…`;
      await setLocalEndpoint(`https://${qr.host}.local:${qr.port}`, qr.fingerprint);
      await register({ challenge_id: qr.challenge_id, code: qr.code });
      macName = qr.host;
      step = "done";
    } catch (e) {
      fail(e instanceof Error ? e.message : bridgeErrorText(e));
    }
  }

  async function find() {
    error = null;
    step = "working";
    working = "Looking for your Mac on this Wi-Fi…";
    try {
      const answer = await browseLocalNetwork();
      if (answer.denied) {
        return fail("Sjel may not look on this Wi-Fi. Turn on Local Network for Sjel in Settings, Privacy & Security.");
      }
      if (answer.nodes.length === 0) {
        return fail("No Mac found on this Wi-Fi. Is the Mac on the same network, with Sjel running?");
      }
      if (answer.nodes.length === 1) return void (await ask(answer.nodes[0]));
      nodes = answer.nodes;
      step = "pick";
    } catch (e) {
      fail(bridgeErrorText(e));
    }
  }

  async function ask(node: FoundNode) {
    error = null;
    step = "working";
    working = `Asking ${hostName(node.host)} to add this iPhone…`;
    try {
      await setLocalEndpoint(`https://${node.host}:${node.port}`, node.fingerprint);
      const identity = await getDeviceIdentity();
      const join = await joins.request({
        label: label.trim() || "iPhone",
        platform: identity.platform,
        algorithm: identity.algorithm,
        public_key: identity.public_key,
      });
      macName = hostName(node.host);
      code = joinCode(comparisonCode(node.fingerprint), await keyCode(identity.public_key));
      step = "waiting";
      poll = setInterval(() => void check(join.id), 2000);
    } catch (e) {
      await setLocalEndpoint("", "").catch(() => {});
      fail(e instanceof Error ? e.message : bridgeErrorText(e));
    }
  }

  async function check(id: string) {
    try {
      const join = await joins.status(id);
      if (join.status === "approved") {
        stopPolling();
        step = "done";
      } else if (join.status === "denied" || join.status === "expired") {
        await setLocalEndpoint("", "").catch(() => {});
        fail(join.status === "denied" ? "The Mac did not allow this iPhone." : "The Mac did not answer in time. Try again.");
      }
    } catch {
      // One missed poll is not a failure; the next one tries again.
    }
  }

  function close() {
    stopPolling();
    setupSheet.open = false;
    if (step === "done") location.reload();
  }
</script>

{#if setupSheet.open}
  <Overlay title="Set up Sjel" onClose={close} width="420px">
    {#if step === "intro"}
      <p>Connect this iPhone to Sjel on your Mac.</p>
      <ol class="steps">
        <li>On your Mac, open Sjel and choose <strong>Devices</strong>, then <strong>Add iPhone</strong>.</li>
        <li>Scan the code it shows.</li>
      </ol>
      <label>
        Name for this iPhone
        <input bind:value={label} maxlength="80" autocomplete="off" />
      </label>
      <div class="actions">
        <button type="button" class="primary" onclick={() => void scan()}>Scan the code</button>
        <button type="button" class="quiet" onclick={() => void find()}>No code? Find my Mac on this Wi-Fi</button>
      </div>
      {#if error}<p class="error">{error}</p>{/if}
    {:else if step === "working"}
      <p class="status">{working}</p>
    {:else if step === "pick"}
      <p>Which Mac?</p>
      <ul class="nodes">
        {#each nodes as node (node.name)}
          <li><button type="button" class="quiet" onclick={() => void ask(node)}>{hostName(node.host)}</button></li>
        {/each}
      </ul>
    {:else if step === "waiting"}
      <p>On <strong>{macName}</strong>, Sjel asks to allow this iPhone.</p>
      <p>Allow it only if the Mac shows this code:</p>
      <p class="code">{code}</p>
      <p class="status">Waiting for the Mac…</p>
      <button type="button" class="quiet" onclick={() => fail("Cancelled.")}>Cancel</button>
    {:else}
      <p class="ok">✓ Connected to {macName}</p>
      <p class="ok">✓ This iPhone is added</p>
      <div class="actions">
        <button type="button" class="primary" onclick={close}>Done</button>
      </div>
    {/if}
  </Overlay>
{/if}

<style>
  .steps {
    padding-left: 1.2rem;
    display: grid;
    gap: 0.35rem;
  }
  label {
    display: grid;
    gap: 0.35rem;
    margin: 0.75rem 0;
  }
  input {
    font: inherit;
    padding: 0.4rem 0.5rem;
  }
  .actions {
    display: grid;
    gap: 0.5rem;
  }
  .primary {
    padding: 0.6rem;
    font-weight: 600;
  }
  .quiet {
    background: none;
    border: 0;
    text-decoration: underline;
    cursor: pointer;
    color: inherit;
    font: inherit;
  }
  .nodes {
    list-style: none;
    padding: 0;
  }
  .code {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 1.15rem;
    letter-spacing: 0.04em;
    text-align: center;
  }
  .status {
    opacity: 0.75;
  }
  .ok {
    font-size: 1.1rem;
    margin: 0.2rem 0;
  }
  .error {
    color: var(--danger, #b00020);
  }
</style>
