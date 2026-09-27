# scouting

A generic opportunity-discovery pipeline: scrape a source (events, CFPs/scholarships, ...),
normalize into one shared `Opportunity` type, score it against a configurable "interest
profile" via embeddings/cosine similarity, rank, persist, and optionally cross-reference
against existing notes. Ported from a private LifeOS-mono service; ported here because the
architecture (adapter trait, generic pipeline, config-driven scoring) was never actually
hardcoded to one person's setup — the personal coupling was narrow (a few default paths and
one DB connection string), not a redesign problem.

## Boundary: opportunity engine, not general feed

Scouting finds and ranks opportunities. It normalizes source data and preserves the decision
state for each result. Scholarships, hackathons, events, calls for papers and travel deals
belong here when they need ranking against an interest profile.

It does not own Axon's general observation stream. Security advisories and updates to systems
or packages belong in Feed. Watched-repository changes, general news and interesting articles
also stay there unless a user explicitly promotes one into an opportunity workflow. Scouting
may consume a typed Feed reference and may publish a scored result back to Feed, but the source
ID and provenance must survive that handoff. Feed remains useful without Scouting, and
Scouting can run directly against declared sources without Feed.

The Obsidian Markdown adapter is one declared opportunity source. It does not make Scouting a
general vault importer. It reads only the configured opportunity/profile globs and may link a
result to an existing event note. Trip notes remain owned by `capabilities/trips`; distilled
reading keepers remain owned by `capabilities/comms`.

## Verdict

