import { describe, expect, test } from 'bun:test';
import { runtimeView, type RuntimeStatus } from '../src/lib/runtime';

const status: RuntimeStatus = {
  device: 'test', selection: 'auto', effective: 'on-the-go', power: 'battery',
  power_fresh: true, allow: [], revision: 1, detail: null, configured: true,
};

describe('runtime presentation', () => {
  test('AFM only requires both model exceptions off and On the go active', () => {
    expect(runtimeView(status).afmOnly).toBe(true);
    for (const category of ['other-local-models', 'remote-models'] as const) {
      expect(runtimeView({ ...status, allow: [category] }).afmOnly).toBe(false);
    }
    expect(runtimeView({ ...status, effective: 'normal' }).afmOnly).toBe(false);
    expect(runtimeView({ ...status, allow: ['transcription'] }).afmOnly).toBe(true);
  });
  test('exceptions are persistent but only active in On the go', () => {
    expect(runtimeView({ ...status, allow: ['transcription'] }).exceptions).toEqual(['Transcription']);
    expect(runtimeView({ ...status, effective: 'normal', allow: ['transcription'] }).exceptions).toEqual([]);
  });
  test('source distinguishes Auto, manual, unconfigured and stale power', () => {
    expect(runtimeView(status).source).toBe('Auto · Battery');
    expect(runtimeView({ ...status, selection: 'normal' }).source).toBe('Manual');
    expect(runtimeView({ ...status, configured: false }).source).toBe('Not configured');
    expect(runtimeView({ ...status, power_fresh: false }).source).toContain('stale');
  });
});
