import { describe, expect, test } from 'bun:test';
import { fitsWindow, plan, reasonText, type ModelTask, type RungStatus } from '../src/lib/intelligence/ladder';

const task = (chars: number, over: Partial<ModelTask> = {}): ModelTask => ({
  kind: 'summarize',
  prompt: 'x'.repeat(chars),
  ...over,
});

const onDevice = (over: Partial<RungStatus> = {}): RungStatus => ({
  rung: 'on-device',
  available: true,
  contextTokens: 4096,
  ...over,
});
const local = (over: Partial<RungStatus> = {}): RungStatus => ({ rung: 'local', available: true, contextTokens: 4096, ...over });
const mac = (over: Partial<RungStatus> = {}): RungStatus => ({ rung: 'mac', available: true, contextTokens: 4096, ...over });

describe('plan', () => {
  test('tries the phone, local assistant, Mac, then rules', () => {
    expect(plan(task(100), [onDevice(), local(), mac()]).tries).toEqual(['on-device', 'local', 'mac', 'rules']);
  });

  test('an ineligible phone goes straight to the Mac and says why', () => {
    const result = plan(task(100), [onDevice({ available: false, reason: 'deviceNotEligible' }), local(), mac()]);
    expect(result.tries).toEqual(['local', 'mac', 'rules']);
    expect(result.skipped).toEqual([{ rung: 'on-device', reason: 'deviceNotEligible' }]);
  });

  test('with no model anywhere, rules still answer', () => {
    expect(plan(task(100), []).tries).toEqual(['rules']);
  });

  test('a task too long for a 4,096 window skips that rung rather than truncating', () => {
    const result = plan(task(12_000), [onDevice(), local(), mac({ contextTokens: 32_000 })]);
    expect(result.tries).toEqual(['mac', 'rules']);
    expect(result.skipped).toEqual([
      { rung: 'on-device', reason: 'too long for this model' },
      { rung: 'local', reason: 'too long for this model' },
    ]);
  });
});

describe('fitsWindow', () => {
  // Same cases as libs/summarize/src/lib.rs: 8,000 characters at a ~500-token reply fits 4,096.
  test('matches the summarize arithmetic', () => {
    expect(fitsWindow(task(8_000, { maxResponseTokens: 500 }), 4096)).toBe(true);
    expect(fitsWindow(task(14_064, { maxResponseTokens: 1_000 }), 4096)).toBe(false);
  });

  test('the reply counts against the window', () => {
    expect(fitsWindow(task(9_000, { maxResponseTokens: 100 }), 4096)).toBe(true);
    expect(fitsWindow(task(9_000, { maxResponseTokens: 1_000 }), 4096)).toBe(false);
  });

  test('an unknown window defers to the backend', () => {
    expect(fitsWindow(task(1_000_000), undefined)).toBe(true);
  });
});

test('reasons read as plain words', () => {
  expect(reasonText('appleIntelligenceNotEnabled')).toBe('Apple Intelligence is off in Settings');
  expect(reasonText('something new')).toBe('something new');
});
