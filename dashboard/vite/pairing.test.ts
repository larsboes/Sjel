import { describe, expect, test } from 'bun:test';

import { joinCode, keyCode, pairingQrPayload, parsePairingQr } from '../src/lib/devices';

describe('guided pairing', () => {
  test('the key code matches the node (capabilities/devices/src/store.rs, key_code)', async () => {
    expect(await keyCode('ab'.repeat(32))).toBe('571 846');
    expect(await keyCode('AB'.repeat(32))).toBe('571 846');
  });

  test('a pairing QR round-trips', () => {
    const qr = { host: 'Lars-Mac-2', port: 8443, fingerprint: 'a'.repeat(64), challenge_id: 'pair_1', code: 'ABCDEFGHJK' };
    expect(parsePairingQr(pairingQrPayload(qr))).toEqual(qr);
  });

  test('a code that is not a Sjel pairing code is refused', () => {
    expect(() => parsePairingQr('https://example.com')).toThrow('not a Sjel pairing code');
    expect(() => parsePairingQr('{"k":"other"}')).toThrow('not a Sjel pairing code');
    const bad = JSON.stringify({ k: 'sjel-pair/1', h: 'mac', p: 8443, f: 'short', i: 'x', c: 'y' });
    expect(() => parsePairingQr(bad)).toThrow('damaged');
    const badHost = JSON.stringify({ k: 'sjel-pair/1', h: 'evil.example.com', p: 8443, f: 'a'.repeat(64), i: 'x', c: 'y' });
    expect(() => parsePairingQr(badHost)).toThrow('damaged');
  });

  test('the join code puts the certificate half first', () => {
    expect(joinCode('A6C2 45F2 C26E F925', '571 846')).toBe('A6C2 45F2 C26E F925 · 571 846');
  });
});
