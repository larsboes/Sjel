#!/usr/bin/env bun
// tools/demo-origin.ts — the synthetic remote the demo's last three capabilities read from.
//
// Comms, Scouting and Transit were the three the published demo could not show, and all three
// for the same reason: each one's job is to go and fetch something from somewhere else. A demo
// has nowhere else. The options were a hand-written fixture per capability — which breaks the
// manifest's one invariant, that every value shown was produced by a capability's own code —
// or giving them a somewhere else to fetch from. This is that somewhere.
//
// It is not a mock of a capability. It is a plain HTTP origin serving an RSS feed, some article
// pages, and bahn.de-shaped journey payloads, and the capabilities read it with exactly the code
// they use against the real internet. Comms' extractor really extracts, Scouting's rss adapter
// really parses, Transit's HAFAS parser really parses. What gets recorded is their output, not
// this file's input.
//
// Everything it serves descends from demo.toml's seed, so two runs of the same commit publish
// the same corpus and a rebuild diff shows code changes rather than dice.
//
//   tools/demo-origin           serve until killed
//   tools/demo-origin --once    print one of each payload and exit (what the tests read)

import { Rng, VOCABULARY, addDays, email } from "./lib/demo-data.ts";
import { loadManifest, type DemoManifest } from "./lib/demo-endpoints.ts";

// ─── Content ──────────────────────────────────────────────────────────────────

/** Absolute link to one article, built here rather than stored in the vocabulary: a literal
 *  URL in there is how a real host reaches a published page, and the sweep rejects one. */
const articleUrl = (base: string, slug: string) => `${base}/articles/${slug}`;

/** An article as a page a reader — or an extractor — sees.
 *
 *  Deliberately ordinary HTML with a `<h1>`, a meta description and paragraphs, because the
 *  thing being demonstrated is Comms' extraction. A page shaped to be easy to parse would
 *  record an extraction quality nothing else can reproduce. */
function articlePage(article: (typeof VOCABULARY.articles)[number], published: string): string {
  const paragraphs = [
    article.summary,
    article.body,
    "Written for a demonstration. Nothing here describes a real event, product or person.",
  ];
  return [
    "<!doctype html>",
    '<html lang="en">',
    "<head>",
    '<meta charset="utf-8">',
    `<title>${article.title}</title>`,
    `<meta name="description" content="${article.summary}">`,
    `<meta property="article:published_time" content="${published}">`,
    "</head>",
    "<body>",
    "<article>",
    `<h1>${article.title}</h1>`,
    `<p class="byline">Demo Origin · ${published}</p>`,
    ...paragraphs.map((p) => `<p>${p}</p>`),
    "</article>",
    "</body>",
    "</html>",
    "",
  ].join("\n");
}

/** RSS 2.0, which is the format Scouting's `rss` adapter takes and the one Comms would use if
 *  its collectors were pointed here. One feed serves both, so the two capabilities are reading
 *  the same synthetic publication rather than two that happen to agree. */
function feedXml(base: string, manifest: DemoManifest): string {
  const items = VOCABULARY.articles.map((article, index) => {
    const published = addDays(manifest.anchor, -(index * 3 + 1));
    const url = articleUrl(base, article.slug);
    const dated =
      article.kind === "opportunity"
        ? `<category>opportunity</category>\n   <dc:date>${addDays(manifest.anchor, article.offsetDays)}</dc:date>`
        : "<category>reading</category>";
    return [
      "  <item>",
      `   <title>${article.title}</title>`,
      `   <link>${url}</link>`,
      `   <guid isPermaLink="true">${url}</guid>`,
      `   <description>${article.summary}</description>`,
      `   <pubDate>${new Date(`${published}T08:00:00Z`).toUTCString()}</pubDate>`,
      `   ${dated}`,
      "  </item>",
    ].join("\n");
  });
  return [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<rss version="2.0" xmlns:dc="http://purl.org/dc/elements/1.1/">',
    " <channel>",
    "  <title>Demo Origin</title>",
    `  <link>${base}</link>`,
    "  <description>A synthetic publication, for a demonstration.</description>",
    ...items,
    " </channel>",
    "</rss>",
    "",
  ].join("\n");
}

// ─── Rail ─────────────────────────────────────────────────────────────────────

/** Two minutes past the hour, in whichever of the two shapes the backend expects.
 *
 *  dbweb takes a naive local string and dbnav an offset-carrying one — that difference is
 *  real, it is what `station_time` exists to reconcile, and flattening it here would let a
 *  timezone bug pass the demo and fail on the live endpoint. */
