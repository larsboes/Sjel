/**
 * The app's way to the canonical Sjel node. Inside the Tauri bundle a relative `fetch` resolves
 * against the app's own origin and reaches nothing, so `request()` in `./api.ts`
 * hands node paths to the native `mac_request` command instead
 * (`src-tauri/src/mac_bridge.rs`, which states why the call leaves from Rust).
 *
 * The node address is not in this file or in any other: it is a deployment fact and
 * the repository is public. The operator sets it in the app, and the app keeps it
 * in its own data directory.
 */
import { invoke } from '@tauri-apps/api/core';

/** Mirrors `NOT_CONFIGURED` in `src-tauri/src/mac_bridge.rs`. */
export const NOT_CONFIGURED = 'axon-node: not configured';

/** The path the settings panel calls to test the address. It is on the allow-list. */
export const HEALTH_PATH = '/axon-status/api/axon-status/health';

export interface MacResponse {
  status: number;
  content_type: string | null;
  body: string;
  /** True when the Mac was not reached and this is the device's copy (`src-tauri/src/sync.rs`). */
  stale?: boolean;
  /** For a stale answer: when the copy was fetched, unix milliseconds. */
  fetched_at?: number | null;
}

/** Mirrors `STALE_BYTES_STATUS` in `src-tauri/src/sync.rs`: a byte answer from the device's copy. */
export const STALE_BYTES_STATUS = 203;

/** Mirrors `QUEUED_STATUS` in `src-tauri/src/sync.rs`: an item edit went into the outbox. */
export const QUEUED_STATUS = 202;

export interface ConnectionSettings {
  protocol_version: 'axon-node/v1';
  node_id: string;
  canonical_base_url: string | null;
  /** The node on the local network, tried first (`LocalEndpoint` in src-tauri/src/mac_bridge.rs). */
  local?: { base_url: string; pin_sha256: string } | null;
}

/** An Sjel node found with Bonjour (plugins/local-network). Unverified until compared. */
export interface FoundNode {
  name: string;
  host: string;
  port: number;
  fingerprint: string;
}

export function inTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/** The headers the Rust side forwards. Anything else is not sent across. */
function headerPairs(headers: HeadersInit | undefined): [string, string][] {
  if (!headers) return [];
  if (headers instanceof Headers) return [...headers.entries()];
  if (Array.isArray(headers)) return headers.map(([k, v]) => [k, v] as [string, string]);
  return Object.entries(headers);
}

/**
 * Sends one request to the canonical node. Rejects with the native error string when the
 * request never got an answer (no address set, refused path, network down).
 */
export async function macRequest(path: string, init?: RequestInit): Promise<MacResponse> {
  const body = init?.body;
  if (body != null && typeof body !== 'string') {
    throw new Error('axon-node: only a text body can be sent to the canonical node');
  }
  return invoke<MacResponse>('mac_request', {
    request: {
      method: (init?.method ?? 'GET').toUpperCase(),
      path,
      body: body ?? null,
      headers: headerPairs(init?.headers),
    },
  });
}

/** One binary answer from the Mac, as `mac_request_bytes` returns it. */
export interface MacBytes {
  status: number;
  contentType: string | null;
  bytes: Uint8Array;
}

/** Mirrors `SyncStatus` in `src-tauri/src/sync.rs`. Times are unix milliseconds. */
export interface SyncStatus {
  offline: boolean;
  offline_since: number | null;
  /** The oldest fetch time among the copies shown since the Mac stopped answering. */
  showing_from: number | null;
  pending: number;
  conflicts: number;
  failed: number;
  store_error: string | null;
}

/** Mirrors `OutboxEntry` in `src-tauri/src/sync.rs`: one queued item edit. */
export interface OutboxEntry {
  id: number;
  item_id: string;
  method: 'PUT' | 'PATCH';
  path: string;
  body: Record<string, unknown>;
  if_match: string;
  state: 'pending' | 'conflict' | 'failed';
  error: string | null;
  /** On a conflict: the Mac's `current` from the 409 body. */
  current: { item: Record<string, unknown>; state: string | null } | null;
  attempts: number;
  created_at: number;
  updated_at: number;
}

/**
 * Reads the frame `frame_bytes` in `src-tauri/src/mac_bridge.rs` writes:
 * status (u16 BE), content-type length (u16 BE), content-type, body.
 */
export function decodeBytesFrame(raw: ArrayBuffer | ArrayBufferView | number[]): MacBytes {
  const all =
    raw instanceof ArrayBuffer
      ? new Uint8Array(raw)
      : ArrayBuffer.isView(raw)
        ? new Uint8Array(raw.buffer, raw.byteOffset, raw.byteLength)
        : Uint8Array.from(raw);
  if (all.length < 4) throw new Error('mac-bridge: the binary answer is shorter than its header');
  const status = (all[0] << 8) | all[1];
  const typeLength = (all[2] << 8) | all[3];
  if (all.length < 4 + typeLength) throw new Error('mac-bridge: the binary answer is cut short');
  const type = new TextDecoder().decode(all.subarray(4, 4 + typeLength));
  return { status, contentType: type || null, bytes: all.subarray(4 + typeLength) };
}

/**
 * Fetches one canonical-node path as bytes (GET only). The native side refuses an answer
 * above 64 MiB. Rejects with the native error string, as `macRequest` does.
 */
export async function macRequestBytes(path: string): Promise<MacBytes> {
  const raw = await invoke<ArrayBuffer | ArrayBufferView | number[]>('mac_request_bytes', {
    path,
    headers: null,
  });
  return decodeBytesFrame(raw);
}

export function syncStatus(): Promise<SyncStatus> {
  return invoke<SyncStatus>('sync_status');
}

export function syncEntries(): Promise<OutboxEntry[]> {
  return invoke<OutboxEntry[]>('sync_entries');
}

/** Sends pending edits now. Without a canonical node address it sends nothing. */
export function syncFlush(): Promise<SyncStatus> {
  return invoke<SyncStatus>('sync_flush');
}

/** `keep_mine` re-sends against the Mac's current revision; `discard` drops the edit. */
export function syncResolve(id: number, action: 'keep_mine' | 'discard'): Promise<SyncStatus> {
  return invoke<SyncStatus>('sync_resolve', { id, action });
}

export function getConnectionSettings(): Promise<ConnectionSettings> {
  return invoke<ConnectionSettings>('connection_settings_get');
}

export function setCanonicalBaseUrl(canonicalBaseUrl: string): Promise<ConnectionSettings> {
  return invoke<ConnectionSettings>('connection_settings_set', { canonicalBaseUrl });
}

/** The error text of a failed `invoke`, which Tauri rejects with as a bare string. */
/** Saves the node's local-network address and its certificate pin. An empty URL clears it. */
export function setLocalEndpoint(baseUrl: string, pinSha256: string): Promise<ConnectionSettings> {
  return invoke<ConnectionSettings>('connection_local_set', { baseUrl, pinSha256 });
}

/** Browses the local network for Sjel nodes for a few seconds. iOS app only. */
export function browseLocalNetwork(timeoutMs = 3000): Promise<{ nodes: FoundNode[]; denied: boolean }> {
  return invoke('plugin:local-network|browse', { timeoutMs });
}

export function bridgeErrorText(error: unknown): string {
  if (typeof error === 'string') return error;
  if (error instanceof Error) return error.message;
  return String(error);
}
