# places

Canonical place registry, geocode cache and geographic layers shared by Finance, Trips, Transit and Entities.

## Why this shape: one place identity

A shared registry gives capabilities one durable place identity and one geocoding cache. Consumers can link to a place instead of maintaining their own partial coordinates and names. Places owns writes to its tables and reads the source data it needs from other capabilities.

## Why this shape: place precision (D1)

Addresses are linked at venue precision when the source supports an address; other transactions use city precision. A city-kind registry entry remains city precision regardless of the requested precision.

## Why this shape: stable transaction links (D2)

Finance links use the journal-stable transaction `source_id`, so projection rebuilds do not change link identity.

## Why this shape: bounded geocoding (D3)

Geocoding sends place text or a coordinate pair only, never person names, amounts or dates. Responses are cached locally so repeated queries do not leave the machine again.

## Why this shape: companion register (D4)

The companion register is owned here because its proposals, confirmations and disclosure rules are geographic. Confirmed rows remain here; planning consumers receive bounded aggregates rather than register rows.

## Why this shape: climate normals (D5)

Monthly normals are stored here because the place registry owns durable coordinates and the permanent provider cache. The capability stores observations and computes month rankings from code.

## Run

The service is managed through `tools/service-runner.sh` and its `service.toml` manifest. The live HTTP contract is available at `GET /routes`.

### Backfills

One-shot imports and refreshes use the `places-server` CLI:

```sh
places-server backfill amex
places-server backfill cities
places-server backfill stations
places-server backfill calendar
places-server backfill takeout
places-server backfill travelers
places-server backfill vault
places-server climate fetch
```

## HTTP surface

The service publishes its route manifest at `GET /routes`; the listener port is declared in `service.toml`.

## Considered and declined

- **No PostGIS.** The registry's geographic queries use bounded coordinate sets and distance calculations; a spatial database extension is not required.
- **No GPS trace.** Travel history is assembled from plans, transit legs and other explicit records. Places does not track a device.
- **No photo storage.** Media remains in its original library. Places serves indexed geotags without copying photo bytes.
