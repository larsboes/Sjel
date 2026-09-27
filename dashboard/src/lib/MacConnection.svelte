<script lang="ts">
  // How this app reaches its Sjel node. Rendered only inside the Tauri app: the web shell
  // reaches the node through its own proxy and has nothing to set.
  //
  // PRD Q119: every way to connect is listed with its pros and cons, and more than one can be
  // set. The local address is tried first and the main address after it
  // (src-tauri/src/mac_bridge.rs, ReqwestTransport).
  import { onMount } from "svelte";
  import Overlay from "./Overlay.svelte";
  import { addressKind, comparisonCode, TRANSPORTS, type TransportId } from "./connection/transports";
  import {
    browseLocalNetwork,
    bridgeErrorText,
    getConnectionSettings,
    HEALTH_PATH,
    inTauri,
    macRequest,
    setCanonicalBaseUrl,
    setLocalEndpoint,
    type ConnectionSettings,
    type FoundNode,
  } from "./mac-bridge";

  const shown = inTauri();
  const onPhone = shown && /iPhone|iPad|iPod/.test(navigator.userAgent);
  let open = $state(false);
  let settings = $state<ConnectionSettings | null>(null);
  let draft = $state("");
  let busy = $state(false);
  let found = $state<FoundNode[] | null>(null);
  let confirming = $state<FoundNode | null>(null);
  let result = $state<{ ok: boolean; text: string } | null>(null);

  const kind = $derived(addressKind(settings?.canonical_base_url ?? null));
  const configured = $derived(Boolean(settings?.canonical_base_url || settings?.local));

  function isSet(id: TransportId): boolean {
    if (id === "local") return Boolean(settings?.local);
    if (id === "tailnet" || id === "server") return kind === id;
    return false;
  }

  onMount(async () => {
    if (!shown) return;
    try {
      settings = await getConnectionSettings();
      draft = settings.canonical_base_url ?? "";
    } catch (error) {
      result = { ok: false, text: bridgeErrorText(error) };
    }
  });

  async function run(action: () => Promise<string>) {
    busy = true;
    result = null;
    try {
      result = { ok: true, text: await action() };
    } catch (error) {
      result = { ok: false, text: bridgeErrorText(error) };
    } finally {
      busy = false;
    }
  }

  const saveAddress = () =>
    run(async () => {
      settings = await setCanonicalBaseUrl(draft);
      draft = settings.canonical_base_url ?? "";
      return settings.canonical_base_url ? "Saved." : "Cleared.";
    });

  const findMac = () =>
    run(async () => {
      found = null;
      confirming = null;
      const answer = await browseLocalNetwork();
      if (answer.denied) {
        throw new Error("Local network access is off for this app. Turn it on in Settings, Privacy & Security, Local Network.");
      }
      found = answer.nodes;
      return answer.nodes.length ? "Pick your Mac." : "No Mac found on this Wi-Fi. Is Sjel's Same Wi-Fi option on at the Mac?";
    });

  const confirmMac = (node: FoundNode) =>
    run(async () => {
      settings = await setLocalEndpoint(`https://${node.host}:${node.port}`, node.fingerprint);
      found = null;
      confirming = null;
      return `Connected to ${node.name} on this Wi-Fi.`;
    });

  const forgetLocal = () =>
    run(async () => {
      settings = await setLocalEndpoint("", "");
      return "Same Wi-Fi removed.";
    });

  const test = () =>
    run(async () => {
      const answer = await macRequest(HEALTH_PATH);
      if (answer.status < 200 || answer.status >= 300) throw new Error(`HTTP ${answer.status}: ${answer.body.slice(0, 200)}`);
      return answer.stale ? "Not reached: showing this device's copy." : "Sjel answered.";
    });
</script>

