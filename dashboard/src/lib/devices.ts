import { invoke } from '@tauri-apps/api/core';
import { bridgeErrorText, inTauri } from './mac-bridge';
import { jsonInit, request } from './api';

export interface PairingChallenge {
  protocol_version: 'axon-pairing/v1';
  challenge_id: string;
  code: string;
  expires_at: number;
  qr_payload: string;
}

export interface DeviceRecord {
  id: string;
  label: string;
  platform: string;
  algorithm: 'ed25519';
  public_key: string;
  fingerprint: string;
  status: 'active' | 'revoked';
  created_at: number;
  last_seen_at: number | null;
  revoked_at: number | null;
}

export interface DeviceClaim {
  challenge_id: string;
  code: string;
  label: string;
  platform: string;
  algorithm: 'ed25519';
  public_key: string;
}

export interface DeviceIdentity {
  id: string;
  platform: string;
  algorithm: 'ed25519';
  public_key: string;
}

const base = '/devices/api';

export function canClaimOnThisDevice(): boolean {
  return inTauri() && typeof navigator !== 'undefined' && /iPhone|iPad|iPod/i.test(navigator.userAgent);
}

export async function getDeviceIdentity(): Promise<DeviceIdentity> {
  if (!canClaimOnThisDevice()) {
    throw new Error('Device identity is available only inside the Sjel iOS app.');
  }
  try {
    return await invoke<DeviceIdentity>('device_identity_get');
  } catch (error) {
    throw new Error(bridgeErrorText(error));
  }
}

export async function resetDeviceIdentity(): Promise<DeviceIdentity> {
  if (!canClaimOnThisDevice()) {
    throw new Error('Device identity is available only inside the Sjel iOS app.');
  }
  try {
    return await invoke<DeviceIdentity>('device_identity_reset');
  } catch (error) {
    throw new Error(bridgeErrorText(error));
  }
}

export const devices = {
  list: () => request<{ devices: DeviceRecord[] }>(`${base}/devices`),
  me: () => request<DeviceRecord>(`${base}/devices/me`),
  createChallenge: () =>
    request<PairingChallenge>(`${base}/pairing/challenges`, jsonInit('POST', {})),
  claim: (claim: DeviceClaim) =>
    request<DeviceRecord>(`${base}/pairing/claims`, jsonInit('POST', claim)),
  revoke: (id: string) =>
    request<DeviceRecord>(
      `${base}/devices/${encodeURIComponent(id)}/revoke`,
      jsonInit('POST', {}),
    ),
};

// ─── Guided pairing ──────────────────────────────────────────────────────────
//
// Two ways in, both ending in a registered key:
// 1. The Mac shows a QR code (`pairingQrPayload`). It carries the node's Same Wi-Fi address, its
//    certificate fingerprint and a one-time code, so one scan finds the Mac, pins it and claims.
//    The camera is the trusted channel; nothing is compared by eye.
// 2. The phone finds the Mac on the Wi-Fi and asks to join (`devices.requestJoin`). The owner
//    allows it on the Mac, where the same `joinCode` shows as on the phone. Its first half is the
//    certificate the phone pinned, which a relay cannot forge; its second half is the phone's key,
//    which tells this phone's request from any other.

export interface JoinRequest {
  id: string;
  label: string;
  platform: string;
  key_code: string;
  status: 'pending' | 'approved' | 'denied' | 'expired';
  created_at: number;
  expires_at: number;
  device: DeviceRecord | null;
}

/** What the Mac's pairing QR code carries. */
export interface PairingQr {
  host: string;
  port: number;
  fingerprint: string;
  challenge_id: string;
  code: string;
}

const QR_KIND = 'sjel-pair/1';

export function pairingQrPayload(qr: PairingQr): string {
  return JSON.stringify({ k: QR_KIND, h: qr.host, p: qr.port, f: qr.fingerprint, i: qr.challenge_id, c: qr.code });
}

/** Reads a scanned code. Anything that is not a Sjel pairing code is refused with a sentence. */
export function parsePairingQr(content: string): PairingQr {
  let value: Record<string, unknown>;
  try {
    value = JSON.parse(content) as Record<string, unknown>;
  } catch {
    throw new Error('That is not a Sjel pairing code.');
  }
  const { k, h, p, f, i, c } = value ?? {};
  if (k !== QR_KIND) throw new Error('That is not a Sjel pairing code.');
  if (
    typeof h !== 'string' || !/^[A-Za-z0-9-]{1,63}$/.test(h) ||
    typeof p !== 'number' || !Number.isInteger(p) || p < 1 || p > 65535 ||
    typeof f !== 'string' || !/^[0-9a-f]{64}$/.test(f) ||
    typeof i !== 'string' || i === '' ||
    typeof c !== 'string' || c === ''
  ) {
    throw new Error('This pairing code is damaged. Show a new one on the Mac.');
  }
  return { host: h, port: p, fingerprint: f, challenge_id: i, code: c };
}

/** Six digits from the key, `482 913`. Mirrors `key_code` in capabilities/devices/src/store.rs. */
export async function keyCode(publicKey: string): Promise<string> {
  const data = new TextEncoder().encode(`sjel-pairing-key/v1:${publicKey.trim().toLowerCase()}`);
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', data));
  const value = (((digest[0] << 24) | (digest[1] << 16) | (digest[2] << 8) | digest[3]) >>> 0) % 1_000_000;
  const digits = String(value).padStart(6, '0');
  return `${digits.slice(0, 3)} ${digits.slice(3)}`;
}

/** The code both screens show while a join waits: certificate half, then key half. */
export function joinCode(certificateCode: string, key: string): string {
  return `${certificateCode} · ${key}`;
}

export const joins = {
  request: (body: { label: string; platform: string; algorithm: 'ed25519'; public_key: string }) =>
    request<JoinRequest>(`${base}/pairing/requests`, jsonInit('POST', body)),
  status: (id: string) => request<JoinRequest>(`${base}/pairing/requests/${encodeURIComponent(id)}`),
  pending: () => request<{ requests: JoinRequest[] }>(`${base}/pairing/requests`).then((r) => r.requests),
  approve: (id: string) =>
    request<JoinRequest>(`${base}/pairing/requests/${encodeURIComponent(id)}/approve`, jsonInit('POST', {})),
  deny: (id: string) =>
    request<JoinRequest>(`${base}/pairing/requests/${encodeURIComponent(id)}/deny`, jsonInit('POST', {})),
};