function stamp(date: string, minutes: number, offset: boolean): string {
  const hh = String(Math.floor(minutes / 60) % 24).padStart(2, "0");
  const mm = String(minutes % 60).padStart(2, "0");
  return `${date}T${hh}:${mm}:00${offset ? "+02:00" : ""}`;
}

interface PlannedLeg {
  service: (typeof VOCABULARY.services)[number];
  /** The train number: `28510` on a regional service, `1022` on an ICE. */
  number: string;
  /** What the train is announced as. A regional service is a LINE — `RE5`, `RB26` — whose
   *  number has nothing to do with `number`; a long-distance one is announced by its train
   *  number, `ICE 1022`. Getting this wrong in a stub would be quiet: `train_type_of` trims
   *  trailing digits either way, so the type stays right while the field stops resembling
   *  what bahn.de sends, and the next person to read a fixture learns the wrong shape. */
  label: string;
  from: (typeof VOCABULARY.stations)[number];
  to: (typeof VOCABULARY.stations)[number];
  departMinutes: number;
  arriveMinutes: number;
  platform: string;
}

interface PlannedJourney {
  id: string;
  legs: PlannedLeg[];
  priceCents: number;
}

/** Three journeys between two stations, deterministic in both.
 *
 *  One direct and two with a change, because a search that only ever returns direct trains
 *  demonstrates neither the transfer buffer nor the reliability product built on it. */
function planJourneys(
  manifest: DemoManifest,
  fromId: string,
  toId: string,
  date: string,
): PlannedJourney[] {
  const station = (id: string) =>
    VOCABULARY.stations.find((s) => s.id === id) ?? VOCABULARY.stations[0];
  const from = station(fromId);
  const to = station(toId);
  const via = VOCABULARY.stations.find((s) => s.id !== from.id && s.id !== to.id)!;

  const rng = new Rng(`${manifest.seed}:rail:${from.id}:${to.id}:${date}`);
  const journeys: PlannedJourney[] = [];

  for (let index = 0; index < 3; index++) {
    const depart = 9 * 60 + index * 47 + rng.int(0, 11);
    const direct = index === 0;
    const long = VOCABULARY.services.filter((s) => !s.regional);
    const regional = VOCABULARY.services.filter((s) => s.regional);

    if (direct) {
      const service = rng.pick(long);
      const number = String(rng.int(100, 999));
      journeys.push({
        id: `demo-${from.id}-${to.id}-${index}`,
        priceCents: rng.int(3_900, 12_900),
        legs: [
          {
            service,
            number,
            label: `${service.label} ${number}`,
            from,
            to,
            departMinutes: depart,
            arriveMinutes: depart + rng.int(160, 240),
            platform: String(rng.int(1, 12)),
          },
        ],
      });
      continue;
    }

    // The change is a regional leg into a long-distance one, which is the shape that made the
    // train-type bug visible in the first place: the arriving train at the transfer is the
    // regional one, and it is the term the reliability product needs.
    const first = rng.pick(regional);
    const second = rng.pick(long);
    const changeAt = depart + rng.int(38, 66);
    const buffer = rng.int(8, 21);
    const firstNumber = String(rng.int(10_000, 39_999));
    const secondNumber = String(rng.int(100, 999));
    journeys.push({
      id: `demo-${from.id}-${to.id}-${index}`,
      priceCents: rng.int(2_400, 9_800),
      legs: [
        {
          service: first,
          number: firstNumber,
          label: `${first.label}${rng.int(1, 89)}`,
          from,
          to: via,
          departMinutes: depart,
          arriveMinutes: changeAt,
          platform: String(rng.int(1, 9)),
        },
        {
          service: second,
          number: secondNumber,
          label: `${second.label} ${secondNumber}`,
          from: via,
          to,
          departMinutes: changeAt + buffer,
          arriveMinutes: changeAt + buffer + rng.int(120, 210),
          platform: String(rng.int(1, 12)),
        },
      ],
    });
  }
  return journeys;
}

/** dbnav's shape: `produktGattung`, `mitteltext`, offset-carrying stamps, `evaNr` on the stop.
 *  Copied from the capture `capabilities/transit/src/hafas.rs` parses in its own tests, so the
 *  parser meets the field names it was written against. */
