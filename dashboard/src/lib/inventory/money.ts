// Inventory keeps money in cents (capabilities/inventory/src/store.rs, `price_cents`).

const EUR = new Intl.NumberFormat('de-DE', { style: 'currency', currency: 'EUR' });

export const euro = (cents: number): string => EUR.format(cents / 100);

/**
 * `39,90`, `39.90`, `1.234,56` or `39,90 €` -> cents. Empty clears the price (`null`);
 * anything else that is not one amount is `undefined`, so a typo never becomes a number.
 */
export function centsFrom(text: string | null): number | null | undefined {
  let t = (text ?? '').replace(/€|eur/gi, '').trim();
  if (!t) return null;
  if (t.includes('.') && t.includes(',')) t = t.replace(/\./g, '');
  t = t.replace(',', '.');
  if (!/^\d+(\.\d{1,2})?$/.test(t)) return undefined;
  return Math.round(Number(t) * 100);
}
