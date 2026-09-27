// tools/lib/demo-data.ts — a deterministic source of fictional values (#168).
//
// This module knows nothing about capabilities or HTTP. It answers "give me a plausible
// merchant name" and "give me a date eleven days after the anchor", always with the same
// answer for the same seed. tools/demo-seed decides what to do with them.
//
// The split is the point. Every shape in the demo — what a transaction looks like, which
// endpoint accepts it — lives next to the API it targets, in demo-seed.ts. Everything a
// reviewer has to eyeball for accidental realism lives here, in one file that imports
// nothing and can be read top to bottom in a minute.
//
// TWO RULES GOVERN THE VOCABULARY, and tools/lib/demo-data.test.ts enforces both:
//
//   1. Nothing here names a real person, company, or address. Merchants are invented
//      rather than borrowed — "Nordlicht Kaffee" instead of a chain that exists — because
//      a real brand in a screenshot invites a question nobody wants to answer, and because
//      an invented one is unmistakably a fixture the moment you read it.
//   2. Cities are real, and are chosen to be somewhere the principal is not. A map needs
//      coordinates that exist to render at all, and where a public demo says it is going
//      on holiday is not a private fact — but where its author actually lives might be, so
//      the set deliberately excludes it. The published-payload gate (tools/check-site-payload)
//      independently rejects the overlay's own home city if one ever appeared.

/** Deterministic 32-bit PRNG (mulberry32). Chosen for being eight lines and exactly
 *  reproducible across runtimes — a demo whose corpus depends on V8's internals would
 *  regenerate differently in CI than on the machine that reviewed it. */
export class Rng {
  private state: number;

  constructor(seed: string) {
    // FNV-1a over the seed string, so a human-readable seed becomes a 32-bit state.
    let h = 0x811c9dc5;
    for (let i = 0; i < seed.length; i++) {
      h ^= seed.charCodeAt(i);
      h = Math.imul(h, 0x01000193);
    }
    this.state = h >>> 0;
  }

  /** Uniform in [0, 1). */
  next(): number {
    this.state = (this.state + 0x6d2b79f5) >>> 0;
    let t = this.state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  }

  /** Uniform integer in [min, max]. */
  int(min: number, max: number): number {
    return min + Math.floor(this.next() * (max - min + 1));
  }

  pick<T>(items: readonly T[]): T {
    if (items.length === 0) throw new Error("demo-data: pick() from an empty list");
    return items[Math.floor(this.next() * items.length)];
  }

  /** `count` distinct members, or the whole list when it is shorter. */
  sample<T>(items: readonly T[], count: number): T[] {
    const pool = [...items];
    const out: T[] = [];
    while (out.length < count && pool.length > 0) {
      out.push(pool.splice(Math.floor(this.next() * pool.length), 1)[0]);
    }
    return out;
  }

  bool(trueProbability = 0.5): boolean {
    return this.next() < trueProbability;
  }

  /** A skewed amount in cents. Real spending is not uniform — a few large rents among
   *  many small coffees — and a uniform distribution makes every chart in the demo look
   *  like noise. Cubing the sample pushes the mass toward the floor. */
  amountCents(minCents: number, maxCents: number): number {
    const skew = this.next() ** 3;
    return Math.round((minCents + skew * (maxCents - minCents)) / 10) * 10;
  }
}

// ─── Dates ────────────────────────────────────────────────────────────────────

/** Calendar arithmetic in UTC. The capabilities store instants and the demo's anchor is a
 *  plain date, so every conversion here is deliberately timezone-free — a demo rebuilt in
 *  a different TZ must produce the same bytes, and local-time arithmetic would not. */
