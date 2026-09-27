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
