<script lang="ts">
  import Overlay from './Overlay.svelte';
  import { request } from './api';
  import { comparisonCode } from './connection/transports';
  import {
    canClaimOnThisDevice,
    devices,
    getDeviceIdentity,
    resetDeviceIdentity,
    type DeviceRecord,
    type PairingChallenge,
  } from './devices';

  let open = $state(false);
  let busy = $state(false);
  let records = $state<DeviceRecord[]>([]);
  let challenge = $state<PairingChallenge | null>(null);
  let payloadDraft = $state('');
  let deviceLabel = $state('iPhone');
  let result = $state<{ ok: boolean; text: string } | null>(null);
  const canClaim = canClaimOnThisDevice();
  /** The node's Same Wi-Fi listener, from the shell (PRD Q119); null when it is off. */
  let lan = $state<{ port: number; host: string; fingerprint: string } | null>(null);

  function failure(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
  }

  async function refresh(): Promise<void> {
    busy = true;
    result = null;
    try {
      records = (await devices.list()).devices;
    } catch (error) {
      result = { ok: false, text: failure(error) };
    } finally {
      busy = false;
    }
  }

  async function openPanel(): Promise<void> {
    open = true;
    await refresh();
    try {
      const answer = await request<{ enabled: boolean; lan?: { port: number; host: string; fingerprint: string } }>(
        '/sjel-status/api/sjel-status/lan',
      );
      lan = answer.enabled && answer.lan ? answer.lan : null;
    } catch {
      lan = null;
    }
  }

  async function createChallenge(): Promise<void> {
    busy = true;
    result = null;
    try {
      challenge = await devices.createChallenge();
      result = { ok: true, text: 'Challenge ready. It expires in ten minutes.' };
    } catch (error) {
      result = { ok: false, text: failure(error) };
    } finally {
      busy = false;
    }
  }

  function parsePayload(value: string): { challenge_id: string; code: string } {
    let parsed: unknown;
    try {
      parsed = JSON.parse(value);
    } catch {
      throw new Error('The pairing payload is not valid JSON.');
    }
    if (
      !parsed ||
      typeof parsed !== 'object' ||
      (parsed as Record<string, unknown>).protocol_version !== 'axon-pairing/v1' ||
      typeof (parsed as Record<string, unknown>).challenge_id !== 'string' ||
      typeof (parsed as Record<string, unknown>).code !== 'string'
    ) {
      throw new Error('The pairing payload is not an Sjel pairing challenge.');
    }
    const valueObject = parsed as Record<string, string>;
    return { challenge_id: valueObject.challenge_id, code: valueObject.code };
  }

  async function rotateIdentity(): Promise<void> {
    if (!window.confirm('Create a new iPhone identity? The current Keychain identity will no longer be used.')) return;
    busy = true;
    result = null;
    try {
      await resetDeviceIdentity();
      payloadDraft = '';
      result = { ok: true, text: 'New Keychain identity ready. Create a fresh code on the Mac, then register this iPhone.' };
    } catch (error) {
      result = { ok: false, text: failure(error) };
    } finally {
      busy = false;
    }
  }

  async function claimThisDevice(): Promise<void> {
    busy = true;
    result = null;
    try {
      const payload = parsePayload(payloadDraft);
      const identity = await getDeviceIdentity();
      const registered = await devices.claim({
        ...payload,
        label: deviceLabel.trim() || 'iPhone',
        platform: identity.platform,
        algorithm: identity.algorithm,
        public_key: identity.public_key,
      });
      records = [registered, ...records.filter((record) => record.id !== registered.id)];
      result = { ok: true, text: `${registered.label} is now registered.` };
      payloadDraft = '';
    } catch (error) {
      result = { ok: false, text: failure(error) };
    } finally {
      busy = false;
    }
  }

  async function copy(value: string, label: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(value);
      result = { ok: true, text: `${label} copied.` };
    } catch (error) {
      result = { ok: false, text: `Could not copy ${label.toLowerCase()}: ${failure(error)}` };
    }
  }

  async function verifyConnection(): Promise<void> {
    busy = true;
    result = null;
    try {
      const device = await devices.me();
      records = records.map((record) => (record.id === device.id ? device : record));
      result = { ok: true, text: `Signed connection verified. Last seen ${date(device.last_seen_at)}.` };
    } catch (error) {
      result = { ok: false, text: failure(error) };
    } finally {
      busy = false;
    }
  }

  async function revoke(record: DeviceRecord): Promise<void> {
    if (!window.confirm(`Revoke ${record.label}? The device will no longer be trusted.`)) return;
    busy = true;
    result = null;
    try {
      const updated = await devices.revoke(record.id);
      records = records.map((item) => (item.id === updated.id ? updated : item));
      result = { ok: true, text: `${record.label} revoked.` };
    } catch (error) {
      result = { ok: false, text: failure(error) };
    } finally {
      busy = false;
    }
  }

  function date(value: number | null): string {
    return value === null ? 'Never' : new Date(value * 1000).toLocaleString();
  }
</script>

<button class="link" type="button" onclick={() => void openPanel()}>Devices</button>