**Adopt the architecture.** The `SourceAdapter` trait + `Opportunity` schema + cosine-scoring
pipeline is genuinely reusable and reasonably well-tested (unit tests across
store/score/merge/sources/adapters/config/vault_linker; `pipeline`, `embed`, `source`,
`sources/mod` and `server` carry none — `cargo test -- --list` is the count, per
CONTRIBUTING.md#documentation-stays-owned-and-current's no-live-counts-in-prose clause) — proven out again by
`adapters/transit_fare.rs` (`capabilities/store/README.md`'s correlation section, Phase 2), which plugs a fare-search source
from an entirely different capability's crate into this same pipeline with zero changes to
`score`/`pipeline`/`store`.

**The backend has gone Postgres → SQLite → Postgres → SQLite, and every move was right at the
time.** The original port reasoned that Postgres in a docker container for a single-user local
tool was exactly the "way more machinery than a single-user setup needs" anti-pattern
`capabilities/vaultwarden/README.md` calls out for the old HashiCorp Vault experiment — true, at
the time, when this was a standalone tool with no other consumer. Phase 2 changed the shape of
the problem: **persistent cross-domain correlation between scouting and `transit`** is the whole
point of that phase (see `capabilities/store/README.md`'s correlation section), and that requires
one queryable store both capabilities can join across, which a SQLite file *per capability*
cannot be. The third move followed from a measurement rather than a preference (PRD Q45,
2026-08-27): one *shared* SQLite file keeps the joins — that is what `capabilities/store` is —
so the container, the password and the watchdog were paying for nothing this capability used.
Scouting's tables kept their names with the schema folded into the prefix: `scouting.opportunities`
is `scouting_opportunities`. `capabilities/store/README.md` carries the full reasoning; this file
only carries the scouting-specific half of it.

**2 of 4 generic-opportunity adapters are now live-verified: `euro_hackathons` and, since
2026-07-30, `luma`.** `cfp_conferences` and `meetup` are still ported as-is — they
compile/unit-test clean against fixtures, but their live HTTP paths have not been re-checked
since the original build. Treat them as untested until you run each one for real.
`transit_fare` (Phase 2) is separately live-verified — real bahn.de calls during the port that
wired it in, which is also how a real bug in the underlying HAFAS parser got caught (see
`capabilities/transit/README.md` Gotchas).

**What the Luma verification found is the argument for doing these at all.** The adapter
compiled, passed its fixture tests, and did not work: the first real call returned
`parse: events decode: missing field 'entries'`. Nothing was wrong with the parser. Three
separate defects, none of which a fixture can catch, are written up in `src/adapters/luma.rs`'s
module header — a stale hardcoded city table (19 of 20 ids 404 today), a renamed pagination
cursor, and an unchecked HTTP status that turned every 404 into a bogus parse error. The fix
also added the thing the calendar work actually needed and the adapter never had: fetching a
**specific Luma calendar** rather than a city's discover feed (`LumaScope::Calendar`, declared
as a `luma-calendar` source).

The unchecked-status half of that was not Luma's alone — sweeping for it found the same
`.text()`-regardless-of-status shape in four more fetch sites. It now lives once, in `src/http.rs`:
adapters still build their own requests, because their headers genuinely differ, but none of them
holds a private opinion about what a 404 means. Classification is a pure function over
`(url, status, body)`, so the failure path is tested without a network or a fixture server —
which is the whole reason the original defect survived a green fixture suite.

## Architecture

```
main.rs                  ── CLI binary ("scout"), one lib crate "scouting"
  server.rs               ── HTTP binary ("scout-server"), Axum: health, discovery,
                              source registry and persistent opportunity triage
  config.rs               ── resolves interest_profile_dir / events_dir / database_path / port
  source.rs                   (overlay config, see below)
  adapters/{euro_hackathons,cfp_conferences,luma,meetup,transit_fare}.rs  ── one SourceAdapter impl each
  opportunity.rs           ── the shared Opportunity type (folded in as a module, not a
                               second crate -- it's ~90 lines used by one consumer)
  normalize.rs             ── unused scaffold, ported for parity (see file header)
  embed.rs                 ── interest-profile vectors via the `embedding` role from
                               libs/inference; writes telos_vectors.json cache on success
  score.rs                 ── cosine-similarity scoring against interest-profile vectors;
                               cache -> live embed -> hash-embedding fallback (see below)
  sources/mod.rs           ── config-driven source registry, create_adapter() factory
  sources/obsidian_md.rs   ── obsidian-markdown adapter (typed opportunity notes from a vault)
  sources/rss.rs           ── rss adapter (any RSS/Atom feed as a declared source)
  localtime.rs             ── UTC instant -> naive local wall time (hand-rolled EU zone rules,
                               closed set; unsupported zone = error, never a guess)
  calendar_promote.rs      ── saved luma events -> capabilities/calendar entries over HTTP
  pipeline.rs              ── glues adapter -> score -> vault_linker -> store together
  vault_linker.rs          ── annotate-only matching against an events_dir of markdown notes
  store.rs                 ── persistence in the shared SQLite file (`scouting_` table prefix,
                               see Verdict)
```

`cargo` builds and tests this, as it does every Rust capability: `cargo build -p scouting` /
`cargo test -p scouting`. It is a member of the root workspace and resolves through the one
root `Cargo.lock`, while this package's manifest owns its own direct dependencies. That is
the same shape a second Rust capability joins with — `transit` landed next and needed no
build wiring of its own. Between 2026-07 and 2026-08-25 a Bazel graph ran alongside cargo as
the CI-grade path; PRD Q44 retired it, and CI runs the cargo commands above directly.

**`scout-server` HTTP binary** (`src/server.rs`, Axum + Tokio):

- `GET /health` reports service health.
- `GET /discover?adapter=<source-id>` runs one declared or built-in adapter through the full
  score/store pipeline. Results include the stable opportunity id, source, current status
  and any Obsidian match; this is a state-changing scan even though the inherited endpoint
  is still `GET`.
- `GET /sources` lists configured `sources[]` entries and the one live-verified built-in
  network source, `euro_hackathons`. The three fixture-only adapters are deliberately not
  advertised in the dashboard picker. A second array, `proposed`, carries the candidate-source
  inbox with a derived `declared` flag; a database that is down empties that array rather than
  taking the declared list with it.
- `POST /sources/proposed` records a candidate: `adapter` and `locator` required, `label`,
  `note` and `found_by` optional, the last defaulting to `manual`. It cannot start anything,
  and the response says so.
- `POST /sources/proposed/:id/dismiss` takes one out of the inbox for good.
- `GET /opportunities?include_dismissed=true` returns the persistent ranked backlog,
  including stable ids plus start, end and location evidence used by the explicit
  Calendar promotion in Feed's **Discover** view.
- `POST /opportunities/:id/status` accepts `new`, `saved` or `dismissed`. It changes only
  human triage state; a later source scan preserves that decision.

The server was originally dropped during the port — the source
service's server binary had zero consumers in Axon at the time, the same "more machinery
than needed" anti-pattern `capabilities/vaultwarden/README.md` flags for the old HashiCorp
Vault setup. Reopened once `dashboard` landed as a real, named consumer — see
`dashboard/README.md` for the full reasoning and why
that reopening is recorded rather than silent. Port resolution is `SJEL_PORT` (exported by
`tools/service-runner.sh` from `service.toml`) → `config.rs`'s `port` field → `8084`. Built
as the `scout-server` binary declared in `Cargo.toml`.

The default is 8084 rather than 8080 because `capabilities/vaultwarden` publishes 8080 on the
host: two capabilities shipping the same default port is a collision that stays invisible
until something starts both, which is exactly what the runner's `up --all` did the first time.

## Is this simple enough? Size and what's actually load-bearing

This crate is big, but it isn't one lump. `wc -l src/**/*.rs` gives the current size and
`cargo test -- --list` the current test count; what matters here is which part carries which
risk, and that doesn't change when a number does (CONTRIBUTING.md#documentation-stays-owned-and-current — the hand-counted totals
that used to open this section had drifted by the time anyone read them).

| Part | Status |
|---|---|
| Core engine (config, pipeline, scoring, store, vault-linker, opportunity type) | Unit-tested throughout; the store tests need a live local Postgres — see Gotchas |
| `euro_hackathons` adapter | **Live-verified** — real API call confirmed during the port |
| `luma` adapter | **Live-verified 2026-07-30** — both scopes run against Luma for real; three defects found and fixed, see § Verdict |
| `cfp_conferences` + `meetup` adapters | Compile, pass fixture tests, **never verified against their live APIs** |
| `transit_fare` adapter | **Live-verified** — real bahn.de calls during Phase 2's wiring |
| `obsidian_md` + `rss` source adapters, `merge` | fixture/unit-tested; added after this table's first draft |
| `localtime` + `calendar_promote` | unit-tested; the promotion path was additionally exercised end to end against a live calendar service |

The honest simplification isn't deleting the unverified adapters (they're tested against
fixtures, they cost nothing to keep, and deleting working code to re-derive it later is worse
than labeling it correctly) — it's making sure nobody mistakes "compiles" for "trustworthy."
`main.rs` prints a loud warning to stderr for the remaining fixture-only API adapters
(`meetup`, `cfp`, `cfp_conferences`); the live-verified `euro_hackathons`, `luma` and
`transit_fare` paths, and configured sources, stay quiet. Default behavior only ever uses a
verified path.

