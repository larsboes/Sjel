import { describe, expect, test } from 'vitest';
import { isWebUrl } from './string-utils';

describe('isWebUrl', () => {
	test('accepts http and https', () => {
		expect(isWebUrl('https://example.org/a')).toBe(true);
		expect(isWebUrl('http://example.org')).toBe(true);
	});
	test('refuses script, data and unparsable URLs', () => {
		expect(isWebUrl('javascript:alert(1)')).toBe(false);
		expect(isWebUrl('data:text/html,<b>x</b>')).toBe(false);
		expect(isWebUrl('not a url')).toBe(false);
	});
});
