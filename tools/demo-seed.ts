#!/usr/bin/env bun
// tools/demo-seed.ts — fill running capabilities with fictional data, through their own APIs (#168).
//
// No capability crate contains a line of demo code. Everything below is a POST to a route
// the dashboard also calls, which buys three things: the seeders cannot reach past a public
// contract into a store, a seeding run is an end-to-end exercise of the write paths, and a
// capability's demo data goes stale the same way its API does — loudly, with a 404.
//
// THE GUARD, and why it is not optional. These are real writes to whatever database the
// running capabilities are pointed at. Run this against a machine's own Sjel and it would
// post invented transactions into a real ledger. So it refuses to start unless the resolved
// overlay is this repository's demo overlay, and refuses each capability that already holds
// data. Both checks are cheap; the failure they prevent is not recoverable by re-running.
//
//   tools/demo-seed              seed every capability with a `writes` in demo.toml
//   tools/demo-seed --only trips seed one
//   tools/demo-seed --force      skip the not-empty refusal (never the overlay check)

import { execFileSync } from "node:child_process";
import { join } from "node:path";

import {
  addDays,
  at,
  daysBetween,
  eventWindow,
  monthStart,
  Rng,
  VOCABULARY,
} from "./lib/demo-data.ts";
import {
  SJEL_ROOT,
  loadManifest,
  resolvePath,
  routes,
  type DemoManifest,
} from "./lib/demo-endpoints.ts";

const DEMO_OVERLAY = join(SJEL_ROOT, "demo/overlay");

interface Ctx {
  rng: Rng;
  anchor: string;
  manifest: DemoManifest;
  /** Resolve a browser path to a live URL, e.g. `/trips/api/plans`. */
  url: (browserPath: string) => string;
}

// ─── HTTP ─────────────────────────────────────────────────────────────────────

/** Comms disables every mutating route unless a shared secret is configured, so /ingest is a
 *  403 without this header rather than an open write path. tools/demo-up generates the token
 *  per run and exports it; sent on every request because the capabilities that do not want it
 *  ignore an unknown header, and one conditional per seeder would be the thing somebody
 *  forgets. */
function authHeaders(): Record<string, string> {
  const token = process.env.SJEL_DEMO_COMMS_TOKEN;
  return token ? { "X-Sjel-Token": token } : {};
}