function dbnavResponse(journeys: PlannedJourney[], date: string): unknown {
  const ort = (s: (typeof VOCABULARY.stations)[number]) => ({
    name: s.name,
    locationId: `A=1@O=${s.name}@L=${s.id}@`,
    evaNr: s.id,
    position: { latitude: s.latitude, longitude: s.longitude },
  });
  return {
    verbindungen: journeys.map((journey) => ({
      angebote: {
        preise: { gesamt: { ab: { betrag: journey.priceCents / 100, waehrung: "EUR" } } },
      },
      verbindung: {
        checksum: journey.id,
        reiseDauer:
          (journey.legs[journey.legs.length - 1].arriveMinutes - journey.legs[0].departMinutes) * 60,
        umstiegeAnzahl: journey.legs.length - 1,
        verbindungsAbschnitte: journey.legs.map((leg) => ({
          typ: "FAHRZEUG",
          abgangsDatum: stamp(date, leg.departMinutes, true),
          ankunftsDatum: stamp(date, leg.arriveMinutes, true),
          abschnittsDauer: (leg.arriveMinutes - leg.departMinutes) * 60,
          // The label, which is what transit reads the punctuality train type off.
          mitteltext: leg.label,
          kurztext: leg.service.label,
          produktGattung: leg.service.gattung,
          zugNummer: leg.number,
          verkehrsmittelNummer: leg.number,
          abgangsOrt: ort(leg.from),
          ankunftsOrt: ort(leg.to),
          halte: [
            { abgangsDatum: stamp(date, leg.departMinutes, true), gleis: leg.platform, ort: ort(leg.from) },
            { ankunftsDatum: stamp(date, leg.arriveMinutes, true), gleis: leg.platform, ort: ort(leg.to) },
          ],
        })),
      },
    })),
  };
}

/** dbweb's shape: `verkehrsmittel.kategorie`, naive stamps, `sollzeit`/`echtzeit` on the halt.
 *
 *  Regional trains are named by their bare number here, with no label — not a shortcut, that is
 *  what bahn.de actually returns, and it is the reason a dbweb regional leg cannot be scored.
 *  A stub that invented a label would hide the one thing this backend gets wrong. */
function dbwebResponse(journeys: PlannedJourney[], date: string): unknown {
  const halt = (
    s: (typeof VOCABULARY.stations)[number],
    minutes: number,
    event: "abfahrt" | "ankunft",
    platform: string,
  ) => ({
    id: `A=1@O=${s.name}@L=${s.id}@`,
    extId: s.id,
    name: s.name,
    gleis: platform,
    [event]: { sollzeit: stamp(date, minutes, false) },
  });
  return {
    verbindungen: journeys.map((journey) => ({
      tripId: journey.id,
      angebote: {
        preise: { gesamt: { ab: { betrag: journey.priceCents / 100, waehrung: "EUR" } } },
      },
      verbindungsAbschnitte: journey.legs.map((leg) => ({
        verkehrsmittel: {
          // The regional case is the bare number, with the label nowhere in the response.
          // That is what bahn.de actually returns, and it is the reason a dbweb regional leg
          // cannot be scored at all — a stub that helpfully supplied `leg.label` here would
          // hide the one thing this backend gets wrong.
          name: leg.service.regional ? leg.number : leg.label,
          nummer: leg.number,
          kategorie: leg.service.regional ? "DRB" : leg.service.label,
          zugattribute: leg.service.regional ? [{ key: "9G", value: "Deutschlandticket" }] : [],
        },
        halte: [
          halt(leg.from, leg.departMinutes, "abfahrt", leg.platform),
          halt(leg.to, leg.arriveMinutes, "ankunft", leg.platform),
        ],
      })),
    })),
  };
}

/** The suggest endpoint's shape: `extId`, `name`, `lat`, `lon`. */
function orteResponse(query: string): unknown {
  const needle = query.trim().toLowerCase();
  const hits = needle
    ? VOCABULARY.stations.filter((s) => s.name.toLowerCase().includes(needle))
    : VOCABULARY.stations;
  return hits.map((s) => ({
    extId: s.id,
    name: s.name,
    lat: s.latitude,
    lon: s.longitude,
  }));
}

/** Nominatim's `/search?format=jsonv2` shape, for the demo cities only. Places reads `lat` and
 *  `lon` as strings, `display_name`, `name`, `address.country_code` and `addresstype`
 *  (capabilities/places/src/geocode.rs). The osm ids are a synthetic block. */
