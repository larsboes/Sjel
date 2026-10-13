import { expect, test } from 'bun:test';
import { centsFrom, euro } from '../src/lib/inventory/money';

test('a typed price becomes cents, and a typo becomes nothing', () => {
  expect(centsFrom('39,90')).toBe(3990);
  expect(centsFrom('39.9')).toBe(3990);
  expect(centsFrom('1.234,56 €')).toBe(123456);
  expect(centsFrom(euro(4990))).toBe(4990);
  expect(centsFrom('  ')).toBeNull();
  expect(centsFrom(null)).toBeNull();
  expect(centsFrom('ab 79')).toBeUndefined();
  expect(centsFrom('79.901')).toBeUndefined();
  expect(centsFrom('1.234')).toBeUndefined();
});
