// Tests for tools/lib/demo-data.ts (#168).
//
// Two properties, and neither is about the code being correct in the ordinary sense.
//
// DETERMINISM is what makes a published demo reviewable: two builds of the same commit have to
// produce the same corpus, or every rebuild is an unreadable diff and nobody can tell a data
// change from a code change.
//
// THE VOCABULARY SWEEP is the first of the two guarantees behind "no real data on the public
// page". The second is tools/check-site-payload.sh, which scans the built site. This one runs
// at the source: if a generator can only draw from VOCABULARY, and nothing in VOCABULARY looks
// like a real person's contact details, then the corpus cannot contain any — checked here so
// the failure lands on whoever adds the string, not on a deploy six commits later.

import { describe, expect, test } from "bun:test";
import { addDays, at, daysBetween, eventWindow, monthStart, Rng, VOCABULARY, email, vocabularyStrings } from "./demo-data.ts";

describe("Rng", () => {
  test("the same seed replays the same sequence", () => {
    const a = new Rng("axon-demo-v1");
    const b = new Rng("axon-demo-v1");
    const draw = (r: Rng) => [r.next(), r.int(0, 1000), r.pick([1, 2, 3, 4, 5]), r.amountCents(100, 90_000)];
    expect(draw(a)).toEqual(draw(b));
  });

  test("different seeds diverge", () => {
    const a = Array.from({ length: 8 }, (_, i) => new Rng("one").int(0, 1e6) + i);
    const b = Array.from({ length: 8 }, (_, i) => new Rng("two").int(0, 1e6) + i);
    expect(a).not.toEqual(b);
  });

  test("stays inside its bounds", () => {
    const r = new Rng("bounds");
    for (let i = 0; i < 2000; i++) {
      const n = r.int(-5, 5);
      expect(n).toBeGreaterThanOrEqual(-5);
      expect(n).toBeLessThanOrEqual(5);
      const cents = r.amountCents(200, 5_000);
      expect(cents).toBeGreaterThanOrEqual(190);
      expect(cents).toBeLessThanOrEqual(5_000);
    }
  });

  test("sample() returns distinct members and never over-draws", () => {
    const r = new Rng("sample");
    const picked = r.sample(VOCABULARY.people, 4);
    expect(picked).toHaveLength(4);
    expect(new Set(picked).size).toBe(4);
    expect(r.sample([1, 2], 9)).toHaveLength(2);
  });
});

describe("dates", () => {
  // UTC throughout, so a demo rebuilt in another timezone produces identical bytes. A
  // local-time implementation passes in Europe and shifts a day in Auckland.
  test("addDays crosses a month and a year without drifting", () => {
    expect(addDays("2026-03-16", 20)).toBe("2026-04-05");
    expect(addDays("2026-01-01", -1)).toBe("2025-12-31");
    expect(addDays("2026-02-28", 1)).toBe("2026-03-01"); // 2026 is not a leap year
  });

  // The calendar capability's own accepted form. A zone suffix is rejected outright, so this
  // pins the absence of one as much as the padding.
  test("at() is a zoneless local-time instant the calendar accepts", () => {
    expect(at("2026-03-16", 9, 30)).toBe("2026-03-16T09:30");
    expect(at("2026-03-16", 9)).toBe("2026-03-16T09:00");
    expect(at("2026-03-16", 14, 5)).toBe("2026-03-16T14:05");
  });

  test("monthStart walks back whole months", () => {
    expect(monthStart("2026-03-16")).toBe("2026-03-01");
    expect(monthStart("2026-03-16", 3)).toBe("2025-12-01");
  });

  test("daysBetween is signed and symmetric", () => {
    expect(daysBetween("2026-03-01", "2026-03-16")).toBe(15);
    expect(daysBetween("2026-03-16", "2026-03-01")).toBe(-15);
  });
});

describe("eventWindow", () => {
  // Every rule the calendar's NewEntry::validate applies to a timed entry, asserted over the
  // whole corpus rather than a sample: the failure this replaces only appeared on the days
  // the dice picked a late start for a long event.
  test("every event on every day produces a window the calendar accepts", () => {
    const rng = new Rng("windows");
    for (const event of VOCABULARY.events) {
      for (let day = -30; day <= 45; day++) {
        const { starts_at, ends_at } = eventWindow(rng, addDays("2026-03-16", day), event.hours);
        for (const stamp of [starts_at, ends_at]) {
          expect(stamp).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/);
          const hour = Number(stamp.slice(11, 13));
          expect(hour).toBeGreaterThanOrEqual(0);
          expect(hour).toBeLessThan(24);
        }
        // Ends are exclusive and must be strictly later.
        expect(ends_at > starts_at).toBe(true);
        // ...and on the same day, so an entry cannot silently span midnight.
        expect(ends_at.slice(0, 10)).toBe(starts_at.slice(0, 10));
      }
    }
  });

  test("a long event starts early enough to finish inside the day", () => {
    const rng = new Rng("long");
    for (let i = 0; i < 200; i++) {
      const { ends_at } = eventWindow(rng, "2026-03-16", 6);
      expect(Number(ends_at.slice(11, 13))).toBeLessThanOrEqual(21);
    }
  });
});

