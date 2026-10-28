// Tests for tools/demo-origin.ts.
//
// Not "does the server respond". The origin exists so Comms, Scouting and Transit can be
// demonstrated without a hand-written fixture, and that only holds while what it serves is
// still the shape those three parse. Each assertion below names the consumer it protects:
//
//   the feed          capabilities/scouting/src/sources/rss.rs reads four tags per <item>
//   the article       capabilities/comms extracts a real page, so it has to look like one
//   dbnav / dbweb     capabilities/transit/src/hafas.rs, two different field vocabularies
//
// The one that matters most is the LAST: dbweb names a regional train by its bare number and
// carries no label, which is why a dbweb regional leg cannot be scored at all. A stub that
// helpfully supplied a label would make the demo show a number the real backend cannot
// produce, and would quietly delete the evidence for that decision.

import { describe, expect, test } from "bun:test";

import { handle } from "./demo-origin.ts";
import { loadManifest } from "./lib/demo-endpoints.ts";
import { VOCABULARY } from "./lib/demo-data.ts";

const manifest = loadManifest();
const BASE = manifest.origin;

const ask = (path: string, init?: RequestInit) =>
  handle(new Request(`${BASE}${path}`, init), manifest, BASE);

/** The dbnav request shape transit actually sends: station ids as the short lid form. */
const dbnavBody = (from: string, to: string, date: string) =>
  JSON.stringify({
    reiseHin: {
      wunsch: {
        abgangsLocationId: `A=1@L=${from}@`,
        zielLocationId: `A=1@L=${to}@`,
        zeitWunsch: { reiseDatum: `${date}T09:00:00` },
      },
    },
  });

const journeys = async (path: string, body: string) => {
  const res = await ask(path, { method: "POST", body, headers: { "Content-Type": "application/json" } });
  expect(res.status).toBe(200);
  return (await res.json()) as any;
};

describe("the feed scouting's rss adapter reads", () => {
  test("every item carries the four tags that adapter extracts", async () => {
    const xml = await (await ask("/feed.xml")).text();
    const items = xml.split("<item>").slice(1);
    expect(items).toHaveLength(VOCABULARY.articles.length);
    for (const item of items) {
      // rss.rs does lightweight tag extraction rather than full XML parsing, and an item
      // missing any of these is dropped without a word — which surfaces as a Scout page that
      // is simply short, with nothing to trace it back to.
      for (const tag of ["title", "link", "description", "pubDate"]) {
        expect(item).toContain(`<${tag}>`);
      }
    }
  });

  test("links point at this origin and nowhere else", async () => {
    const xml = await (await ask("/feed.xml")).text();
    for (const url of xml.match(/https?:\/\/[^<]+/g) ?? []) {
      // The one exception is the RSS namespace, which is a spec identifier rather than a
      // host anything fetches.
      if (url.startsWith("http://purl.org/")) continue;
      expect(url.startsWith(BASE)).toBe(true);
    }
  });
});

describe("the article comms extracts", () => {
  test("is an ordinary page with a heading and body paragraphs", async () => {
    const html = await (await ask(`/articles/${VOCABULARY.articles[0].slug}`)).text();
    expect(html).toContain("<h1>");
    expect(html).toContain('<meta name="description"');
    // Three paragraphs, one of which says out loud that this is a demonstration. A reader
    // who finds an extracted item should be able to tell from the item itself.
    expect(html.match(/<p[ >]/g)?.length).toBeGreaterThanOrEqual(3);
    expect(html).toContain("Written for a demonstration");
  });

  test("an unknown slug is a 404, not an invented page", async () => {
    expect((await ask("/articles/nothing-here")).status).toBe(404);
  });
});

