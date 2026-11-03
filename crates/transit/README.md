# transit

HAFAS journey search, split-ticket solving, rail ticket extraction, journey reliability scoring,
and fuzzy trip-search sessions.

The technical manual for Travel and Transit is in [`website/content/capabilities/travel.md`](../../website/content/capabilities/travel.md).

## Surface

### HTTP API

The transit server (`transit-server`) binds loopback on `SJEL_PORT` (or `TRANSIT_PORT`, default 3000):

- `GET /health`, `GET /api/health` — service status and database connectivity.
- `GET /api/suggest?q=<query>` — station name autocomplete to EVA codes.
- `GET /api/search?from=<eva>&to=<eva>&time=<iso>` — journey search with reliability scoring and preference ranking.
- `GET /api/split?from=<eva>&to=<eva>&time=<iso>` — cheapest split-ticket chain with segment train matching.
- `GET /api/trips?session_id=<id>&limit=<n>` — stored trips with legs, cheapest-first.

### CLI

```bash
transit suggest --query "Bonn"
transit search  --from 8000044 --to 8000207 --time 2026-09-15T09:00:00
transit split   --from 8000044 --to 8000207 --time 2026-09-15T09:00:00
transit import  ticket.pdf

# Fuzzy trip-search session across candidate cities and date windows
transit plan --from 8000044 --destinations "Valencia,Copenhagen" \
  --date-from 2026-09-01 --date-to 2026-09-30 --intent "late summer trip"
transit plan --show trip:session:<id>
```

Overlay configuration at `$SJEL_PERSONAL_ROOT/config/transit.json` can define `default_from_eva`,
`default_to_eva`, and `default_time`. When omitted, the CLI requires `--from`, `--to`, and `--time` explicitly.

## Why this shape: Journey reliability

`Journey.reliability` calculates the probability that a connection holds together and arrives on schedule:

1. **Empirical exceedance over fixed estimates.** Reliability is the product of transfer catch probabilities
   and the final leg arriving within six minutes. Each transfer is evaluated against `punctuality`'s stored
   delay distribution at the exact buffer duration of that transfer.
2. **Missing terms invalidate the product.** If a leg cannot be scored (such as an unmonitored local tram),
   the overall journey reliability is reported as `null` rather than a partial product that would deceptively
   understate risk.
3. **Train label mapping.** Queries ask `punctuality` for the service category derived from the train label
   (`train_name` prefix like `ICE`, `RE`, `IC`, `RB`), rather than raw HAFAS backend product classes which
   conflate regional lines into broader categories. Known lines bridge backends via `train_number`.

## Why this shape: Ranked journey search

`GET /api/search` orders connections according to traveler preferences when configured:

- **Four factors**: `price` (relative to cheapest in result), `duration` (relative to fastest), `changes`
  (relative to fewest), and `reliability` (absolute probability).
- **Normalization**: Relative factors scale against the current search window. Unmeasured factors drop
  out and remaining weights re-normalize to 1.0. If `traveler` is unreachable or unconfigured, results
  preserve the carrier backend's native order.
- **Trip overrides**: Requests can specify `priority` presets (`cheapest`, `fastest`, `fewest_changes`,
  `reliable`, `balanced`) or custom `weights` (`price:0.5,duration:0.2,changes:0.1,reliability:0.2`).
  Natural language intent phrases are resolved via deterministic keyword vocabulary in `src/ranking.rs`.

## Why this shape: Split-ticket solving

The split-ticket solver evaluates intermediate stop combinations via dynamic programming to identify
chains cheaper than the through-ticket fare:

- **Segment matching**: Because pairwise queries return independent results, each segment carries a
  `train_match` status (`exact`, `partial`, `different`, `unknown`) compared against the through train.
- **Confidence reporting**: The chain reports overall `confidence` (`low` if any segment uses a different
  train) and tracks `unpriced_pairs` vs `queried_pairs`. Chains with mismatched trains withhold direct
  booking links in user interfaces.

## Why this shape: Dual rail backends

Transit supports two independent upstream backends, selected via `SJEL_TRANSIT_BACKEND`:

- **`dbnav` (default)**: Queries the mobile app API (`https://app.services-bahn.de`). Provides geographic coordinates
  on all stations, consistent ISO offset datetimes, and product classes aligned with open punctuality data.
- **`dbweb`**: Queries `https://bahn.de/web/api`. Serves as fallback if the mobile endpoint experiences disruption.

Endpoints can be overridden for testing or synthetic demonstrations:
`SJEL_TRANSIT_DBNAV_FAHRPLAN_URL`, `SJEL_TRANSIT_FAHRPLAN_URL`, and `SJEL_TRANSIT_ORTE_URL`.
When set, the real parsers execute against test origins (such as `tools/demo-origin`) without altering
data extraction logic.

## Why this shape: Ticket extraction

Ticket parsing in `src/extractor.rs` extracts booking references, passenger names, journey segments,
and price details:

- **Backend selection**: `document_backend` in overlay config selects `builtin` (native PDF and email text
  parser) or `xberg` (CLI wrapper for image and OCR ingestion).
- **Row shape matching**: Because PDF text extraction often flattens table cells, the parser matches
  chronological row structures (departure time, origin, arrival time, destination, train number) rather
  than relying on fixed layout markers.
- **Schedule vs real-time**: Parsed legs distinguish scheduled times from real-time updates and cancellations
  (`originCancelled` / `destinationCancelled`).

## Why this shape: Trip persistence

Transit persists data in the shared SQLite database (`sjel_config::database_path()`) under the `transit_` prefix:

- `transit_trips`: Saved journeys and search outcomes.
- `transit_trip_legs`: Individual journey segments with station EVAs, operator details, and timestamps.
- `transit_trip_sessions`: Fuzzy search intent sessions with target destination sets and date bounds.
- **Triggers**: Journeys are tagged by origin trigger: `manual` (direct CLI or API call), `auto`
  (background scans via `crates/scouting`), or `session` (`transit plan` multi-destination exploration).
  Upserts preserve existing session associations on conflict.

## Considered and declined

- **No ONNX delay model in transit.** Machine learning inference models with static weights are declined
  in favor of empirical delay distributions queried live from `capabilities/punctuality`.
- **No automated ticket purchasing.** Sjel stores booking confirmations and links to carrier interfaces,
  but does not execute monetary transactions or hold payment credentials.
- **No reverse coupling to scouting.** Transit remains an independent transport search and persistence
  layer. Scouting consumes transit routes; transit never imports scouting internals.

## Build

```bash
cargo test -p sjel-transit
cargo build --locked --release --bin transit-server
```
