/**
 * Which ladder bands are expanded, and the memory of that across visits.
 *
 * Home rendered every commitment at once: 126 rows and 15,338px of page on the
 * morning this was written, with the most urgent row and the least urgent one
 * drawn at the same weight. The ladder already knows the answer — PRD §8.1 ranks
 * every row into four bands — so the fix is to render the ranking rather than
 * flatten it: the first band that has rows opens, the rest state their count and
 * wait to be asked.
 *
 * State lives in `localStorage`, not in a table. It is a per-device reading
 * preference, it must survive a reload without a round trip, and nothing else in
 * Sjel should have to know about it. Same reasoning the rail's `<details>`
 * sections already carry, made explicit here because this one persists.
 *
 * A band absent from the stored record is CLOSED, not defaulted-open. A record
 * written today must not silently open a band that did not exist when it was
 * written — the reader closed what they could see, and a new band is something
 * they have not seen yet. The one exception is the first ever visit, which has no
 * record at all and opens the leading band so the page is not a wall of headings.
 */

const KEY = "axon.home.bands";

function load(): Record<string, boolean> | null {
  if (typeof localStorage === "undefined") return null;
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return null;
    // Values from disk are not trusted to be booleans: a hand-edited or
    // half-migrated record must not make `open` return a string.
    return Object.fromEntries(
      Object.entries(parsed as Record<string, unknown>).map(([k, v]) => [k, v === true]),
    );
  } catch {
    // Private browsing, a full quota or a corrupt value. A reading preference is
    // never worth an error boundary, so the page falls back to the first visit.
    return null;
  }
}

export function createBandDisclosure() {
  let stored = $state<Record<string, boolean> | null>(load());

  return {
    /** `leading` is the first band that has rows — open on a first visit only. */
    isOpen(band: string, leading: string): boolean {
      if (stored === null) return band === leading;
      return stored[band] === true;
    },

    toggle(band: string, leading: string) {
      // The first toggle materialises the whole record, so the bands the reader
      // left alone are written as closed rather than left to the null branch and
      // re-opened by a later leading-band change.
      const next = { ...(stored ?? { [leading]: true }) };
      next[band] = !(stored === null ? band === leading : stored[band] === true);
      stored = next;
      try {
        localStorage.setItem(KEY, JSON.stringify(next));
      } catch {
        // Kept in memory for this session; the preference simply does not persist.
      }
    },
  };
}