export function nominatimResponse(query: string): unknown[] {
  const needle = query.split(",")[0].trim().toLowerCase();
  const hits = VOCABULARY.cities.filter((c) => needle !== "" && c.name.toLowerCase() === needle);
  return hits.map((c) => ({
    lat: String(c.latitude),
    lon: String(c.longitude),
    name: c.name,
    display_name: `${c.name}, ${c.region}`,
    addresstype: "city",
    osm_type: "node",
    osm_id: 990_000 + VOCABULARY.cities.indexOf(c),
    address: { city: c.name, country: c.region, country_code: c.countryCode },
  }));
}

// ─── Gmail ────────────────────────────────────────────────────────────────────
//
// The Gmail API v1 shapes comms reads (capabilities/comms/src/google.rs): the inbox page, one
// thread as metadata or full, labels, and the three thread actions. comms calls these through
// SJEL_COMMS_GMAIL_URL, set by tools/demo-up, and sends a placeholder token while it is set.

type Mail = (typeof VOCABULARY.mail)[number];

const MONTH_NAMES = ["January", "February", "March", "April", "May", "June", "July", "August",
  "September", "October", "November", "December"];

/** The thread id for the mail at `index`: hex, like Gmail's. */
export const mailId = (index: number) => `d3m0${index.toString(16).padStart(4, "0")}`;

/** The day an invitation's event falls on, relative to the anchor. */
function invitationDate(mail: Mail, anchor: string): string | null {
  if (!("invitation" in mail)) return null;
  const inv = mail.invitation as { tripCity?: string; offsetDays?: number };
  const trip = VOCABULARY.trips.find((t) => t.city === inv.tripCity);
  return addDays(anchor, trip ? trip.offsetDays + 1 : (inv.offsetDays ?? 0));
}

function mailText(mail: Mail, anchor: string): string {
  const lines = ["Hello,", "", mail.snippet];
  const date = invitationDate(mail, anchor);
  if (date && "invitation" in mail) {
    const [y, m, d] = date.split("-").map(Number);
    lines.push("", "Event details", `Date: ${MONTH_NAMES[m - 1]} ${d}, ${y}`,
      `Time: ${mail.invitation.time}`, `Location: ${mail.invitation.location}`);
  }
  return lines.concat("", `${mail.sender}`).join("\n");
}

function mailMessage(index: number, anchor: string, full: boolean): unknown {
  const mail = VOCABULARY.mail[index];
  const sent = new Date(`${addDays(anchor, -mail.daysAgo)}T09:${String(10 + index).padStart(2, "0")}:00Z`);
  const local = "noreply" in mail && mail.noreply ? "noreply@example.org" : email(mail.sender);
  const headers = [
    { name: "From", value: `${mail.sender} <${local}>` },
    { name: "Subject", value: mail.subject },
    { name: "Date", value: sent.toUTCString().replace("GMT", "+0000") },
  ];
  if ("unsubscribe" in mail && mail.unsubscribe) {
    headers.push({ name: "List-Unsubscribe", value: "<mailto:unsubscribe@example.org>" });
  }
  const payload: Record<string, unknown> = { headers };
  if (full) {
    payload.mimeType = "text/plain";
    payload.body = { data: Buffer.from(mailText(mail, anchor)).toString("base64url") };
  }
  return {
    id: `${mailId(index)}m`,
    threadId: mailId(index),
    labelIds: ["INBOX", ...(mail.daysAgo === 0 ? ["UNREAD"] : [])],
    snippet: mail.snippet,
    internalDate: String(sent.getTime()),
    payload,
  };
}

function gmail(url: URL, method: string, anchor: string): Response | null {
  const path = url.pathname.slice("/gmail".length);
  if (method === "GET" && path === "/threads") {
    return Response.json({
      threads: VOCABULARY.mail.map((m, i) => ({ id: mailId(i), snippet: m.snippet })),
      resultSizeEstimate: VOCABULARY.mail.length,
    });
  }
  const thread = /^\/threads\/([0-9a-z]+)(\/(modify|trash|untrash))?$/.exec(path);
  if (thread) {
    const index = VOCABULARY.mail.findIndex((_, i) => mailId(i) === thread[1]);
    if (index < 0) return Response.json({ error: { code: 404, message: "Not Found" } }, { status: 404 });
    if (thread[3] && method === "POST") return Response.json({ id: thread[1] });
    const full = url.searchParams.get("format") === "full";
    return Response.json({ id: thread[1], messages: [mailMessage(index, anchor, full)] });
  }
  if (path === "/labels") {
    if (method === "POST") return Response.json({ id: "Label_1", name: "Waiting" });
    return Response.json({ labels: [{ id: "INBOX", name: "INBOX" }, { id: "Label_1", name: "Waiting" }] });
  }
  return null;
}

