# Vendored basemap assets

Everything in this directory is fetched from OpenFreeMap by `tools/fetch-basemap` and committed
so the map's fixed half is served from this shell's own origin. Nothing here is authored in this
repository, and nothing here is data about the operator. Run `tools/fetch-basemap` to refresh it;
`tools/fetch-basemap --check` asserts the set is complete.

The adoption verdict lives in `upstreams.toml` under `[openfreemap]`.

| Path | Upstream | Licence |
| --- | --- | --- |
| `style.json` | <https://tiles.openfreemap.org/styles/liberty> | OpenFreeMap server MIT; the style descends from OpenMapTiles' Liberty (BSD-3-Clause) |
| `sprite/ofm*.{json,png}` | <https://tiles.openfreemap.org/sprites/ofm_f384/> | as above |
| `fonts/Noto Sans */*.pbf` | <https://tiles.openfreemap.org/fonts/> | Noto Sans, SIL Open Font License 1.1 — the same licence `../fonts/OFL.txt` carries for IBM Plex |

The rendered map is data from **OpenStreetMap** (ODbL) served through **OpenMapTiles**. That
attribution is a licence obligation, not decoration: it is carried in `style.json`'s
`sources.openmaptiles.attribution`, which is where `tools/fetch-basemap` copies it when it inlines
the TileJSON, and MapLibre's attribution control renders it on every map.

## What is not here

The vector tiles and the Natural Earth raster still come from `tiles.openfreemap.org` over the
network. They are the large half — a z6 vector tile measured 269 KB on 2026-09-06 — and they are
the half a self-hosted PMTiles deployment would replace. That decision is still open; see
`ISA.md`, "Not yet specified".

Two ranges of each fontstack are vendored (`0-255`, `256-511`), which covers every Latin-script
place name. A label outside those ranges falls back to the upstream host through the surface
module's `transformRequest` — slower, and correct. `manifest.json` lists exactly what is here and
is what the browser reads to decide.
