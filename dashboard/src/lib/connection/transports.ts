/**
 * The ways a device can reach its Sjel node, in plain words, for the connection screen.
 *
 * PRD Q119 (2026-09-25): several transports side by side, and the person choosing sees each
 * one's pros and cons. None of them is the trust root: the device's paired key is, so every
 * option below carries the same signed requests (capabilities/devices/README.md).
 */

export type TransportId = 'local' | 'tailnet' | 'server' | 'icloud';

export interface TransportOption {
  id: TransportId;
  title: string;
  /** One line: what it is. */
  summary: string;
  pros: string[];
  cons: string[];
  /** `ready` is built; `later` is ruled but not built yet, and the screen says so. */
  availability: 'ready' | 'later';
}

export const TRANSPORTS: readonly TransportOption[] = [
  {
    id: 'local',
    title: 'Same Wi-Fi',
    summary: 'The phone finds your Mac on the home network by itself.',
    pros: ['Nothing to install and no account', 'Fast, and your data stays at home'],
    cons: ['Works only when the phone is on the same Wi-Fi as the Mac'],
    availability: 'ready',
  },
  {
    id: 'tailnet',
    title: 'Tailscale',
    summary: 'A private network between your devices, wherever they are.',
    pros: ['Works from anywhere', 'Encrypted end to end between your devices'],
    cons: ['Needs the Tailscale app and an account on every device'],
    availability: 'ready',
  },
  {
    id: 'server',
    title: 'Your server or a hosted Sjel',
    summary: 'Sjel runs on a machine that is always on and reachable.',
    pros: ['Works from anywhere', 'The Mac can sleep or be away'],
    cons: ['Someone has to run the server', 'Your data lives on that server'],
    availability: 'ready',
  },
  {
    id: 'icloud',
    title: 'iCloud',
    summary: 'Encrypted sync through your Apple account.',
    pros: ['Works from anywhere, and the Mac can sleep', 'Only your devices hold the key (PRD Q120)'],
    cons: ['Apple devices only', 'Not built yet'],
    availability: 'later',
  },
];

/** Which option a saved address is: a tailnet name, or any other https host. */
export function addressKind(url: string | null): 'tailnet' | 'server' | null {
  if (!url) return null;
  try {
    return new URL(url).hostname.endsWith('.ts.net') ? 'tailnet' : 'server';
  } catch {
    return null;
  }
}

/**
 * The fingerprint as the person compares it: the first 16 hex characters in four groups.
 * 64 bits is enough for a human comparison against a nearby attacker, who would have to find a
 * key whose certificate hash matches in the time the screen is open (my estimate, not measured).
 */
export function comparisonCode(fingerprint: string): string {
  return fingerprint
    .slice(0, 16)
    .toUpperCase()
    .replace(/(.{4})(?=.)/g, '$1 ');
}
