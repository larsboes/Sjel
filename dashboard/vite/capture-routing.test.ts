import { describe, expect, test } from 'bun:test';
import { extractPrice, routeFor, slug, suggestTag } from '../src/lib/capture';

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

  test('a note without a link stays in the inbox unless it is a buy or a go', () => {
    expect(routeFor({ tag: 'learn', url: null })).toBeNull();
    expect(routeFor({ tag: 'look', url: null })).toBeNull();
    expect(routeFor({ tag: 'go', url: 'https://example.com' })).toBe('places');
    expect(routeFor({ tag: 'go', url: null })).toBe('places');
  });

  test('a slug reads like an inventory id', () => {
    expect(slug('Proper shoes: maybe On?')).toBe('proper-shoes-maybe-on');
    expect(slug('Größe')).toBe('grose');
    expect(slug('???')).toBe('wunsch');
  });

  test('extractPrice parses typed prices from text at triage', () => {
    const { label, price_cents } = extractPrice('Linen Shirt 49.90 €');
    expect(label).toBe('Linen Shirt');
    expect(price_cents).toBe(4990);

    const euroOnly = extractPrice('Oxford Hemd 39,90');
    expect(euroOnly.label).toBe('Oxford Hemd');
    expect(euroOnly.price_cents).toBe(3990);

    const noPrice = extractPrice('Blue Jacket');
    expect(noPrice.label).toBe('Blue Jacket');
    expect(noPrice.price_cents).toBeUndefined();
  });
});