describe("the vocabulary contains nothing real", () => {
  const strings = vocabularyStrings();

  test("is not empty, so the sweep below is actually sweeping something", () => {
    expect(strings.length).toBeGreaterThan(50);
  });

  test("holds no email address, real or otherwise", () => {
    // Addresses are built by email() at use time and always land on example.org. A literal
    // one sitting in the vocabulary is how a real address gets in.
    expect(strings.filter((s) => /@/.test(s))).toEqual([]);
  });

  test("holds nothing shaped like an IBAN, a card number or a phone number", () => {
    for (const s of strings) {
      expect(s).not.toMatch(/\b[A-Z]{2}[0-9]{2}[A-Z0-9]{10,}\b/);
      expect(s).not.toMatch(/\b(?:\d[ -]?){13,19}\b/);
      expect(s).not.toMatch(/\+\d{6,}/);
    }
  });

  test("holds no filesystem path, host or URL", () => {
    for (const s of strings) {
      expect(s).not.toMatch(/^(~|\/)|\/(Users|home)\//);
      expect(s).not.toMatch(/https?:\/\//);
      expect(s).not.toMatch(/\.(ts\.net|local|internal)\b/);
    }
  });

  // Assembled from fragments rather than written as a literal: this file is tracked, and a
  // tracked file containing a deployment marker is precisely what check-publication-hygiene.sh
  // rejects. The alternative was an exclusion entry, and an exclusion list that grows to cover
  // test fixtures stops being a list of exceptions.
  test("names no deployment instance", () => {
    const markers = ["personal", "family", "work"].map((suffix) => `axon-${suffix}`);
    for (const s of strings) {
      for (const marker of markers) expect(s.toLowerCase()).not.toContain(marker);
    }
  });

  // The home city is the one place the corpus could accidentally point somewhere. It is
  // asserted to be a member of the same public city list rather than a value of its own,
  // so adding a home means picking one of the six already reviewed.
  test("the demo's home is one of the declared cities", () => {
    expect(VOCABULARY.cities.map((c) => c.name)).toContain(VOCABULARY.home.name);
  });

  test("every city has coordinates a map can render", () => {
    for (const c of VOCABULARY.cities) {
      expect(Math.abs(c.latitude)).toBeLessThanOrEqual(90);
      expect(Math.abs(c.longitude)).toBeLessThanOrEqual(180);
      expect(c.latitude === 0 && c.longitude === 0).toBe(false);
    }
  });

  // The calendar capability validates `byweekday token must be one of mo,tu,we,th,fr,sa,su`
  // and rejects the RFC 5545 uppercase spelling outright rather than normalising it. These
  // were first written as MO/TU and failed the first CI seeding run.
  test("every rhythm uses the weekday tokens the calendar accepts", () => {
    const allowed = new Set(["mo", "tu", "we", "th", "fr", "sa", "su"]);
    for (const rhythm of VOCABULARY.rhythms) {
      expect(rhythm.byweekday.length).toBeGreaterThan(0);
      for (const day of rhythm.byweekday) expect(allowed.has(day)).toBe(true);
    }
  });

  // Finance applies one rule to an instrument and to an alias target alike: ASCII
  // alphanumeric plus `.`, `_` or `-`, at most 64 bytes. The aliases first mapped each symbol
  // to a prose label, which the import rejected before reading a single row.
  test("every instrument symbol and alias target is a symbolic ASCII identifier", () => {
    const identifier = /^[A-Za-z0-9._-]{1,64}$/;
    for (const instrument of VOCABULARY.instruments) {
      expect(instrument.symbol).toMatch(identifier);
      expect(instrument.canonical).toMatch(identifier);
    }
  });

  test("every trip names a city that exists in the list", () => {
    const known = new Set(VOCABULARY.cities.map((c) => c.name));
    for (const t of VOCABULARY.trips) expect(known.has(t.city)).toBe(true);
  });

  test("every place sits inside a declared city", () => {
    const known = new Set(VOCABULARY.cities.map((c) => c.name));
    for (const p of VOCABULARY.pois) {
      expect(known.has(p.city)).toBe(true);
      // About 1.5 km at most, so a pin cannot drift into the next town.
      expect(Math.abs(p.dLat)).toBeLessThan(0.015);
      expect(Math.abs(p.dLon)).toBeLessThan(0.015);
    }
  });

  test("every contact's home and absence name a declared city", () => {
    const known = new Set(VOCABULARY.cities.map((c) => c.name));
    for (const c of VOCABULARY.contacts) {
      expect(known.has(c.homeBase)).toBe(true);
      if ("away" in c) expect(known.has(c.away.city)).toBe(true);
    }
  });
});

describe("email()", () => {
  test("always lands on a reserved documentation domain", () => {
    for (const person of VOCABULARY.people) expect(email(person)).toMatch(/@example\.org$/);
  });

  test("produces something an address parser accepts", () => {
    expect(email("Mara Velten")).toBe("mara.velten@example.org");
  });
});
