# The demo

A running copy of Axon, filled with data that was never real, published at
`larsboes.github.io/Sjel/` and reproducible on any machine with one command.

The dashboard IS the site. A visitor lands in the Axon shell and navigates from there; the
generated reference, self-model included, lives under `/docs`.

It exists because the repository's public page used to be tables generated from `self.json`.
Those are true, checkable and completely useless to somebody asking what the thing looks like.

```
tools/demo-up            start the stack, seed it, record it
tools/demo-site          assemble site/ from the recording
tools/demo-origin        the synthetic remote, started by demo-up (--once prints its payloads)
```

## How the data gets there

Nothing writes fixtures by hand, and no capability crate contains a line of demo code.
`tools/demo-seed` POSTs generated entities through each capability's **own HTTP write API** —
the same routes the dashboard calls — and `tools/demo-record` then GETs every path declared in
`demo.toml` and saves what came back.

Three capabilities do not have a write API to seed through, because their job is to go and
**fetch** something: Comms collects, Scouting scans, Transit queries a timetable. A demo has
nowhere else to fetch from, which is why all three were absent from the first wave. So it now
has one — `tools/demo-origin`, a plain HTTP origin serving an RSS feed, article pages and
bahn.de-shaped journey payloads at `[demo] origin` in the manifest.

It is not a mock of a capability, and none of the three gained a line of demo code. Comms is
fed by posting URLs to its ordinary `/ingest` route and fetches and extracts the pages itself;
Scouting is pointed at the feed by a generated source declaration and parses it with its own
`rss` adapter; Transit reads the origin through the endpoint overrides its HAFAS client already
supports (`SJEL_TRANSIT_FAHRPLAN_URL` and its two siblings), so its real parser does the work.
What gets recorded is their output, not the origin's input.

Comms' half of that needs one written permission. `POST /ingest` refuses a URL that resolves to
an address inside this machine, because an ingested link would otherwise drive any
loopback-bound Axon API (`capabilities/comms/src/media.rs`, `check_destination`, Q74) — and
every URL this demo seeds is exactly such an address, deliberately, so the published corpus
depends on no host anybody else owns. So `tools/demo-up` writes the manifest's `[demo] origin`
into `demo/overlay/config/comms.json` as the single entry of `ingest_allowed_origins`. The demo
clears one origin, not the guard: an entry names a scheme, a host **and a port**, so the
capability ports on the same loopback address stay refused. Seeding is what proves it — if that
entry and `[demo] origin` ever disagree, `tools/demo-up` fails on the first article instead of
publishing a short corpus.

The rail payloads reproduce the real backends *including where they disagree*: `dbnav` carries a
train label and `dbweb` names a regional train by its bare number with no label at all. That
second one is not a shortcut — it is what bahn.de returns, and it is why Transit reports those
legs as unscorable rather than guessing. A stub that helpfully supplied a label would publish a
number the real backend cannot produce.

That indirection is the point:

- A seeder cannot reach past a public contract into a store.
- A seeding run is an end-to-end exercise of the write paths. Finance is filled by generating
  a European bank CSV and pushing it through preview → import → review, which is how a person
  actually onboards, and is why the demo's Import Review screen has something in it.
- The fixtures cannot drift from the API. If a response shape changes, the recording changes
  with it in the same commit; if an endpoint disappears, recording fails rather than
  publishing a page that describes it.

Every value descends from the fixed seed in `demo.toml` through `tools/lib/demo-data.ts`, so
two builds of one commit produce identical bytes and a rebuild is a readable diff.

## Why it cannot leak

Two independent guarantees, deliberately both:

**Synthetic by construction.** The corpus can only contain what `VOCABULARY` holds and what the
generators derive from it — invented people, invented merchants, invented instruments, and
real cities chosen to be somewhere the author is not. `tools/lib/demo-data.test.ts` sweeps every
string in that vocabulary for anything shaped like contact details, a path, a host or an IBAN.

**A gate over the built bytes.** `tools/check-site-payload.sh` scans the assembled `site/`
before it is published and fails the build on a real email address, an IBAN, a workstation home
path, a tailnet name, a private address, or any term derived from the active overlay. In CI it
catches almost nothing, because a runner has no overlay and no real database. It exists for the
**local** build, where both are a directory away.

Three guards stop a local run from writing into a real system, because that failure cannot be
undone:

| Guard | Refuses when |
|---|---|
| `tools/demo-up` | anything is already listening on a capability's port |
| `tools/demo-seed` | the resolved overlay is not `demo/overlay` |
| `tools/demo-seed` | a capability's store already holds rows |

The port check is load-bearing rather than cosmetic. The demo runs on the *same* ports a real
installation does — `tools/capability.sh registry` reads ports from `service.toml` and does not
apply `machine.toml`'s per-capability `port` override, so a demo overlay that moved its ports
would be started on one number and recorded from another. Sharing them means a real stack that
is already up would be silently adopted, and the seeder would post invented rows into it.

## What is missing, and why

`demo.toml`'s `[absent]` section names every capability the demo cannot include, with the
reason. Those reasons are rendered — on the reference pages, and as the error the demo itself
returns if you reach one of their endpoints. A page that 404s teaches a visitor the software is
broken; a page that is honestly missing teaches them where the edge is.

Today that is punctuality, and only punctuality. Its input is Deutsche Bahn's monthly open-data
publication — 100+ MB of parquet folded into cells by a batch job — so there is no POST that
could fill it, and a synthetic histogram would be a hand-written fixture wearing a
measurement's clothes.

Its absence is worth reading rather than skipping: Transit's `reliability` and
`delay_risk_score` come back null in the demo, because the capability that measures them is not
running. That is the designed behaviour — absence degrades, it never fails — shown rather than
papered over.

Comms, Scouting and Transit left this section on 2026-08-20, when the synthetic origin gave
them somewhere to fetch from.

## Files

| Path | What it is |
|---|---|
| `demo.toml` | the manifest — seed, anchor, seeded capabilities, recorded paths, absences |
| `overlay/` | a tracked, fictional Axon overlay; the deployment the demo runs as |
| `fixtures/` | the recording. Untracked, regenerated by `tools/demo-up` |
| `vault/` | fictional subscription notes `tools/demo-seed` writes. Untracked |
| `state/` | the ledger journal and the holdings/balance snapshots Finance keeps outside its database. Untracked, cleared on every start |
| `overlay/config/scouting.json` | Scouting's demo source, generated by `tools/demo-up` from `[demo] origin`. Untracked, because a tracked copy would be the second place that port is written down |
| `overlay/config/comms.json` | Comms' per-run API secret and the one ingest origin the fetch guard clears, generated by `tools/demo-up`. Untracked, for the same reason and because the token is regenerated on every run |
