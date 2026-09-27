import { describe, expect, test } from 'bun:test';
import { addressKind, comparisonCode, TRANSPORTS } from '../src/lib/connection/transports';

describe('connection options (PRD Q119)', () => {
  test('every option states at least one pro and one con', () => {
    for (const option of TRANSPORTS) {
      expect(option.pros.length).toBeGreaterThan(0);
      expect(option.cons.length).toBeGreaterThan(0);
    }
  });

  test('a saved address is a tailnet or a server', () => {
    expect(addressKind('https://mac.example.ts.net')).toBe('tailnet');
    expect(addressKind('https://sjel.example.com')).toBe('server');
    expect(addressKind(null)).toBeNull();
    expect(addressKind('not a url')).toBeNull();
  });

  test('the comparison code is the first 16 hex characters in four groups', () => {
    expect(comparisonCode('ab12cd34ef567890' + '0'.repeat(48))).toBe('AB12 CD34 EF56 7890');
  });
});