{#if shown}
  <button class="link" type="button" onclick={() => (open = true)}>
    Sjel connection{configured ? "" : " (not set)"}
  </button>
  {#if open}
    <Overlay title="Sjel connection" onClose={() => (open = false)} {busy}>
      <p class="hint">
        Pick one or more. Whichever you choose, only this paired device can get in: it signs every request with a key that never leaves it.
      </p>
      <ul class="options">
        {#each TRANSPORTS as option (option.id)}
          <li class="option" class:set={isSet(option.id)}>
            <header>
              <strong>{option.title}</strong>
              {#if isSet(option.id)}<span class="badge">In use</span>{/if}
              {#if option.availability === "later"}<span class="badge muted">Coming later</span>{/if}
            </header>
            <p>{option.summary}</p>
            <div class="tradeoffs">
              <ul class="pros">{#each option.pros as pro}<li>{pro}</li>{/each}</ul>
              <ul class="cons">{#each option.cons as con}<li>{con}</li>{/each}</ul>
            </div>

            {#if option.id === "local" && onPhone}
              <div class="actions">
                <button type="button" disabled={busy} onclick={() => void findMac()}>Find my Mac</button>
                {#if settings?.local}
                  <button type="button" class="quiet" disabled={busy} onclick={() => void forgetLocal()}>Remove</button>
                {/if}
              </div>
              {#if found}
                <ul class="found">
                  {#each found as node (node.name)}
                    <li>
                      {#if confirming?.name === node.name}
                        <p>Check that your Mac shows this code under Devices, Same Wi-Fi:</p>
                        <p class="code">{comparisonCode(node.fingerprint)}</p>
                        <div class="actions">
                          <button type="button" disabled={busy} onclick={() => void confirmMac(node)}>It matches</button>
                          <button type="button" class="quiet" disabled={busy} onclick={() => (confirming = null)}>It does not</button>
                        </div>
                      {:else}
                        <button type="button" class="quiet" disabled={busy} onclick={() => (confirming = node)}>{node.name}</button>
                      {/if}
                    </li>
                  {/each}
                </ul>
              {/if}
            {/if}

            {#if option.id === "server"}
              <form
                onsubmit={(event) => {
                  event.preventDefault();
                  void saveAddress();
                }}
              >
                <label>
                  Address (a Tailscale name or your server)
                  <input
                    type="url"
                    bind:value={draft}
                    placeholder="https://<name>.ts.net or https://axon.example.com"
                    autocapitalize="off"
                    autocomplete="off"
                    spellcheck="false"
                  />
                </label>
                <div class="actions">
                  <button type="submit" disabled={busy}>Save</button>
                </div>
              </form>
            {/if}
          </li>
        {/each}
      </ul>
      <div class="actions">
        <button type="button" disabled={busy || !configured} onclick={() => void test()}>Test connection</button>
      </div>
      {#if result}
        <p class="result" class:bad={!result.ok}>{result.text}</p>
      {/if}
    </Overlay>
  {/if}
{/if}

<style>
  .link {
    background: none;
    border: 0;
    padding: 0;
    color: inherit;
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }
  .options,
  .found,
  .pros,
  .cons {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .options {
    display: grid;
    gap: 0.75rem;
  }
  .option {
    border: 1px solid var(--border, #ddd);
    border-radius: 8px;
    padding: 0.6rem 0.75rem;
    display: grid;
    gap: 0.4rem;
  }
  .option.set {
    border-color: var(--accent, #2b6cb0);
  }
  .option header {
    display: flex;
    gap: 0.5rem;
    align-items: baseline;
  }
  .option p {
    margin: 0;
  }
  .badge {
    font-size: var(--text-sm);
    padding: 0 0.4rem;
    border-radius: 4px;
    background: var(--accent, #2b6cb0);
    color: white;
  }
  .badge.muted {
    background: var(--muted, #888);
  }
  .tradeoffs {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.5rem;
    font-size: var(--text-sm);
  }
  .pros li::before {
    content: "+ ";
  }
  .cons li::before {
    content: "− ";
  }
  .code {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 1.2rem;
    letter-spacing: 0.05em;
  }
  label {
    display: grid;
    gap: 0.35rem;
  }
  input {
    font: inherit;
    padding: 0.4rem 0.5rem;
  }
  .hint {
    opacity: 0.75;
    font-size: var(--text-sm);
  }
  .actions {
    display: flex;
    gap: 0.5rem;
  }
  .result {
    word-break: break-word;
  }
  .bad {
    color: var(--danger, #b00020);
  }
</style>