describe("the journeys transit parses", () => {
  const [from, to] = VOCABULARY.stations;

  test("dbnav carries produktGattung and a label, the fields that vocabulary uses", async () => {
    const body = await journeys("/dbnav/fahrplan", dbnavBody(from.id, to.id, manifest.anchor));
    expect(body.verbindungen).toHaveLength(3);
    const sections = body.verbindungen[1].verbindung.verbindungsAbschnitte;
    expect(sections.length).toBeGreaterThan(1); // the journey with a change
    for (const section of sections) {
      expect(section.typ).toBe("FAHRZEUG");
      expect(typeof section.produktGattung).toBe("string");
      // The label is what transit reads the punctuality train type off, so it must survive
      // having its trailing digits trimmed and still name a type.
      expect(section.mitteltext.replace(/\s?\d+$/, "").trim()).not.toBe("");
      expect(section.abgangsOrt.evaNr).toMatch(/^\d{7}$/);
      // Offset-carrying, which is the half of the timestamp difference dbnav is responsible
      // for. Flattening it here would let a timezone bug pass the demo.
      expect(section.abgangsDatum).toMatch(/[+-]\d{2}:\d{2}$/);
    }
  });

  test("dbweb names a regional train by its bare number, defect included", async () => {
    const body = await journeys(
      "/fahrplan",
      JSON.stringify({ abfahrtsHalt: from.id, ankunftsHalt: to.id, anfrageZeitpunkt: `${manifest.anchor}T09:00:00` }),
    );
    const sections = body.verbindungen[1].verbindungsAbschnitte;
    const regional = sections.find((s: any) => s.verkehrsmittel.kategorie === "DRB");
    expect(regional).toBeDefined();
    // The point of the whole test file. bahn.de returns the bare number here and no label,
    // which is why transit reports that leg as unscorable rather than guessing a type from
    // `DRB` — a class code it uses for both RE and RB.
    expect(regional.verkehrsmittel.name).toMatch(/^\d+$/);
    // Naive local, no offset: the other half of the timestamp difference.
    expect(sections[0].halte[0].abfahrt.sollzeit).not.toMatch(/[+-]\d{2}:\d{2}$/);
  });

  test("the same query answers identically, which is what makes a rebuild diffable", async () => {
    const once = await journeys("/dbnav/fahrplan", dbnavBody(from.id, to.id, manifest.anchor));
    const twice = await journeys("/dbnav/fahrplan", dbnavBody(from.id, to.id, manifest.anchor));
    expect(twice).toEqual(once);
  });

  test("a different route answers differently, so the seed is doing work", async () => {
    const a = await journeys("/dbnav/fahrplan", dbnavBody(from.id, to.id, manifest.anchor));
    const b = await journeys(
      "/dbnav/fahrplan",
      dbnavBody(from.id, VOCABULARY.stations[2].id, manifest.anchor),
    );
    expect(b).not.toEqual(a);
  });
});

describe("station suggest", () => {
  test("filters on the query bahn.de uses, and lists everything without one", async () => {
    const all = (await (await ask("/orte")).json()) as any[];
    expect(all).toHaveLength(VOCABULARY.stations.length);
    const hits = (await (await ask("/orte?suchbegriff=Ghent")).json()) as any[];
    expect(hits).toHaveLength(1);
    // `extId`, not `id`: the field transit's parse_suggest_response reads.
    expect(hits[0].extId).toBe(VOCABULARY.stations[0].id);
  });
});

describe("the geocoder places calls", () => {
  test("answers a demo city in the shape places parses", async () => {
    const res = await ask("/nominatim/search?q=Lisbon&format=jsonv2&limit=1");
    expect(res.status).toBe(200);
    const [hit] = (await res.json()) as any[];
    expect(Number(hit.lat)).toBeCloseTo(38.7223);
    expect(typeof hit.lon).toBe("string");
    expect(hit.address.country_code).toBe("pt");
    expect(hit.addresstype).toBe("city");
  });

  test("an unknown place is an empty list, which places reads as not found", async () => {
    const res = await ask("/nominatim/search?q=Atlantis");
    expect(await res.json()).toEqual([]);
  });
});

describe("the Gmail stand-in comms reads", () => {
  test("lists every demo thread on one inbox page", async () => {
    const page = (await (await ask("/gmail/threads?q=in:inbox&maxResults=100")).json()) as any;
    expect(page.threads.length).toBe(VOCABULARY.mail.length);
    expect(page.nextPageToken).toBeUndefined();
  });

  test("metadata carries From, Subject and Date, and an address on the reserved domain", async () => {
    const page = (await (await ask("/gmail/threads")).json()) as any;
    const thread = (await (await ask(`/gmail/threads/${page.threads[0].id}?format=metadata`)).json()) as any;
    const headers = Object.fromEntries(thread.messages[0].payload.headers.map((h: any) => [h.name, h.value]));
    expect(headers.From).toMatch(/<[^@]+@example\.org>$/);
    expect(headers.Subject).toBe(VOCABULARY.mail[0].subject);
    expect(thread.messages[0].payload.body).toBeUndefined();
  });

  test("an invitation's full body has the labels comms reads without a model", async () => {
    const index = VOCABULARY.mail.findIndex((m) => "invitation" in m);
    const page = (await (await ask("/gmail/threads")).json()) as any;
    const thread = (await (await ask(`/gmail/threads/${page.threads[index].id}?format=full`)).json()) as any;
    const text = Buffer.from(thread.messages[0].payload.body.data, "base64url").toString();
    expect(text).toContain("Event details");
    expect(text).toMatch(/^Date: [A-Z][a-z]+ \d{1,2}, \d{4}$/m);
    expect(text).toMatch(/^Time: \d\d:\d\d - \d\d:\d\d$/m);
    expect(text).toMatch(/^Location: .+, \d{4,6} [A-Z]/m);
  });

  test("an unknown thread is a Gmail-shaped 404", async () => {
    const res = await ask("/gmail/threads/ffffffff");
    expect(res.status).toBe(404);
  });
});
