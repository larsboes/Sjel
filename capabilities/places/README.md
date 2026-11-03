# places

Canonical place registry, geocode cache and geographic layers shared by Finance, Trips, Transit and Entities.

## Why this shape: one place identity

A shared registry gives capabilities one durable place identity and one geocoding cache. Consumers can link to a place instead of maintaining their own partial coordinates and names. Places owns writes to its tables and reads the source data it needs from other capabilities.

Geocoding sends place text or a coordinate pair only. It does not send person names, amounts or dates. Responses are cached locally so repeated queries do not leave the machine again.

The companion register is owned here because its proposals, confirmations and disclosure rules are geographic. Confirmed rows remain here; planning consumers receive bounded aggregates rather than register rows.

## Run

The service is managed through `tools/service-runner.sh` and its `service.toml` manifest. The live HTTP contract is available at `GET /routes`.

One-shot imports and refreshes use the `places` CLI:

```sh
places backfill amex
places backfill cities
places backfill stations
places backfill calendar
places backfill takeout
places backfill travelers
places backfill vault
places climate fetch
```

## Considered and declined

- **No PostGIS.** The registry's geographic queries use bounded coordinate sets and distance calculations; a spatial database extension is not required.
- **No GPS trace.** Travel history is assembled from plans, transit legs and other explicit records. Places does not track a device.
- **No photo storage.** Media remains in its original library. Places serves indexed geotags without copying photo bytes.