## Extending it

Adding a new source is genuinely low-ceremony, not aspirationally so: implement
`SourceAdapter`'s four methods (`name`, `opportunity_type`, `rate_limit_per_min`, `search`
returning `Vec<Opportunity>`), register it in `src/sources/mod.rs`'s `create_adapter()`
factory (one arm), and add a config entry to `scouting.json`'s `sources[]`. Nothing in
`main.rs`, `server.rs`, `score`, `pipeline`, or `store` needs to change — they're already
adapter-agnostic. Budget ~150–300 lines per source (mostly the HTTP client + response parsing
+ field mapping into `Opportunity`), based on the five shipped adapters.

See § Verdict above for the architecture call.

## Candidate sources (queued, not built)

Two source ideas carried over from `inspirations.md` (now drained and removed), neither turned
into an adapter yet:

- **Job scouting** — [lifeelo.com](https://www.lifeelo.com), not evaluated. Same shape as any
  other source if it turns out to have a scrapeable listing feed: a new `SourceAdapter` impl
  plus one match-arm (see Extending it above). No council held yet on whether it's actually
  good — don't assume it is just because it's listed here.
- **Travel deals** — [travel-dealz.de](https://travel-dealz.de) as a deal-aggregator source,
  plus the TER Grand Est ["Pass Jeune"](https://www.ter.sncf.com/grand-est/tarifs-cartes/bons-plans/pass-jeune)
  regional youth rail pass as a concrete opportunity type worth scoring (cheap short trips from
  Bonn into RLP/Grand Est). This one sits closer to `transit` than to the generic adapters
  above — likely the second cross-capability source after `transit_fare`
  (`capabilities/store/README.md`'s correlation section, Phase 2), not a standalone adapter.

## Alternatives considered (adopt-before-build check)

Axon doctrine (CONTRIBUTING.md#dependency-verdicts-and-provenance, `upstreams.toml`) requires checking for an existing tool
before building custom. Researched before writing this section:

**Whole capability** — nothing found that does "aggregate heterogeneous sources → score
against one person's static interest profile → rank," turnkey:
- [`gorse-io/gorse`](https://github.com/gorse-io/gorse) (Go, Apache-2.0, ~9.7k stars,
  actively maintained) is the closest real recommendation-engine framework and does support
  embedding-based ranking — but it's built around collaborative filtering over
  users×items×feedback (Redis/MySQL/ClickHouse backing services), for multi-user products.
  Modeling one person's static profile into that schema plus running a multi-service stack
  would cost more than the ~1450-line engine it would replace. Not adopted: wrong shape of
  problem.
- RSS readers (Miniflux, FreshRSS, Feedbin — all active/maintained) solve feed reading, not
  scored multi-source discovery; none support pluggable embedding-based relevance scoring
  against a personal profile, and none ingest non-RSS sources (Luma, Meetup, CFP boards)
  natively. Not adopted: doesn't fit.
- [`troyriverabusiness/scout`](https://github.com/troyriverabusiness/scout) — already logged
  in `upstreams.toml` as the architecture reference for this design. Re-checked: it's VC
  deal-flow scoring (startups against investment-thesis vectors), a ~3-day weekend build
  with no commits since, 0 stars, **no LICENSE file** (legally un-adoptable regardless of
  fit). Confirmed as inspiration-only, correctly logged, not a runnable candidate.

**Per-adapter** — narrower, more likely to have real alternatives:
- **CFP/conferences**: real find —
  [`scraly/developers-conferences-agenda`](https://github.com/scraly/developers-conferences-agenda)
  (MIT, ~2k stars, actively maintained) publishes structured JSON feeds
  (`developers.events/all-cfps.json`) instead of requiring scraping. Not swapped in here:
  it's general developer-conference coverage, not the ML/AI-specific deadline list (with
  h-index venue ranking) the current adapter tracks — different topical scope, not a strict
  upgrade for this use case. Worth adding as a second, complementary adapter later if
  broader non-ML coverage becomes useful; noting it here rather than silently ignoring it.
- **Luma**: only maintained-looking unofficial client found
  ([`Zettersten/Lu.Ma`](https://github.com/Zettersten/Lu.Ma), .NET) hasn't been touched since
  2024 and has 0 stars — effectively abandoned. Hand-rolled adapter stands.
- **Meetup**: every scraper found (several Python/Scrapy variants, a commercial Apify actor)
  does the same job this adapter does — none more robust against Meetup's frontend churn,
  and Meetup has had no public search API since 2019. Nothing free/self-hosted beats the
  hand-rolled approach.
- **Euro hackathons**: the upstream source
  ([`lorenzopalaia/Euro-Hackathons`](https://github.com/lorenzopalaia/Euro-Hackathons),
  actively maintained) is already a good, current source — confirms the one live-verified
  adapter is pointed at the right thing.

## Why this shape: sources are declared, never discovered

Event sources live in a config array, not hardcoded in code and not scattered across manifests.
The heading keeps the shape it was written in; what it rules out has since been split in two,
because the original absolute ruled out something worth having.

**What runs is declared.** Nothing polls unless it has an entry in the overlay's `sources[]`
array. Axon does not probe for unannounced sources, and no discovery path may promote itself
into a running one — explicit declaration beats heuristic detection, and the overlay is the
right place for personal declarations. This half is unchanged and is the one that matters.

**What is proposed may be discovered.** A candidate source found while doing something else is
not noise, and throwing it away to protect the rule above was the overcorrection. A proposal
lands disabled, carrying where it came from, and stays inert until a human moves it into
`sources[]`. Discovery earns a suggestion, never a fetch.

The second half is the `proposed_sources` table plus two routes. A candidate lands with its
provenance (what found it, when, and any note), shows up under `proposed` in `GET /sources`, and
sits there. Nothing in this crate can promote it: `create_adapter` is only ever called on
`Config::sources`, which is read from the overlay file that no code path here writes. Promotion
is a human copying the entry across.

Three details decide whether the inbox stays worth opening. Identity is the lowercased
`(adapter, locator)` pair, so noticing the same hub on three runs is one proposal seen three
times; a re-sighting keeps the original `found_at`, since when it first appeared is the useful
fact, and refreshes the label and what found it. A dismissal sticks, because re-proposing what
the operator already refused is how an inbox stops being read. And there is no `promoted`
status: promotion happens in a file this process cannot write, so a stored status claiming it
had happened would be this table's opinion rather than a fact. The listing derives `declared`
instead, by comparing each proposal against the sources configured right now.

Typing a hub id in by hand is a first-class producer, and today it is the only one. Automatic
discovery is a separate, larger question: a Splash *event* page carries no hub id anywhere in its
markup, so finding a hub means visiting a hub, which is a search sweep and a rung above this.

The other limit here is unrelated to discovery and stays absolute. Adapter names are a closed
enum: `sources[].adapter` accepts a fixed set (`obsidian-markdown`,
`rss`, `luma-calendar`, `splash-hub` — those four only; anything else is an `UnknownAdapter`
error), and
adding a type means adding an arm to `create_adapter()` in
`capabilities/scouting/src/sources/mod.rs`. Not a plugin system on purpose: the set of formats
worth connecting to is small enough that a match arm beats dynamic loading.

`splash-hub` is the same move for Splash That, a white-label event platform of Luma's shape.
One adapter keyed by hub id covers every brand hosting there, so the declared thing is
`<host>/<hub_id>` in `url` — same-origin query, so the numeric id alone does not say which host
answers for it.

Three measured facts shaped the implementation, and each has a test. The date filter is not
optional: unfiltered, a hub returns its whole history, which for the one this was built against
is 3015 records and 5.5 MB, 3009 of them already past. Asking for `upcoming` returns about
10 KB. The `result` field is an object keyed by event id rather than an array, so its keys are
read and then sorted, because an object has no order and a run that reshuffles its own output
cannot be diffed. And `end_timestamp` arrives as a number or an empty string within the same
response, so it is coerced rather than strictly typed; one event with no end time would
otherwise fail the entire hub.

The record also carries a local wall-time string beside the epoch, and the two differ by the
venue's offset. The epoch is what this reads; parsing the string as UTC would put every North
American event on the wrong evening. Yield is honestly small: on 2026-08-05 the hub carried six
upcoming events, one of them European. Expect empty European sweeps for stretches.

`luma-calendar` is what makes "which Luma calendars do I track" a declaration rather than a
code change. Each entry names one calendar by its `cal-…` api id in the `url` field; the
adapter fetches that calendar's future events. `--luma-calendar <id>` runs one ad hoc, which is
the try-before-you-declare path — the durable answer belongs in `sources[]`.

## Config resolution

Mirrors `capabilities/printing/printctl.py`'s `_cfg_path()`/`load_cfg()` exactly (see
`src/config.rs`):

1. `$SJEL_SCOUTING_CONFIG` — explicit override, full path to a JSON file
2. `$SJEL_PERSONAL_ROOT/config/scouting.json` — the overlay (exported by `tools/lib/paths.sh`)
3. `capabilities/scouting/scouting.config.json` — local, gitignored, dev fallback

Copy `scouting.config.example.json` to `$SJEL_PERSONAL_ROOT/config/scouting.json` and fill
in real values there; nothing personal is stored in Axon. Every field is optional — the tool
runs against empty/local directories with zero config (interest-profile dir gets created if
missing, events-dir cross-referencing is silently skipped if unset). CLI args
(`--database-path`, `--limit`, `--opp-embeddings`, etc.) always override whatever the config
file resolves.

The four tables live in the shared SQLite file — `SJEL_DB_PATH`, else
`$SJEL_PERSONAL_ROOT/data/axon/axon.db` — under the table prefix `scouting`, so they are
`scouting_opportunities`, `scouting_links`, `scouting_source_state` and
`scouting_proposed_sources` (`libs/sjel-store/README.md`). PRD Q45 (2026-08-27) moved them
there from a Postgres schema. The path is a deployment fact rather than a capability one, so
a `database_url` left in `scouting.json` is ignored: a file per capability would drop the
cross-capability correlation with `transit` that Phase 2 exists for.

| Field | Was (LifeOS-mono) | Now |
|---|---|---|
| `interest_profile_dir` | hardcoded `{vault_root}/TELOS/Focus` | directory of markdown files (`summary:`/`current_focus:`/`> [!quote] Charter` convention, parsed generically — doesn't have to be an Obsidian vault) |
| `events_dir` | hardcoded `{vault_root}/Atlas/Events` | any directory of markdown event notes; unset = vault-linking silently skipped |
| `opp_embeddings_path` | `--opp-embeddings` CLI flag / `LIFEOS_ROOT`-relative default | CLI-arg-only still (hash-fallback embedding works with zero config, not worth a config knob) |
| `sources[]` | hardcoded match arms in `main.rs` + `server.rs` | Config array of declared opportunity sources (adapter type, path/URL, glob patterns). Each entry resolves into a `SourceManifest` at startup; enabled sources run by default. `obsidian-markdown` accepts `opportunities_glob` + `opportunity_type`; `events_glob` remains compatible. See § Extending it above. |
| `port` | `SCOUTING_PORT` env var | `SJEL_PORT` (from `service.toml`, set by the runner) → `port` field → `8084` |
| `calendar_base_url` | — (new) | `capabilities/calendar` for the promotion; loopback default `http://127.0.0.1:8087`, `--calendar-url` overrides |
| `home_timezone` | — (new) | **No default.** Required by `--promote-calendar` (or `--timezone`); see § Calendar promotion |
| `geo` | — (new) | Private event-routing policy: optional home coordinate plus explicit local radius, local country tokens, timezone prefixes, and a safe-default-off `allow_unknown` compatibility override. No public home/radius defaults; see § Event routing. |

### Where a profile lives

A source's `profiles_glob` resolves under its `path` unless the entry also declares
`profile_path`. Both forms work; the second exists because the two things a source entry
points at are not owned by the same system.

An interest profile is a **consumer input** — an operator-curated or TELOS-derived predicate
about what is worth surfacing, which is private runtime configuration. Opportunity notes live
in a knowledge store with its own sync lifecycle. Nothing about scouting requires those to
share a root, and forcing them to did two bad things: it made a matching profile a required
resident of somebody's Obsidian vault, and it meant duplicating the profile to score a second
source against it.

So:

```json
{
  "id": "events-radar",
  "adapter": "obsidian-markdown",
  "path": "~/knowledge-store",
  "opportunities_glob": "Applications/*.md",
  "profile_path": "~/profile-state",
  "profiles_glob": "Events Profile.md"
}
```

Moving a profile changes nothing else: source identity, opportunity ids and provenance are
unaffected, because none of them were ever derived from where the profile was read.

Every read is bounded by whichever root declared it — `//libs/markdown-root` refuses an
absolute pattern or one containing `..` before touching the filesystem, and proves each
resolved file is inside the root *after* symlink resolution rather than by string prefix. A
source that declares a `profiles_glob` with no root to resolve it against, or a root that is
not there, is now a named error on stderr instead of a silent skip: a profile that quietly
stops being applied changes every score in the run, and nothing else downstream would mention
it.

## Event routing

Interest score answers whether an event matters. The event route answers where it belongs, and
is returned by both discovery and backlog APIs without removing the opportunity:

- `local` — within the configured radius, or matched by the bounded country/timezone fallback;
- `travel_candidate` — outside that reach and retained for the travel workflow;
- `online` — established by source metadata or a bounded online/virtual/remote location token;
- `unresolved` — the stored evidence or private policy cannot decide safely.

The classifier first trusts explicit online evidence. For a physical event it uses great-circle
distance only when the private policy has a complete, valid home coordinate, a positive local
radius, and the opportunity has a complete, valid coordinate. Otherwise it falls back to the
configured local country tokens and then timezone prefixes. A missing or half coordinate is never
turned into `0,0`, and city-string equality is never used. `allow_unknown = true` is an explicit
legacy override that routes an otherwise unknown event locally; its safe default is `false`.

The public example contains only null/empty placeholders. Real home coordinates, radius, and
reach tokens belong in `$SJEL_PERSONAL_ROOT/config/scouting.json`.

## Calendar promotion (calendar Phase A)

`scout --promote-calendar` classifies every opportunity with `source = luma` and
`status = saved`. It upserts only `local` and `online` events into `capabilities/calendar` via its idempotent
`PUT /api/entries/external`, as kind `event` with the bare Luma event id
(`evt-…`, not scouting's namespaced `evt:luma:evt-…`) as `external_id`. Calendar's partial
unique index on `(source, external_id)` makes a repeat run update the same row rather than add
a second; `created_at` and the entry id stay put across runs, only `updated_at` advances, and
the request body is byte-identical each time (no wall-clock stamp in `payload` —
`calendar_promote::tests::the_request_body_is_byte_stable_across_runs` pins that).

`status = saved` is the trigger because it is already the operator's explicit "yes, this one",
and `store.rs` guarantees it survives a refetch. Scouting never writes calendar's tables —
everything goes over the HTTP contract, so calendar keeps deciding what a valid entry is.

`travel_candidate` and `unresolved` events remain in Scouting and appear in the promotion report
under `routed`, with their classification reason. They are not calendar errors and are not
dropped. Matching travel candidates to Trips and feasible Calendar windows is a separate
consumer concern; this classifier calls neither service. The Travel dashboard owns that
composition through the public Scouting, Trips, and Calendar APIs; its matching and action
contract is documented in `capabilities/trips/README.md`.

Two things it refuses rather than guesses, matching calendar's own no-guessing rule for dates:

- an opportunity with no usable start or end, or a start/end that is not an explicit UTC
  instant, is **reported as skipped** and left alone;
- an unsupported `home_timezone` aborts the run. Luma reports UTC and calendar stores naive
  local wall time; `localtime.rs` covers UTC, fixed `±HH:MM` offsets and a closed set of EU
  zones, hand-rolled rather than pulling `chrono-tz` in for one zone's
  DST rule — `capabilities/calendar/src/date.rs` already hand-rolls the same civil arithmetic.

`payload` carries an inert evidence snapshot: the originating opportunity id, its URL, score,
matched focus and rationale, both original UTC instants alongside the zone they were converted
with, and the event route, basis, reason, and optional distance.

## Scholarship Radar contract

The first scholarship integration is deliberately a manual-sweep workflow, matching the vault's
own rollout rule: verify 2–3 agent-assisted sweeps before adding a scheduled scraper. Scouting reads
the private vault directly; it does not copy personal profile data into Axon.

Configure an `obsidian-markdown` source with:

```json
{
  "id": "scholarship-radar",
  "adapter": "obsidian-markdown",
  "path": "~/path/to/vault",
  "opportunities_glob": "Projects/Life-Plan/Applications/*.md",
  "opportunity_type": "scholarship",
  "profiles_glob": "TELOS/Personal/Scholarship Profile.md",
  "enabled": true
}
```

An actionable scholarship note must declare the following frontmatter:

```yaml
type: scholarship
status: radar
eligibility: eligible
deadline: 2027-03-01
required_start: 2027-10-01       # or "none" for non-degree funding
employment_compatible: yes
deferral: not-applicable         # yes | no | unknown | not-applicable
payment_start: 2027-10-01
source_url: "https://provider.example/call"
```

The adapter is fail-closed. A scholarship is omitted from the actionable ranking unless it has an
active status, `eligibility: eligible`, a deadline and canonical URL, and all four
timing/compatibility fields. `employment_compatible` must be `yes`. Incomplete, terminal, and
ineligible notes remain in Obsidian for review and history; they are not silently promoted by
embedding similarity. This separates the required study start from the payment date and makes
intake, employment, and deferral checks happen before fit scoring.

Run one configured source without writing to Postgres:

```bash
cargo run --manifest-path capabilities/scouting/Cargo.toml --bin scout -- \
  --adapter scholarship-radar --no-store
```

The store path is printed plainly wherever it used to be redacted: `Config::database_path`
names a file, not a credential, so `config::redact_database_url()` is gone with the DSN it
existed for.

Interest-profile vectors resolve in three steps (`score.rs::load_telos_profiles`,
`src/embed.rs`): (1) if a `telos_vectors.json` cache exists inside `interest_profile_dir`
**and names the same producer** the current run would use, it's used for real cosine scoring;
(2) otherwise the pipeline asks `libs/inference` for the `embedding` role — the overlay's
`inference.json` names the backend, the model and its query/document prefixes — and writes
`telos_vectors.json` plus its producer on success, so the next run hits the cache; (3) if this
machine declares no `embedding` role, or the backend is unreachable, it falls back to a
deterministic hash-embedding so the pipeline stays runnable with zero ML tooling installed.

The producer check in step 1 is the point: a cache keyed on the profile alone would serve
`multilingual-e5` vectors to a `nomic-embed-text` run after a backend switch, and every score
would be wrong with nothing logged.
(Path resolution was simplified during the port — the original walked two parent directories
up from a monorepo-relative vault path to find `infra/data/telos_vectors.json`; here it's just
`<interest_profile_dir>/telos_vectors.json`.)

## Status tracking (Phase 1 correlation-engine memory)

The actual point of this phase: **re-running a discovery fetch stops re-surfacing
opportunities you already judged.** Every opportunity row now carries a `status` — `new`
(default), `dismissed`, or `saved` (Postgres `CHECK` constraint; `Store::VALID_STATUSES` is the
same three values enforced in Rust so a typo errors instead of silently no-op'ing). A new
`source_state` table (`adapter_name` PK, `last_run_at`, `cursor`) records one row per *source*
each time a real discovery fetch completes.

**Per source, not per adapter type.** `SourceAdapter::name()` returns the `sources[]` id a
config-built adapter was created from, so two declared RSS feeds keep two cursor rows. It
returned `&'static str` until 2026-08-02, which made that impossible: every `rss` entry answered
`"rss"` and shared one row, and the same held for two tracked Luma calendars. A hardcoded
pipeline adapter has no configured id and keeps its own literal, which is already unique.
Existing `source_state` rows keyed on the old type names are orphaned rather than migrated —
the cursor is scaffolding with no incremental-fetch consumer yet, so the next run simply writes
the new key.

**The one correctness-critical piece:** `upsert()`'s `ON CONFLICT DO UPDATE` deliberately
does **not** touch `status`. An opportunity re-fetched from its source tomorrow — same id,
fresh score/rationale/`fetched_at` — still gets its title/score/etc. refreshed, but a prior
dismiss/save decision survives untouched. `store::db_tests::upsert_preserves_status_across_refetch`
is the test that proves this; it's the test that matters most in this file.

- `scout --dismiss <id>` / `scout --save <id>` — one-shot actions (not part of a discovery
  run): set an opportunity's status and exit. Errors (exit 1) on an unknown id or an invalid
  status string — never silently no-ops.
- The default ranked-output path and `--backlog` both exclude `status = 'dismissed'` by
  default. `--backlog --include-dismissed` overrides this for debugging/visibility. `saved`
  and `new` always show either way.
- Dismissed opportunities that get re-fetched are still upserted (so `last_seen` stays
  current — the record isn't lost, it's just hidden from the ranked view).
- `record_run(adapter_name, cursor)` fires once per real discovery invocation (skipped
  entirely in `--no-store` mode — nothing to write into). **Honest gap, same pattern as the
  "only 1 of 4 adapters live-verified" note above:** `cursor` is scaffolding only right now —
  nothing reads it back yet to do incremental/since-last-run fetching. Every adapter still
  fetches its full result set every run; only the *scoring/display* layer remembers what's
  already been judged. Wiring real incremental fetch per adapter is future work, not this
  phase.

## Commands

`cargo test` needs nothing running. `store`'s tests are real database tests — they were the
reason this line used to name a server — and since PRD Q45 (2026-08-27) each one opens its own
temp SQLite file. `cargo test -p scouting db_tests::` runs that half alone.

```bash
cargo build                                                    # single "scout" binary
cargo test                                                     # unit tests, count grows with adapters/sources

scout --list-sources                                            # show configured event sources
scout                                                           # run all enabled sources (default)
scout --adapter knowledge-base                                  # run one specific source by id
scout --adapter euro_hackathons                                 # run one API adapter (no config needed)
scout --emit-json --adapter euro_hackathons                     # raw fetch, no scoring/store
scout --no-store --limit 5                                      # scored backlog, nothing persisted
scout --backlog                                                 # show what's already in the store
scout --backlog --include-dismissed                             # ...including dismissed
scout --dismiss evt:obsidian:some-event                         # mark dismissed, exit
scout --save evt:obsidian:some-event                            # mark saved, exit
scout --adapter transit_fare --date-from 2026-08-15T08:00:00    # fare-search as a scored source
scout --adapter luma --location berlin                          # a city's Luma discover feed
scout --luma-calendar cal-TOpA5LAFfuDeFpu                       # one Luma calendar, ad hoc
scout --promote-calendar --timezone Europe/Berlin --dry-run     # what would land in calendar
scout --promote-calendar --timezone Europe/Berlin               # saved luma events -> calendar

cargo build --locked --release --bin scout                      # what service-runner builds
cargo test -p scouting db_tests::                                # the store suite alone

cargo run --bin scout-server                                    # HTTP API: health, sources, scan, backlog, status
cargo build --locked --release --bin scout-server               # what service-runner builds
```

`--adapter` accepts any source id from `scouting.json`'s `sources[]` array, plus the built-in
API adapters `euro_hackathons` (default when no sources are configured, live-verified),
`luma`, `meetup`, `cfp`/`cfp_conferences` (fixture-tested only, see Gotchas), and
`transit_fare` (live-verified, needs `--date-from` and `default_from_eva`/`default_to_eva`
configured in transit's overlay config).

Anything else exits 1 with an error listing both the declared source ids and the built-in
names, and a disabled source is listed as disabled rather than omitted. It used to run
`euro_hackathons`: a catch-all arm built that adapter for every unrecognized name, so
`--adapter meetupp` fetched hackathons, printed them, and exited 0. Two nearer misses fell
through the same hole — a declared source whose config could not build (which printed its own
error first and then ran hackathons anyway) and a source the operator had deliberately
disabled. A run answering the wrong question while looking healthy is worse than one that
stops, so none of the three falls through now.

## Redactions applied during the port

- `store.rs`'s hardcoded personal Postgres URL — still gone.
  The backend went back to Postgres (see Verdict), but the connection string is never
  hardcoded with a real credential: it's built from `axon-overlay/config/postgres.env`
  (itself Vaultwarden-provisioned, see `tools/setup-secret.sh`) or a generic
  `axon`/`axon`/`127.0.0.1` placeholder for zero-config local dev — never a personal value.
- `main.rs`/`server_main.rs`'s `VAULT_PATH`, `LIFEOS_ROOT`, `SCOUTING_PORT`, `DATABASE_URL`
  env-var reads and the `CARGO_MANIFEST_DIR`-relative vault-guessing (`.ancestors().nth(2)`) —
  all replaced by `config.rs` (that guessing logic was monorepo-structure-specific and has no
  place in a standalone crate).
- User-Agent strings in `source.rs`, `adapters/cfp_conferences.rs`, `adapters/luma.rs` —
  rebranded to `Axon-Scouting/0.1 (+https://github.com/larsboes/Sjel)`. Axon has no public
  GitHub remote yet (see `PROJECTS.md`) — this is the intended future URL, update the comment
  once it's live.
- The source's separate `server_main.rs` entrypoint collapsed into this port's single
  `server.rs` (handlers + `#[tokio::main]` in one file). `server.rs` itself is a real, current
  binary again (see Architecture) — the "removed entirely, no consumer" era ended once
  `dashboard` became a named consumer; see
  `dashboard/README.md`.

## Gotchas

- **2 of 4 generic-opportunity adapters are live-verified** (see Verdict above) — don't trust
  `meetup` or `cfp_conferences` in production without running them for real first.
  `transit_fare` is separately live-verified (a different capability's crate, own
  verification story — see Verdict).
- **`adapters/luma.rs`'s `FALLBACK_CITIES` is now one entry, and it is a fallback.** It used
  to be a ~20-city table consulted *before* the network, which is how a stale id table
  shadowed a healthy `bootstrap-page` endpoint until 2026-07-30; 19 of those 20 ids 404 today.
  City resolution now hits the live bootstrap first and only falls back offline. Bonn, Cologne
  and Frankfurt were never Luma discover places at all — asking for one now lists the ~80
  places that do exist instead of failing obscurely.
- **A Luma calendar is addressed by its `cal-…` api id, never by its slug.** Luma exposes no
  public slug→id lookup, so `luma-calendar` sources and `--luma-calendar` reject a slug rather
  than guess a URL. Read the id off any event's `raw.calendar_api_id`.
- **`--promote-calendar` refuses to run without a home timezone.** Luma reports UTC instants,
  calendar stores naive local wall time, and guessing between them writes entries that are
  silently an hour or a day off. `localtime.rs` supports UTC, fixed `±HH:MM` offsets and a
  closed set of EU zones; anything else is an error, not a fallback.
- **`adapters/meetup.rs` spoofs a real browser User-Agent on purpose** — Meetup has no
  convenient public JSON API for event search, so this adapter scrapes server-rendered HTML
  and regex-extracts a `__NEXT_DATA__`/Apollo-state JSON blob. That's a genuine ToS-evasion
  pattern, named here rather than hidden: if Meetup changes its frontend framework or adds
  bot detection, this adapter breaks silently (parse error), not loudly.
- **`cargo test` needs no server at all since PRD Q45** — `store`'s tests each get their own temp file, which
  is the same isolation the per-pid schema bought and the same the original SQLite version
  had, with no shared `static Mutex` serializing tests against each other. The config test
  that clears `SJEL_PERSONAL_ROOT` restores it on drop: Rust runs a crate's tests as threads
  of one process, and an unrestored `remove_var` leaves every later store test resolving a
  different file from the one it just wrote to.
- `normalize.rs` is dead scaffold code (ported for parity, `#[allow(dead_code)]`) — every
  shipped adapter does its own normalization inline; nothing calls the top-level function.

## Upstream reference

`troyriverabusiness/scout` — inspiration for the embeddings+scoring architecture (mined for
ideas, not vendored; no code copied). See `upstreams.toml`.