// ─── Request routing ──────────────────────────────────────────────────────────

/** The EVA out of either request shape. dbweb sends it bare; dbnav wraps it in the short lid
 *  form `A=1@L=<eva>@`, which is what the canonical client sends and what transit copies. */
function evaOf(value: unknown): string {
  const text = typeof value === "string" ? value : "";
  const lid = text.split("@").find((part) => part.startsWith("L="));
  return (lid ? lid.slice(2) : text) || VOCABULARY.stations[0].id;
}

/** The travel date out of either request shape, as YYYY-MM-DD. */
const dateOf = (value: unknown) =>
  typeof value === "string" && value.length >= 10 ? value.slice(0, 10) : "";

export async function handle(
  request: Request,
  manifest: DemoManifest,
  base: string,
): Promise<Response> {
  const url = new URL(request.url);
  const json = (body: unknown) => Response.json(body);

  if (url.pathname === "/health") return new Response("ok\n");

  if (url.pathname === "/feed.xml") {
    return new Response(feedXml(base, manifest), {
      headers: { "Content-Type": "application/rss+xml; charset=utf-8" },
    });
  }

  if (url.pathname.startsWith("/articles/")) {
    const slug = url.pathname.slice("/articles/".length);
    const article = VOCABULARY.articles.find((a) => a.slug === slug);
    if (!article) return new Response("no such article\n", { status: 404 });
    const index = VOCABULARY.articles.indexOf(article);
    return new Response(articlePage(article, addDays(manifest.anchor, -(index * 3 + 1))), {
      headers: { "Content-Type": "text/html; charset=utf-8" },
    });
  }

  if (url.pathname === "/orte") {
    // bahn.de takes the query under `suchbegriff`; anything else lists everything, which is
    // what makes a bare probe of this origin readable.
    return json(orteResponse(url.searchParams.get("suchbegriff") ?? ""));
  }

  if (request.method === "POST" && (url.pathname === "/fahrplan" || url.pathname === "/dbnav/fahrplan")) {
    const body = (await request.json().catch(() => ({}))) as Record<string, any>;
    const dbnav = url.pathname === "/dbnav/fahrplan";
    const wish = body?.reiseHin?.wunsch ?? {};
    const from = evaOf(dbnav ? wish.abgangsLocationId : body.abfahrtsHalt);
    const to = evaOf(dbnav ? wish.zielLocationId : body.ankunftsHalt);
    const date =
      dateOf(dbnav ? wish.zeitWunsch?.reiseDatum : body.anfrageZeitpunkt) || manifest.anchor;
    const journeys = planJourneys(manifest, from, to, date);
    return json(dbnav ? dbnavResponse(journeys, date) : dbwebResponse(journeys, date));
  }

  if (url.pathname.startsWith("/gmail/")) {
    const answer = gmail(url, request.method, manifest.anchor);
    if (answer) return answer;
  }

  if (url.pathname === "/nominatim/search") {
    return json(nominatimResponse(url.searchParams.get("q") ?? url.searchParams.get("city") ?? ""));
  }

  return new Response("demo-origin serves /feed.xml, /articles/<slug>, /orte, /fahrplan, /dbnav/fahrplan, /nominatim/search, /gmail/…\n", {
    status: 404,
  });
}

// ─── Main ─────────────────────────────────────────────────────────────────────

function main(): void {
  const manifest = loadManifest();
  const { port } = new URL(manifest.origin);
  const base = manifest.origin.replace(/\/$/, "");

  if (process.argv.includes("--once")) {
    // One of each, for eyeballing and for the test that asserts the shapes are the ones the
    // parsers were written against.
    const journeys = planJourneys(manifest, VOCABULARY.stations[0].id, VOCABULARY.stations[1].id, manifest.anchor);
    console.log(JSON.stringify({
      feed: feedXml(base, manifest),
      article: articlePage(VOCABULARY.articles[0], manifest.anchor),
      orte: orteResponse(""),
      dbnav: dbnavResponse(journeys, manifest.anchor),
      dbweb: dbwebResponse(journeys, manifest.anchor),
    }, null, 2));
    return;
  }

  Bun.serve({
    port: Number(port),
    fetch: (request) => handle(request, manifest, base),
  });
  console.log(`demo-origin: serving ${base} (${VOCABULARY.articles.length} articles, ${VOCABULARY.stations.length} stations)`);
}

if (import.meta.main) main();
