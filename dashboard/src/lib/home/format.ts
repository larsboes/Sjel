/** Date and count wording the ladder's rows share, so eight rows cannot phrase it eight ways. */

import { dayLabel, daysSince } from "../ui/age";

export function localDateKey(date: Date): string {
  return [
    date.getFullYear(),
    String(date.getMonth() + 1).padStart(2, "0"),
    String(date.getDate()).padStart(2, "0"),
  ].join("-");
}

/** Whole days from `from` to `value`, both read at local noon so DST cannot shift one. */
export function daysUntil(value: string, from: Date): number {
  const date = new Date(`${value.slice(0, 10)}T12:00:00`);
  if (Number.isNaN(date.getTime())) return 365;
  const start = new Date(from);
  start.setHours(12, 0, 0, 0);
  return Math.ceil((date.getTime() - start.getTime()) / 86_400_000);
}

export function dateLabel(value: string): string {
  const date = new Date(`${value.slice(0, 10)}T12:00:00`);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleDateString("en-GB", { day: "numeric", month: "short" });
}

export function relativeDate(value: string, now = new Date()): string {
  const days = daysSince(value, now.getTime());
  if (days === null) return "";
  // A day the ladder has not reached is named; an older one is dated. A day in the future
  // has no age, so it takes the date too.
  return days >= 0 && days < 7 ? dayLabel(days) : dateLabel(value);
}

export function countLabel(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

export function sentenceCase(value: string): string {
  return value.charAt(0).toLocaleUpperCase("en-GB") + value.slice(1);
}

/** The one place the row's meta line is assembled, so no row invents a fourth separator.
 *
 * Returns the PARTS. It used to join them with " · ", which put a character between two
 * facts where the layout can put space — and the same middle dot was doing that job in
 * 166 places across the app, which is what made every meta line read the same as every
 * other generated one. `.row-kind` spaces them now, so the separator is a gap that can be
 * tuned per surface rather than a glyph baked into a string. */
export function metaParts(...parts: (string | null | undefined | false)[]): string[] {
  return parts.filter(Boolean) as string[];
}
