# The demo stack

A running copy of Sjel, filled with data that was never real, for screenshots of the app. It is
not published: the public site (`website/`) is built from tracked files and runs nothing.

```
tools/demo-up            start the stack and seed it
tools/demo-origin        the synthetic remote, started by demo-up (--once prints its payloads)
tools/demo-shots         run the stack in a container and screenshot the GPUI app against it
                         (also `sjel demo up | app [page] | mac <page> | down`)
```

Dates are relative to `[demo] anchor`. `"today"` means the run's local date, so the app shows
upcoming trips and recent mail. `SJEL_DEMO_ANCHOR=YYYY-MM-DD` pins it for a reproducible run.

## How the data gets there

No capability crate contains a line of demo code. `tools/demo-seed` POSTs generated entities
through each capability's own HTTP write API. Every value descends from `[demo] seed` through
the PRNG in `tools/lib/demo-data.ts`.

Three capabilities have no write API, because their job is to fetch something: Comms collects,
Scouting scans, Transit queries a timetable. `tools/demo-origin` is a plain HTTP origin serving
an RSS feed, article pages and bahn.de-shaped journey payloads at `[demo] origin`. Comms is fed
by posting URLs to its ordinary `/ingest` route, Scouting by a generated source declaration, and
Transit by the endpoint overrides its HAFAS client already has.

`demo.toml` declares which capabilities are seeded (`[capability.*]`) and which are not, with the
reason (`[absent.*]`).