async function send<T>(method: string, url: string, body?: unknown): Promise<T> {
  const res = await fetch(url, {
    method,
    headers: {
      ...authHeaders(),
      ...(body === undefined ? {} : { "Content-Type": "application/json" }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  if (!res.ok) {
    throw new Error(`${method} ${url} → ${res.status}: ${text.slice(0, 400)}`);
  }
  return (text ? JSON.parse(text) : undefined) as T;
}

const get = <T>(url: string) => send<T>("GET", url);
const post = <T>(url: string, body?: unknown) => send<T>("POST", url, body);
const patch = <T>(url: string, body: unknown) => send<T>("PATCH", url, body);

/** Poll a capability until it answers, so a seeder never races a server that is still
 *  opening its database. Bounded: a capability that is not coming up is a failure to
 *  report, not something to wait out. */
async function waitFor(url: string, seconds = 60): Promise<void> {
  const deadline = Date.now() + seconds * 1000;
  let last = "";
  while (Date.now() < deadline) {
    try {
      const res = await fetch(url);
      // Ready means the server answers, not that this probe is authorized: since B6 every
      // non-exempt route answers 401 to a bare GET, and the probe deliberately sends no token
      // (a token is proof of a caller, not of a listener). Found 2026-09-02 when the Pages
      // build reached this line for the first time since 2026-08-28.
      if (res.ok || res.status === 401 || res.status === 403) return;
      last = `${res.status}`;
    } catch (err) {
      last = err instanceof Error ? err.message : String(err);
    }
    await Bun.sleep(500);
  }
  throw new Error(`demo-seed: ${url} never became ready (${seconds}s; last: ${last})`);
}

// ─── Seeders ──────────────────────────────────────────────────────────────────
//
// One function per `writes` value in demo.toml. Each returns a short line for the log —
// what it created, so a CI run says what the published demo actually contains.

const SEEDERS: Record<string, (ctx: Ctx) => Promise<string>> = {
  async calendar(ctx) {
    const entries = ctx.url("/calendar/api/entries");
    const rhythms = ctx.url("/calendar/api/rhythms");
    const contexts = ctx.url("/calendar/api/contexts");

    // Rhythms first: each materializes its own instances, and an entry posted afterwards
    // can then legitimately collide with one — which is what makes the feasibility view
    // show a conflict rather than an empty grid.
    for (const r of VOCABULARY.rhythms) {
      await post(rhythms, {
        kind: r.kind,
        title: r.title,
        location: VOCABULARY.home.name,
        byweekday: [...r.byweekday],
        start_time: r.start,
        end_time: r.end,
        valid_from: addDays(ctx.anchor, -28),
        valid_until: addDays(ctx.anchor, 45),
      });
    }

    let count = 0;
    for (let day = -26; day <= 44; day++) {
      if (!ctx.rng.bool(0.42)) continue;
      const event = ctx.rng.pick(VOCABULARY.events);
      const date = addDays(ctx.anchor, day);
      const window = eventWindow(ctx.rng, date, event.hours);
      await post(entries, {
        kind: event.kind,
        // Past entries are settled, near ones are planned, far ones are still ideas. A
        // calendar where everything is equally certain cannot demonstrate a tool whose
        // whole model is that they are not.
        commitment: day < 0 ? "committed" : day < 14 ? "planned" : "possible",
        title: event.title,
        starts_at: window.starts_at,
        ends_at: window.ends_at,
        all_day: false,
        location: ctx.rng.bool(0.6) ? VOCABULARY.home.name : null,
        notes: null,
        source: "manual",
      });
      count++;
    }

    for (const trip of VOCABULARY.trips.filter((t) => t.offsetDays > 0)) {
      await post(contexts, {
        kind: "travel",
        title: `Away — ${trip.city}`,
        details: "Reachable, but not for anything that needs the desk.",
        valid_from: addDays(ctx.anchor, trip.offsetDays),
        valid_until: addDays(ctx.anchor, trip.offsetDays + trip.nights),
      });
    }
    return `${VOCABULARY.rhythms.length} rhythms, ${count} entries, ${VOCABULARY.trips.filter((t) => t.offsetDays > 0).length} contexts`;
  },

  // Comms and Scouting are the two capabilities whose job is to go and fetch something. They
  // are seeded by pointing them at tools/demo-origin and letting them do exactly that: neither
  // seeder writes an item, and neither knows what an item looks like. What lands in the store
  // is the output of Comms' own extractor and Scouting's own rss adapter, which is the only
  // reason these two can appear in a demo that forbids hand-written fixtures.

  async comms(ctx) {
    const ingest = ctx.url("/comms/ingest");
    const status = (id: string) => ctx.url(`/comms/feed/${id}/status`);
    const ingested: string[] = [];

    for (const article of VOCABULARY.articles) {
      // No `content`, deliberately. Passing the body would make this a fixture with extra
      // steps; omitting it makes the server fetch and extract the page itself, which is the
      // behaviour worth demonstrating and the one that breaks loudly when extraction does.
      // It is also why demo-up has to name this origin in `ingest_allowed_origins`: Comms
      // refuses to fetch a loopback address unless the deployment wrote that permission down.
      const item = await post<{ item?: { id: string } ; id?: string }>(ingest, {
        url: `${ctx.manifest.origin}/articles/${article.slug}`,
        client: "demo-seed",
      });
      const id = item.item?.id ?? item.id;
      if (id) ingested.push(id);
    }

    // A feed where every row is `new` cannot demonstrate triage, which is the screen that
    // exists to move rows out of it. Two decisions, taken deterministically rather than by
    // dice, so the demo's triage counts are the same on every build.
    let keepers = 0;
    let dismissed = 0;
    for (const [index, id] of ingested.entries()) {
      if (index % 3 === 0) {
        await post(status(id), { status: "keeper" });
        keepers++;
      } else if (index % 5 === 4) {
        await post(status(id), { status: "dismissed" });
        dismissed++;
      }
    }
    // Review lists what the quality pass flags, so it runs once over what just arrived.
    await post(ctx.url("/comms/feed/quality/refresh"), {});

    // The Inbox: things saved for later, through the route `sjel capture` and the share sheet use.
    for (const capture of VOCABULARY.captures) {
      await post(ctx.url("/comms/captures"), {
        text: capture.text,
        url: "article" in capture ? `${ctx.manifest.origin}/articles/${capture.article}` : undefined,
      });
    }
    // Mail: one sweep over the Gmail stand-in (tools/demo-origin, SJEL_COMMS_GMAIL_URL). The rules
    // classify each thread, and the invitations become Calendar proposals without a model.
    const sweep = await post<{ fetched?: number; events?: { proposals?: number } }>(
      ctx.url("/comms/triage/sweep"),
      { limit: 100 },
    );
    if (!sweep.fetched) {
      throw new Error("the mail sweep fetched nothing. Is SJEL_COMMS_GMAIL_URL set by tools/demo-up?");
    }
    return `${ingested.length} items ingested from the demo origin (${keepers} kept, ${dismissed} dismissed), ` +
      `${VOCABULARY.captures.length} captures, ${sweep.fetched} mails (${sweep.events?.proposals ?? 0} event proposals)`;
  },

  async scouting(ctx) {
    // One GET, and it does the work: /discover runs the declared source's adapter, scores
    // what it finds and persists it. The source is `demo-origin`, declared in the generated
    // scouting.json — tools/demo-up writes that from demo.toml's origin, so the id here and
    // the URL there cannot drift apart into a scan of nothing.
    const found = await get<{ results?: unknown[]; total_scored?: number; store_total?: number }>(
      ctx.url("/discover?adapter=demo-origin&limit=20"),
    );
    // `results` and `total_scored` are what /discover answers with; `store_total` is what it
    // persisted. Reading the wrong name here failed as "discovered nothing", which is exactly
    // the message the check below prints — so the guard fired correctly on a seeder bug rather
    // than an empty scan, and cost a run to tell apart. Both counted now.
    const rows = found.results?.length ?? found.total_scored ?? 0;
    if (rows === 0) {
      throw new Error(
        "scouting discovered nothing from the demo origin. Either the origin is not serving " +
          "/feed.xml, or the generated demo/overlay/config/scouting.json did not reach the " +
          "capability — a silently empty Scout page is the thing this check exists to prevent.",
      );
    }
    return `${rows} opportunities discovered through the rss adapter (${found.store_total ?? rows} persisted)`;
  },

  async trips(ctx) {
    const url = ctx.url("/trips/api/plans");
    const home = VOCABULARY.home;
    const place = (c: (typeof VOCABULARY.cities)[number]) => ({
      id: `demo-${c.name.toLowerCase().replace(/[^a-z]/g, "")}`,
      name: c.name,
      kind: "city" as const,
      address: null,
      latitude: c.latitude,
      longitude: c.longitude,
    });

    let itemCount = 0;
    for (const trip of VOCABULARY.trips) {
      const city = VOCABULARY.cities.find((c) => c.name === trip.city)!;
      const start = addDays(ctx.anchor, trip.offsetDays);
      const plan = await post<{ id: string }>(url, {
        title: trip.title,
        origin: place(home),
        destinations: [place(city)],
        date_start: start,
        date_end: addDays(start, trip.nights),
        interests: "Walking the city, one museum, somewhere to work for a morning.",
        travelers: ctx.rng.sample(VOCABULARY.people, ctx.rng.int(1, 2)),
        transport_modes: ["train", "walk"],
      });
      // A day-by-day itinerary: the train out and back, the stay, a museum, and a meetup with
      // whoever lives there. The trains are what the demo's own transit answered for that day
      // (tools/demo-origin's timetable), held in the plan the way the app holds a connection.
      const station = (name: string) => VOCABULARY.stations.find((s) => s.name === `${name} Central`)!;
      const end = addDays(start, trip.nights);
      let items = 0;
      const item = async (body: Record<string, unknown>) => {
        await post(`${url}/${plan.id}/items`, body);
        items++;
      };
      for (const [day, from, to] of [
        [start, home.name, city.name],
        [end, city.name, home.name],
      ] as const) {
        const found = await get<{ journeys?: unknown[] } | unknown[]>(
          ctx.url(`/api/search?from=${station(from).id}&to=${station(to).id}&time=${day}T08:00:00`),
        );
        const journeys = Array.isArray(found) ? found : (found.journeys ?? []);
        if (journeys.length === 0) throw new Error(`transit found no train ${from} → ${to} on ${day}`);
        const journey = journeys[0] as { id?: string };
        await item({
          item_type: "transport",
          day,
          external_id: `demo-train-${plan.id}-${day}`,
          title: `Train ${from} → ${to}`,
          payload: { mode: "train", journey },
        });
        if (trip.offsetDays < 0 || day === start) {
          await item({
            item_type: "booking",
            day,
            external_id: `demo-booking-${plan.id}-${day}`,
            title: `Ticket ${from} → ${to}`,
            payload: { provider: "Regiobahn Tickets", order_ref: `DEMO-${ctx.rng.int(100000, 999999)}`, refundable: false },
          });
        }
      }
      const stay = VOCABULARY.pois.find((p) => p.city === city.name && p.category === "stay");
      await item({
        item_type: "stay",
        day: start,
        external_id: `demo-stay-${plan.id}`,
        title: stay?.name ?? `Apartment near the centre, ${city.name}`,
        // The declared stay shape: where and when are required, the rest rides along.
        payload: {
          check_in: start,
          check_out: end,
          latitude: city.latitude + (stay?.dLat ?? 0),
          longitude: city.longitude + (stay?.dLon ?? 0),
          currency: "EUR",
          price_per_night: ctx.rng.int(70, 140),
        },
      });
      for (const poi of VOCABULARY.pois.filter((p) => p.city === city.name && p.category !== "stay")) {
        await item({
          item_type: "place",
          day: addDays(start, 1),
          external_id: `demo-place-${plan.id}-${poi.name}`,
          title: poi.name,
          payload: { category: poi.category, latitude: city.latitude + poi.dLat, longitude: city.longitude + poi.dLon },
        });
      }
      // A meetup is an activity whose payload names people in `with` (trips store.rs).
      const local = VOCABULARY.contacts.find((c) => c.homeBase === city.name);
      if (local) {
        await item({
          item_type: "activity",
          day: addDays(start, Math.min(2, trip.nights - 1)),
          external_id: `demo-meetup-${plan.id}`,
          title: `Dinner with ${local.name.split(" ")[0]}`,
          payload: { with: [{ person: local.name }], status: trip.offsetDays < 0 ? "done" : "asked" },
        });
      }
      await item({
        item_type: "note",
        day: addDays(start, 1),
        external_id: `demo-note-${plan.id}`,
        title: "Museum is closed on Mondays",
        payload: {},
      });
      itemCount += items;
    }
    return `${VOCABULARY.trips.length} plans, ${itemCount} itinerary items`;
  },

  // Pins through the same route the app's place form posts to, with coordinates, so nothing is
  // geocoded. Companions go in the way a person states them: home cities through the geocoder
  // (answered by tools/demo-origin), confirmed. An absence stays proposed, so the review has a
  // decision waiting.
  async places(ctx) {
    const create = ctx.url("/places/api/places");
    for (const poi of VOCABULARY.pois) {
      const city = VOCABULARY.cities.find((c) => c.name === poi.city)!;
      await post(create, {
        name: poi.name,
        kind: "venue",
        note: poi.category,
        city: city.name,
        country_code: city.countryCode,
        latitude: Number((city.latitude + poi.dLat).toFixed(5)),
        longitude: Number((city.longitude + poi.dLon).toFixed(5)),
      });
    }

    const stated = ctx.url("/places/api/people/places");
    const confirm = (id: string) =>
      ctx.url(`/places/api/people/proposals/${encodeURIComponent(id)}/confirm`);
    let confirmed = 0;
    let proposed = 0;
    for (const c of VOCABULARY.contacts) {
      const home = await post<{ id: string }>(stated, { person: c.name, city: c.homeBase });
      await post(confirm(home.id));
      confirmed++;
      if ("away" in c) {
        await post(stated, {
          person: c.name,
          city: c.away.city,
          from: addDays(ctx.anchor, c.away.fromOffset),
          to: addDays(ctx.anchor, c.away.toOffset),
        });
        proposed++;
      }
    }
    return `${VOCABULARY.pois.length} places, ${confirmed} confirmed homes, ${proposed} proposed absences`;
  },

  async finance(ctx) {
    const base = ctx.url("/finance/api");

    // 1. A bank export, the way somebody actually starts: preview, then import, then
    //    review. Semicolons and comma decimals on purpose — that is the European CSV this
    //    importer exists for, and a demo built on the easy dialect would not show it working.
    const { csv, rows } = bankCsv(ctx);
    const mapping = bankMapping();
    const preview = await post<{ preview_id: string; candidate_count: number }>(
      `${base}/import/csv/preview`,
      { content: csv, mapping },
    );
    await post(`${base}/import/csv`, {
      content: csv,
      mapping,
      expected_preview_id: preview.preview_id,
    });

    // 2. Confirm most of them, at the account the importer proposed, and deliberately
    //    leave a handful pending — the Import Review screen is part of the product, and a
    //    demo with an empty review queue hides it.
    const candidates = await get<Array<{ id: string; proposed_account: string; state: string }>>(
      `${base}/import/candidates`,
    );
    const pending = candidates.filter((c) => c.state === "pending");
    const confirm = pending.slice(0, Math.max(0, pending.length - 6));
    if (confirm.length > 0) {
      await post(`${base}/import/candidates/confirm-batch`, {
        items: confirm.map((c) => ({ id: c.id, account: c.proposed_account })),
      });
    }

    // 3. Holdings, from a broker export in the same shape.
    const investments = investmentCsv(ctx);
    const invMapping = investmentMapping();
    const invPreview = await post<{ snapshot_id: string }>(
      `${base}/import/investments/preview`,
      { content: investments, mapping: invMapping },
    );
    await post(`${base}/import/investments/confirm`, {
      content: investments,
      mapping: invMapping,
      source_key: "demo-broker",
      expected_snapshot_id: invPreview.snapshot_id,
      coverage: "complete",
    });

    // 4. What a bank export cannot know: cash outside the account, and a loan.
    await post(`${base}/balance-snapshot`, {
      as_of: ctx.anchor,
      currency: "EUR",
      coverage: "complete",
      balances: VOCABULARY.balances.map((b, i) => ({
        id: `demo-balance-${i}`,
        label: b.label,
        kind: b.kind,
        amount_cents: b.cents,
      })),
    });

    // 5. Subscriptions, seeded from notes this writes into `projects` first, where finance
    //    reads them. Generated rather than committed: they are demo data like everything
    //    else, and a tracked markdown file that looks like somebody's real note is exactly
    //    the confusion to avoid.
    await writeSubscriptionNotes(ctx.url("/projects/api/record"));
    const imported = await post<{ created: number }>(`${base}/import/obsidian`);

    // 6. Proposals. No price fetch first, and none is needed: with no market
    //    observation a position is valued at the reviewed activity price the step
    //    above just confirmed, which is what makes the recorded demo reproducible
    //    offline and independent of any provider answering. The run turns the
    //    generated targets into a non-empty decisions inbox, which is the whole
    //    point of recording this surface.
    const proposals = await post<{ proposed: number }>(`${base}/decisions/run`, {});

    return (
      `${rows} bank rows → ${preview.candidate_count} candidates ` +
      `(${confirm.length} confirmed, ${pending.length - confirm.length} left for review), ` +
      `${VOCABULARY.instruments.length} holdings, ${VOCABULARY.balances.length} balances, ` +
      `${imported.created} subscriptions, ${proposals.proposed} investment proposals`
    );
  },
  // People go in the way the People page adds them: POST an entity with its field values,
  // then one fact per place. Places is not in the demo, so each fact is stored without a
  // coordinate and says so (`geocode.status = "unavailable"`); the list and Home still work.
  async entities(ctx) {
    const create = ctx.url("/entities/api/entities");
    const facts = (id: string) => ctx.url(`/entities/api/entities/${encodeURIComponent(id)}/facts`);
    let factCount = 0;
    for (const c of VOCABULARY.contacts) {
      const birthday = `1991${addDays(ctx.anchor, c.birthdayOffset).slice(4)}`;
      const person = await post<{ id: string }>(create, {
        kind: "person",
        name: c.name,
        values: { relation: c.relation, sleeping_option: c.sleeping, interests: [...c.interests], birthday },
      });
      await post(facts(person.id), { predicate: "home_base", place: c.homeBase });
      factCount++;
      if ("away" in c && c.away) {
        await post(facts(person.id), {
          predicate: "away",
          place: c.away.city,
          valid_from: addDays(ctx.anchor, c.away.fromOffset),
          valid_to: addDays(ctx.anchor, c.away.toOffset),
        });
        factCount++;
      }
    }
    // The demo's own person, so Calendar knows where home is and never suggests a trip there
    // (calendar server.rs, home_place reads entities' /api/home).
    const self = await post<{ id: string }>(create, { kind: "self", name: "Demo Owner", values: {} });
    await post(facts(self.id), { predicate: "home_base", place: VOCABULARY.home.name });
    factCount++;
    return `${VOCABULARY.contacts.length} people and the owner, ${factCount} place facts`;
  },
};

// ─── Finance inputs ───────────────────────────────────────────────────────────

/** Fourteen months of transactions. Long enough that the trend chart has a trend and the
 *  month-over-month view has something to compare; short enough to import in seconds. */
function bankCsv(ctx: Ctx): { csv: string; rows: number } {
  const lines = ["date;description;amount;currency;reference;category"];
  const cents = (n: number) => (n / 100).toFixed(2).replace(".", ",");

  for (let monthsBack = 13; monthsBack >= 0; monthsBack--) {
    const first = monthStart(ctx.anchor, monthsBack);
    const span = monthsBack === 0 ? daysBetween(first, ctx.anchor) : 27;

    for (const income of VOCABULARY.income) {
      // Freelance work is not every month. Salary is.
      if (income.account.endsWith("freelance") && !ctx.rng.bool(0.35)) continue;
      const day = addDays(first, income.account.endsWith("salary") ? 0 : ctx.rng.int(4, 20));
      if (daysBetween(first, day) > span) continue;
      lines.push(
        [
          german(day),
          income.description,
          cents(income.cents + ctx.rng.int(-4_000, 4_000)),
          "EUR",
          "",
          "income",
        ].join(";"),
      );
    }

    for (let i = 0; i < ctx.rng.int(14, 26); i++) {
      const offset = ctx.rng.int(0, span);
      const merchant = ctx.rng.pick(VOCABULARY.merchants);
      lines.push(
        [
          german(addDays(first, offset)),
          merchant.name,
          `-${cents(ctx.rng.amountCents(merchant.min, merchant.max))}`,
          "EUR",
          "",
          merchant.account.split(":")[1] ?? "",
        ].join(";"),
      );
    }
  }
  return { csv: `${lines.join("\n")}\n`, rows: lines.length - 1 };
}

const german = (iso: string): string => {
  const [y, m, d] = iso.split("-");
  return `${d}.${m}.${y}`;
};

function bankMapping() {
  return {
    delimiter: ";",
    decimal_separator: ",",
    date_column: "date",
    amount_column: "amount",
    description_column: "description",
    categorization_columns: ["category"],
    reference_column: "reference",
    currency_column: "currency",
    default_currency: "EUR",
    source_account: "assets:bank:everyday",
    default_outflow_account: "expenses:uncategorized",
    default_inflow_account: "income:uncategorized",
    // One rule per invented merchant. Written from the same vocabulary the CSV is written
    // from, so the demo's categorization rate is high but not perfect — `expenses:uncategorized`
    // still catches the rows no rule names, which is the honest picture of a real import.
    categorization_rules: VOCABULARY.merchants.map((m) => ({
      description_contains_any: [m.name],
      description_starts_with_any: [],
      direction: "outflow" as const,
      account: m.account,
      confidence_basis_points: 9_200,
    })).concat(
      VOCABULARY.income.map((i) => ({
        description_contains_any: [i.description],
        description_starts_with_any: [],
        direction: "inflow" as const,
        account: i.account,
        confidence_basis_points: 9_800,
      })),
    ),
    row_filter: null,
    amount_sign: "as_provided" as const,
    amount_rounding: "half_away_from_zero" as const,
    date_formats: ["day_month_year_dots" as const],
    row_policy: "required_fields" as const,
  };
}

function investmentCsv(ctx: Ctx): string {
  const lines = ["date;instrument;quantity;price;currency;activity"];
  for (const instrument of VOCABULARY.instruments) {
    // A position built over several buys, which is what makes the holdings view show an
    // aggregate rather than echoing one row back.
    for (let i = 0; i < ctx.rng.int(3, 6); i++) {
      lines.push(
        [
          german(addDays(ctx.anchor, -ctx.rng.int(30, 900))),
          instrument.symbol,
          String(ctx.rng.int(1, 12)),
          (instrument.unitCents / 100).toFixed(2).replace(".", ","),
          "EUR",
          "BUY",
        ].join(";"),
      );
    }
    lines.push(
      [german(addDays(ctx.anchor, -ctx.rng.int(10, 120))), instrument.symbol, "0", "0,00", "EUR", "DIVIDEND"].join(";"),
    );
  }
  return `${lines.join("\n")}\n`;
}

function investmentMapping() {
  return {
    delimiter: ";",
    decimal_separator: ",",
    date_column: "date",
    instrument_column: "instrument",
    quantity_column: "quantity",
    activity_type_column: "activity",
    position_activity_values: ["BUY", "SELL"],
    non_position_activity_values: ["DIVIDEND", "FEE"],
    reference_column: null,
    price_column: "price",
    currency_column: "currency",
    default_currency: "EUR",
    instrument_aliases: Object.fromEntries(
      VOCABULARY.instruments.map((i) => [i.symbol, i.canonical]),
    ),
  };
}

/** The vault the finance capability imports subscriptions from. Written fresh each run into
 *  demo/vault, which is untracked — see the comment at the call site. */
async function writeSubscriptionNotes(record: string): Promise<void> {
  for (const sub of VOCABULARY.subscriptions) {
    const note = [
      "---",
      `cost: ${(sub.cents / 100).toFixed(2)}`,
      "currency: EUR",
      "billing_cycle: monthly",
      `category: ${sub.category}`,
      "status: active",
      `value_rating: ${(sub.cents % 5) + 1}`,
      "---",
      "",
      `# ${sub.name}`,
      "",
      "A fictional service, in a fictional note, for a demonstration.",
      "",
    ].join("\n");
    const name = sub.name.replace(/[^A-Za-z0-9]+/g, "-");
    await post(record, { path: `Atlas/Finance/Subscriptions/${name}.md`, content: note });
  }
}

// ─── Guards ───────────────────────────────────────────────────────────────────

/** Which overlay the running capabilities were configured from. Asked of paths.sh rather
 *  than reimplemented, because the answer has to be the one the service runner got.
 *
 *  The checkout path arrives as `$1`, never as script text. Interpolating it into the
 *  `-c` string made a checkout directory containing `"` or `$(` a command -- the rule the
 *  Rust half already states (capabilities/comms/src/media.rs: argument arrays only, never
 *  a shell string). CodeQL js/shell-command-injection-from-environment reported it. */
export function activeOverlay(pathsSh: string = join(SJEL_ROOT, "tools/lib/paths.sh")): string {
  const out = execFileSync(
    "bash",
    ["-c", 'source "$1" && printf %s "$SJEL_OVERLAY_ROOT"', "demo-seed", pathsSh],
    { encoding: "utf8" },
  );
  return out.trim();
}

function requireDemoOverlay(): void {
  const active = activeOverlay();
  if (active === DEMO_OVERLAY) return;
  console.error(
    [
      "demo-seed: refusing to write — the active overlay is not the demo overlay.",
      "",
      `  active: ${active}`,
      `  demo:   ${DEMO_OVERLAY}`,
      "",
      "These are real POSTs to whatever store the running capabilities use. Against a",
      "personal overlay that means inventing rows in a real ledger and a real calendar.",
      "",
      "Start the demo stack instead, which points every capability at the demo overlay:",
      "",
      "  tools/demo-up",
      "",
    ].join("\n"),
  );
  process.exit(1);
}

/** One list endpoint per seeded capability, used only to prove it is empty before writing.
 *  A capability whose first declared path is not a list is fine — a non-array answer is
 *  not evidence of data, so it does not block. */
async function refuseIfPopulated(cap: string, url: string, force: boolean): Promise<void> {
  let body: unknown;
  try {
    body = await get(url);
  } catch {
    return; // Unreachable is the runner's problem, reported by waitFor, not this check's.
  }
  // A bare array, or the one-key envelope every list endpoint here uses ({plans: []}).
  // The key was hardcoded to `tasks` until that capability retired (PRD Q48); naming any
  // single capability's key was always the accident, because this guard is about "did
  // anything answer with rows", not about which capability answered.
  const rows = Array.isArray(body)
    ? body.length
    : (Object.values((body ?? {}) as Record<string, unknown>).find(Array.isArray) as
        | unknown[]
        | undefined
      )?.length ?? 0;
  if (rows === 0) return;
  if (force) {
    console.warn(`demo-seed: ${cap} already holds ${rows} rows — seeding anyway (--force)`);
    return;
  }
  console.error(
    `demo-seed: ${cap} already holds ${rows} rows. The demo expects an empty store, and\n` +
      `seeding on top of one produces a corpus nobody can reproduce. Reset it, or pass --force.`,
  );
  process.exit(1);
}

// ─── Main ─────────────────────────────────────────────────────────────────────

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  if (args.includes("-h") || args.includes("--help")) {
    console.log("tools/demo-seed [--only <capability>] [--force]");
    return;
  }
  const onlyIdx = args.indexOf("--only");
  const only = onlyIdx >= 0 ? args[onlyIdx + 1] : null;
  const force = args.includes("--force");

  requireDemoOverlay();

  const manifest = loadManifest();
  const table = routes();
  const url = (browserPath: string) => resolvePath(browserPath, table).url;
  const seeded = manifest.capabilities.filter((c) => c.writes && (!only || c.name === only));
  if (seeded.length === 0) {
    console.error(`demo-seed: nothing to seed${only ? ` for '${only}'` : ""}`);
    process.exit(1);
  }

  for (const cap of seeded) {
    const seeder = SEEDERS[cap.writes!];
    if (!seeder) {
      throw new Error(`demo.toml: [capability.${cap.name}] writes = "${cap.writes}", which no seeder implements`);
    }
    await waitFor(url(cap.paths[0]));
    await refuseIfPopulated(cap.name, url(cap.paths[0]), force);
  }

  for (const cap of seeded) {
    // A fresh Rng per capability, seeded from the manifest seed plus the capability name.
    // Seeding only trips then has to produce the same trips as seeding everything — with one
    // shared generator it would not, and a `--only` run would silently differ from CI's.
    const ctx: Ctx = {
      rng: new Rng(`${manifest.seed}:${cap.name}`),
      anchor: manifest.anchor,
      manifest,
      url,
    };
    const summary = await SEEDERS[cap.writes!](ctx);
    console.log(`seeded ${cap.name}: ${summary}`);
  }
}

if (import.meta.main) {
  main().catch((err) => {
    console.error(`demo-seed: ${err instanceof Error ? err.message : String(err)}`);
    process.exit(1);
  });
}