{#if open}
  <Overlay title="Connected devices" eyebrow="Sjel setup" onClose={() => (open = false)} {busy} width="620px">
    <p class="intro">
      Pairing registers a device's public key with this canonical node. The private key must be
      created and kept by the device's platform key store; it is never entered here.
    </p>

    <section class="pairing">
      <div class="section-heading">
        <div>
          <h3>Pair a device</h3>
          {#if lan && !canClaim}
            <p class="hint">
              Same Wi-Fi is on ({lan.host}.local, port {lan.port}). When a phone finds this Mac, it shows a code. It must be
              <strong class="code">{comparisonCode(lan.fingerprint)}</strong>.
            </p>
          {/if}
          <p class="hint">Generate a one-time challenge, then use its code or payload in the device setup flow.</p>
        </div>
        <button type="button" disabled={busy} onclick={() => void createChallenge()}>New code</button>
      </div>
      {#if challenge}
        {@const currentChallenge = challenge}
        <div class="code-block">
          <span class="label">One-time code</span>
          <code>{currentChallenge.code}</code>
          <span class="hint">Expires {date(currentChallenge.expires_at)}</span>
          <button type="button" class="quiet" disabled={busy} onclick={() => void copy(currentChallenge.code, 'Code')}>Copy code</button>
        </div>
        <label>
          Pairing payload
          <textarea readonly rows="4" value={currentChallenge.qr_payload}></textarea>
        </label>
        <button type="button" class="quiet" disabled={busy} onclick={() => void copy(currentChallenge.qr_payload, 'Pairing payload')}>Copy payload</button>
      {/if}
    </section>

    {#if canClaim}
      <section class="claim">
        <div class="section-heading">
          <div>
            <h3>Register this iPhone</h3>
            <p class="hint">Paste the pairing payload from the canonical node. The private key stays in iOS Keychain.</p>
            {#if records.some((record) => record.status === 'revoked')}
              <button type="button" class="quiet" disabled={busy} onclick={() => void rotateIdentity()}>Create new identity</button>
            {/if}
          </div>
        </div>
        <label>
          Device label
          <input bind:value={deviceLabel} maxlength="80" autocomplete="off" />
        </label>
        <label>
          Pairing payload
          <textarea bind:value={payloadDraft} rows="4" placeholder="Paste the payload from the Mac here"></textarea>
        </label>
        <button type="button" disabled={busy || !payloadDraft.trim()} onclick={() => void claimThisDevice()}>Register this iPhone</button>
      </section>
    {/if}

    <section>
      <div class="section-heading">
        <div>
          <h3>Registered devices</h3>
          <p class="hint">Revocation takes effect at the node. Signed requests keep authenticated device routes replay-safe.</p>
        </div>
        <div class="section-actions">
          {#if canClaim}
            <button type="button" class="quiet" disabled={busy} onclick={() => void verifyConnection()}>Verify signed connection</button>
          {/if}
          <button type="button" class="quiet" disabled={busy} onclick={() => void refresh()}>Refresh</button>
        </div>
      </div>
      {#if records.length === 0}
        <p class="empty">No devices are registered.</p>
      {:else}
        <ul class="devices">
          {#each records as record (record.id)}
            <li class:revoked={record.status === 'revoked'}>
              <div>
                <strong>{record.label}</strong>
                <span class="meta">{record.platform} · {record.status} · last seen {date(record.last_seen_at)}</span>
                <code class="fingerprint">{record.fingerprint}</code>
              </div>
              {#if record.status === 'active'}
                <button type="button" class="danger" disabled={busy} onclick={() => void revoke(record)}>Revoke</button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    {#if result}
      <p class:bad={!result.ok} class="result">{result.text}</p>
    {/if}
  </Overlay>
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

  .intro,
  .hint,
  .empty,
  .meta {
    color: var(--text-secondary);
  }

  .intro {
    margin: 0 0 1.25rem;
    line-height: 1.5;
  }

  section + section {
    margin-top: 1.5rem;
    padding-top: 1.5rem;
    border-top: 1px solid var(--card-border);
  }

  .section-heading {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
    margin-bottom: 0.75rem;
  }

  .section-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 0.4rem;
  }

  h3 {
    margin: 0;
    font-size: 1rem;
  }

  .hint {
    margin: 0.25rem 0 0;
    font-size: var(--text-sm);
    line-height: 1.4;
  }

  button {
    border: 1px solid var(--card-border);
    border-radius: 6px;
    padding: 0.4rem 0.65rem;
    background: var(--surface);
    color: inherit;
    font: inherit;
    cursor: pointer;
    white-space: nowrap;
  }

  button:disabled {
    cursor: wait;
    opacity: 0.55;
  }

  .quiet {
    font-size: var(--text-sm);
  }

  .code-block {
    display: grid;
    gap: 0.35rem;
    margin-bottom: 1rem;
    padding: 1rem;
    border: 1px solid var(--card-border);
    border-radius: 8px;
  }

  .label {
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .code-block code {
    font-size: 1.8rem;
    letter-spacing: 0.18em;
  }

  label {
    display: grid;
    gap: 0.35rem;
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  input,
  textarea {
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    padding: 0.5rem;
    border: 1px solid var(--card-border);
    border-radius: 6px;
    background: var(--surface);
    color: inherit;
    font: inherit;
  }

  textarea {
    font: 0.78rem/1.4 var(--font-mono, monospace);
  }

  .devices {
    display: grid;
    gap: 0.6rem;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .devices li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.75rem;
    border: 1px solid var(--card-border);
    border-radius: 8px;
  }

  .devices li.revoked {
    opacity: 0.6;
  }

  .devices li > div {
    display: grid;
    gap: 0.2rem;
    min-width: 0;
  }

  .meta,
  .fingerprint {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-xs);
  }

  .fingerprint {
    color: var(--text-secondary);
  }

  .danger {
    color: var(--danger, #b00020);
  }

  .result {
    margin: 1rem 0 0;
    word-break: break-word;
  }

  .bad {
    color: var(--danger, #b00020);
  }

  @media (max-width: 560px) {
    .section-heading,
    .devices li {
      align-items: stretch;
      flex-direction: column;
    }
  }
  .code {
    font-family: var(--font-mono, ui-monospace, monospace);
    letter-spacing: 0.05em;
  }
</style>