export function addDays(iso: string, days: number): string {
  const d = new Date(`${iso}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

/**
 * A local-time instant at a given hour on a date: `2026-03-16T09:30`.
 *
 * Deliberately NOT RFC3339. The calendar capability parses `YYYY-MM-DD` or
 * `YYYY-MM-DDTHH:MM[:SS]` and accepts no zone suffix at all — a trailing `Z` lands in its
 * seconds field, fails an integer parse, and comes back as "starts_at must be a date or local
 * time". Its model is local wall-clock time on purpose (capabilities/calendar/src/date.rs), so
 * an instant with a zone on it would be a different kind of value, not a formatting variant.
 */
export function at(iso: string, hour: number, minute = 0): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${iso}T${pad(hour)}:${pad(minute)}`;
}

/**
 * A timed entry's window on a date, as the calendar capability will accept it.
 *
 * Here rather than inline in the seeder because it encodes three of that capability's rules
 * at once, and all three are easy to break by arithmetic: an hour field must be under 24,
 * `ends_at` must be strictly after `starts_at`, and a non-all-day entry needs HH:MM on both.
 * The first draft picked a start hour in [8, 19] and added up to six hours, which produces
 * `25:00` for the longest events — rejected outright, and only on the days the dice chose it.
 *
 * The start window is therefore derived from the duration rather than fixed: a six-hour event
 * starts by 15:00, a one-hour event may start as late as 20:00, and nothing ends past 21:00.
 */
export function eventWindow(
  rng: Rng,
  date: string,
  hours: number,
): { starts_at: string; ends_at: string } {
  const duration = Math.min(12, Math.max(1, Math.round(hours)));
  const latestStart = Math.max(8, 21 - duration);
  const startHour = rng.int(8, latestStart);
  return { starts_at: at(date, startHour), ends_at: at(date, startHour + duration) };
}

/** First day of the month `back` months before `iso`. */
export function monthStart(iso: string, back = 0): string {
  const d = new Date(`${iso}T00:00:00Z`);
  d.setUTCDate(1);
  d.setUTCMonth(d.getUTCMonth() - back);
  return d.toISOString().slice(0, 10);
}

export function daysBetween(from: string, to: string): number {
  const a = Date.parse(`${from}T00:00:00Z`);
  const b = Date.parse(`${to}T00:00:00Z`);
  return Math.round((b - a) / 86_400_000);
}

// ─── Vocabulary ───────────────────────────────────────────────────────────────
//
// Exported as one object so demo-data.test.ts can sweep every string in it against the
// same forbidden-pattern set the published-payload gate uses. A generator that reaches
// past this object for a literal defeats that sweep, so they do not.

export const VOCABULARY = {
  /** Invented people. Used as travellers and as the counterparty on a shared expense. */
  people: [
    "Mara Velten",
    "Tomas Iversen",
    "Juno Halvorsen",
    "Priya Rasmussen",
    "Emil Sandoval",
    "Noor Lindqvist",
  ],

  /** What the entity store holds about each of `people`. `birthdayOffset` is days after the
   *  anchor, so Home has a birthday to raise; `away` is an absence covering the anchor, so the
   *  People map has somebody somewhere else today. Home cities come from `cities`. */
  contacts: [
    { name: "Mara Velten", relation: "Friend", homeBase: "Ghent", sleeping: "yes", interests: ["climbing", "film"], birthdayOffset: 9 },
    { name: "Tomas Iversen", relation: "Colleague", homeBase: "Copenhagen", sleeping: "ask", interests: ["cycling"], birthdayOffset: 140, away: { city: "Lisbon", fromOffset: -3, toOffset: 5 } },
    { name: "Juno Halvorsen", relation: "Friend", homeBase: "Tallinn", sleeping: "ask", interests: ["sailing", "board games"], birthdayOffset: 200 },
    { name: "Priya Rasmussen", relation: "Cousin", homeBase: "Turin", sleeping: "yes", interests: ["cooking"], birthdayOffset: 31 },
    { name: "Emil Sandoval", relation: "Neighbour", homeBase: "Ghent", sleeping: "none", interests: ["gardening"], birthdayOffset: 260 },
    { name: "Noor Lindqvist", relation: "Friend", homeBase: "Lisbon", sleeping: "yes", interests: ["photography", "running"], birthdayOffset: 75 },
  ],

  /** Real cities, none of them the principal's, with coordinates so a map renders.
   *  `region` is what the trips UI shows next to the name. */
  cities: [
    { name: "Lisbon", region: "Portugal", latitude: 38.7223, longitude: -9.1393 },
    { name: "Copenhagen", region: "Denmark", latitude: 55.6761, longitude: 12.5683 },
    { name: "Kraków", region: "Poland", latitude: 50.0647, longitude: 19.945 },
    { name: "Turin", region: "Italy", latitude: 45.0703, longitude: 7.6869 },
    { name: "Ghent", region: "Belgium", latitude: 51.0543, longitude: 3.7174 },
    { name: "Tallinn", region: "Estonia", latitude: 59.437, longitude: 24.7536 },
  ],

  /** The demo's home city. One of the above rather than a special case, so nothing in the
   *  corpus is anchored anywhere the real installation is. */
  home: { name: "Ghent", region: "Belgium", latitude: 51.0543, longitude: 3.7174 },

  /** Invented merchants, grouped by the ledger account a transaction from them posts to.
   *  The grouping is what makes the demo's category chart show structure rather than a
   *  flat bar — spending has a shape, and a fixture that does not have one demonstrates
   *  nothing about a tool built to find it. */
  merchants: [
    { name: "Nordlicht Kaffee", account: "expenses:food:cafe", min: 280, max: 950 },
    { name: "Markthal Grocers", account: "expenses:food:groceries", min: 1_200, max: 8_400 },
    { name: "Blau & Sohn Baumarkt", account: "expenses:home:supplies", min: 900, max: 14_000 },
    { name: "Velo Kollektiv", account: "expenses:transport:bike", min: 1_500, max: 22_000 },
    { name: "Stadtwerke Ghent", account: "expenses:home:utilities", min: 4_200, max: 11_800 },
    { name: "Regiobahn Tickets", account: "expenses:transport:rail", min: 480, max: 6_900 },
    { name: "Apotheke Zum Anker", account: "expenses:health:pharmacy", min: 620, max: 4_300 },
    { name: "Buchhandlung Silbe", account: "expenses:leisure:books", min: 890, max: 5_600 },
    { name: "Kantine Werk 4", account: "expenses:food:lunch", min: 640, max: 1_800 },
    { name: "Hafenbad Schwimmhalle", account: "expenses:health:sport", min: 450, max: 3_200 },
  ],

  /** Invented services, with a monthly price in cents. Drives the subscriptions page. */
  subscriptions: [
    { name: "Halden Notes", category: "tools", cents: 800 },
    { name: "Prisma Code Host", category: "tools", cents: 2_100 },
    { name: "Ferrit Object Storage", category: "infrastructure", cents: 1_150 },
    { name: "Kanto Music", category: "media", cents: 1_099 },
    { name: "Weidkamp Fitness", category: "health", cents: 3_490 },
    { name: "Loam Language Tutor", category: "learning", cents: 1_500 },
  ],

  /**
   * Invented instruments. Deliberately not real tickers: a demo portfolio quoting real
   * securities at invented prices is the one kind of fixture that could mislead somebody
   * who skims it.
   *
   * `symbol` is what the broker export carries; `canonical` is what the import's
   * `instrument_aliases` maps it to, which is the feature a real broker export needs. Both
   * are symbolic on purpose: Finance validates an alias TARGET with the same rule as an
   * instrument (ASCII alphanumeric plus `.`, `_`, `-`, max 64), so a human-readable
   * "Nordlys Broad Index" on the right-hand side is rejected outright.
   */
  instruments: [
    { symbol: "NDLX", canonical: "NORDLYS-BROAD-INDEX", unitCents: 8_420 },
    { symbol: "TRVA", canonical: "TERRAVIA-SUSTAINABLE", unitCents: 3_190 },
    { symbol: "KPST", canonical: "KEYSTONE-SHORT-BOND", unitCents: 10_240 },
  ],

  /** Calendar entries as {title, kind, hours}. `kind` matches the calendar capability's
   *  own vocabulary, so the demo exercises its real colour and feasibility rules. */
  events: [
    { title: "Standup", kind: "work_onsite", hours: 0.5 },
    { title: "Design review — ingest pipeline", kind: "work_onsite", hours: 1 },
    { title: "Bouldering", kind: "sport", hours: 2 },
    { title: "Dinner with Tomas", kind: "nightlife", hours: 3 },
    { title: "Language class", kind: "learning", hours: 1.5 },
    { title: "Dentist", kind: "appointment", hours: 1 },
    { title: "Long run", kind: "sport", hours: 1.5 },
    { title: "Team retro", kind: "work_onsite", hours: 1 },
    { title: "Parents visiting", kind: "family", hours: 6 },
    { title: "Concert — Kanto Live", kind: "nightlife", hours: 4 },
  ],

  /** Weekly rhythms. `byweekday` is lowercase because the calendar capability's own validator
   *  is: `byweekday token must be one of mo,tu,we,th,fr,sa,su`. The RFC 5545 spelling these
   *  were first written in (MO, TU) is rejected outright rather than normalised. */
  rhythms: [
    { title: "Bouldering", kind: "sport", byweekday: ["tu", "th"], start: "18:30", end: "20:30" },
    { title: "Language class", kind: "learning", byweekday: ["we"], start: "19:00", end: "20:30" },
    { title: "Standup", kind: "work_onsite", byweekday: ["mo", "tu", "we", "th", "fr"], start: "09:30", end: "09:45" },
  ],

  /** Trip titles, paired with how far ahead of the anchor they start. A demo with every
   *  trip in the future has no history and no completed state to show. */
  trips: [
    { title: "Long weekend in Lisbon", city: "Lisbon", offsetDays: 24, nights: 4 },
    { title: "Conference — Copenhagen", city: "Copenhagen", offsetDays: 61, nights: 3 },
    { title: "Kraków with Mara", city: "Kraków", offsetDays: -38, nights: 5 },
  ],

  /** Manual balances: accounts a bank CSV never covers. `kind` drives the net-worth sign. */
  balances: [
    { label: "Everyday account", kind: "asset" as const, cents: 412_300 },
    { label: "Savings", kind: "asset" as const, cents: 1_845_000 },
    { label: "Bike loan", kind: "liability" as const, cents: 78_400 },
  ],

  /** Employers and income lines. Invented, same rule as merchants. */
  income: [
    { description: "Salary — Halden Systems", account: "income:salary", cents: 289_000 },
    { description: "Invoice — Velten Studio", account: "income:freelance", cents: 62_500 },
  ],

  /** Articles the synthetic origin publishes, for Comms to ingest and Scouting to scan.
   *
   *  No URL here, and that is not an oversight: the sweep below forbids one, because a
   *  literal address in the vocabulary is how a real host gets published. tools/demo-origin
   *  builds every link from `slug` against its own listening address at serve time, the same
   *  way email() builds an address rather than storing one.
   *
   *  `kind` splits them: `reading` items are ordinary feed material, `opportunity` items
   *  carry a date and a place so Scouting has something with a deadline to rank. */
  articles: [
    {
      slug: "small-tools-that-outlive-their-authors",
      title: "Small tools that outlive their authors",
      kind: "reading" as const,
      summary: "Why the utilities that survive a decade are the ones nobody had to maintain.",
      body: "A tool that does one thing and states its contract can be left alone. One that grows a plugin system acquires an owner, and owners move on. The argument is not against ambition — it is about which parts you promise to keep.",
    },
    {
      slug: "reading-a-binary-format-you-did-not-design",
      title: "Reading a binary format you did not design",
      kind: "reading" as const,
      summary: "Schema drift, projection, and the columns you never noticed you depended on.",
      body: "The failure is rarely a corrupt file. It is a column quietly renamed upstream, read as absent, and folded in as a zero — a number that looks like a measurement and is not.",
    },
    {
      slug: "the-cost-of-a-second-system-of-record",
      title: "The cost of a second system of record",
      kind: "reading" as const,
      summary: "Two places that describe the same work will disagree by Friday.",
      body: "Every duplicated fact is a scheduled argument. The question is never whether the copies drift, only which one somebody trusts when they do.",
    },
    {
      slug: "measuring-what-you-already-collect",
      title: "Measuring what you already collect",
      kind: "reading" as const,
      summary: "Most systems throw away the number they were asked for.",
      body: "Before adding a metric, check whether the value is already in the request and simply never read. The instrumentation you skip is the instrumentation you cannot get wrong.",
    },
    {
      slug: "open-call-riverside-build-week",
      title: "Open call: Riverside Build Week",
      kind: "opportunity" as const,
      summary: "A week of shared workshop time for people building physical things.",
      body: "Bring a project and a plan for finishing it. Benches, tools and a kiln are provided; applications close a fortnight before the first session.",
      offsetDays: 38,
      city: "Ghent",
    },
    {
      slug: "grant-window-small-software-commons",
      title: "Grant window: Small Software Commons",
      kind: "opportunity" as const,
      summary: "Modest funding for maintained tools with no commercial path.",
      body: "Applications want a maintenance plan rather than a roadmap. Preference is given to work already in use by somebody other than its author.",
      offsetDays: 55,
      city: "Copenhagen",
    },
    {
      slug: "call-for-talks-northern-systems-days",
      title: "Call for talks: Northern Systems Days",
      kind: "opportunity" as const,
      summary: "Two days on operating small systems well.",
      body: "Thirty-minute slots, no keynotes, and a strong preference for talks about something that actually ran in production and broke.",
      offsetDays: 72,
      city: "Tallinn",
    },
  ],

  /** Rail stations for the synthetic HAFAS origin.
   *
   *  Names are generic rather than the real station names in these cities, and the ids are
   *  a deliberately synthetic 99xxxxx block — Deutsche Bahn's own EVA numbers start 80xxxxx
   *  for German stations, so nothing here can be mistaken for a real one or accidentally
   *  match a cell in punctuality's data. Seven digits, which keeps them clear of the
   *  card-number shape the sweep rejects. */
  stations: [
    { id: "9900001", name: "Ghent Central", latitude: 51.0543, longitude: 3.7174 },
    { id: "9900002", name: "Copenhagen Central", latitude: 55.6761, longitude: 12.5683 },
    { id: "9900003", name: "Turin Central", latitude: 45.0703, longitude: 7.6869 },
    { id: "9900004", name: "Kraków Central", latitude: 50.0647, longitude: 19.945 },
    { id: "9900005", name: "Lisbon Central", latitude: 38.7223, longitude: -9.1393 },
    { id: "9900006", name: "Tallinn Central", latitude: 59.437, longitude: 24.7536 },
  ],

  /** Train services the synthetic origin puts on those routes. `label` is what a real
   *  backend would put in `mitteltext`, and it is the field transit reads the punctuality
   *  train type off, so the shape matters: a label plus a number. */
  services: [
    { label: "ICE", gattung: "ICE", regional: false },
    { label: "IC", gattung: "IC_EC", regional: false },
    { label: "RE", gattung: "RB", regional: true },
    { label: "RB", gattung: "RB", regional: true },
  ],
} as const;

/** Every string anywhere in VOCABULARY, flattened. The test sweeps this; the payload gate
 *  sweeps the built site. Two passes over the same rule, at the two moments it can fail. */
export function vocabularyStrings(): string[] {
  const out: string[] = [];
  const walk = (value: unknown): void => {
    if (typeof value === "string") out.push(value);
    else if (Array.isArray(value)) value.forEach(walk);
    else if (value && typeof value === "object") Object.values(value).forEach(walk);
  };
  walk(VOCABULARY);
  return out;
}

/** An address that is syntactically an email and semantically nobody. RFC 2606 reserves
 *  example.org for exactly this, so a scanner can allow it by name without allowing a
 *  pattern that a real address could slip through. */
export function email(name: string): string {
  return `${name.toLowerCase().replace(/[^a-z]+/g, ".")}@example.org`;
}
