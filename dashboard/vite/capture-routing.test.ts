import { describe, expect, test } from 'bun:test';
import { routeFor, slug, suggestTag } from '../src/lib/capture';

describe('capture routing', () => {
  test('a set tag wins over the domain', () => {
    expect(suggestTag({ tag: 'buy', url: 'https://github.com/x/y' })).toBe('buy');
    expect(routeFor({ tag: 'buy', url: null })).toBe('wishlist');
  });

  test('a known feed domain suggests look and goes to the feed', () => {
    expect(suggestTag({ tag: null, url: 'https://www.reddit.com/r/x' })).toBe('look');
    expect(routeFor({ tag: null, url: 'https://old.reddit.com/r/x' })).toBe('feed');
    expect(routeFor({ tag: null, url: 'https://notgithub.com/x' })).toBeNull();
  });

  test('a note without a link stays in the inbox unless it is a buy', () => {
    expect(routeFor({ tag: 'learn', url: null })).toBeNull();
    expect(routeFor({ tag: 'go', url: 'https://example.com' })).toBeNull();
  });

  test('a slug reads like an inventory id', () => {
    expect(slug('Proper shoes: maybe On?')).toBe('proper-shoes-maybe-on');
    expect(slug('Größe')).toBe('grose');
    expect(slug('???')).toBe('wunsch');
  });
});
