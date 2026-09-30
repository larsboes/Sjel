# knowledge-graph

REST API over the Sjel code-dependency graph built by
[graphify](https://github.com/safishamsi/graphify).

## What it serves

| Endpoint | Description |
|---|---|
| `GET /api/graph` | Full graph (nodes + edges) |
| `GET /api/graph/stats` | Summary stats (node/edge/community counts) |
| `GET /api/graph/search?q=<term>` | Search nodes by label or file type |
| `GET /api/graph/community/<id>` | Nodes and edges in one community |
| `GET /api/graph/node/<id>` | One node with its connections |
| `GET /api/graph/unit/<name>` | One unit's files and the edges among them, capped |

`/api/graph/unit/<name>` is the drill-down the dashboard uses. It maps a unit name
onto the repo-relative prefixes that unit owns — the same mapping
`tools/lib/self-model.ts` uses to name units in the first place, so the two cannot
disagree about what `comms` means — then returns its nodes ranked by degree. The
answer is capped and says so: `total`, `returned` and `truncated` are part of the
body, because a capped answer that looked complete would read as the whole unit.

## Where the browser view went

This capability used to serve a second SvelteKit app on `:4244` that drew the entire
graph — 6,645 nodes, 13,070 edges — into one vis-network canvas with `improvedLayout`
enabled, from a script loaded off unpkg. That layout never converged, so the page was
a black rectangle, and a local-first surface depended on a CDN being reachable.

The browser view is now the dashboard's self-model page (`/self`). It reads the
self-model first — ~38 units, 29 couplings, a graph small enough to be a picture —
and calls `/api/graph/unit/<name>` to go one level deeper into a single unit. Nothing
renders the whole graph, which is what makes it render at all.

That leaves this capability with the half that had no duplicate: the data. Agents and
scripts were always the API's other caller, and they still are.

## How it connects to graphify

`tools/graphify.sh` builds `graphify-out/graph.json`, `graph.html`, and
`GRAPH_REPORT.md` at the repo root. This capability reads `graph.json` live on every
API call — no data duplication, always the current graph.

Rebuild the graph with:

```bash
tools/graphify.sh
```

## Run

```bash
tools/service-runner.sh start knowledge-graph   # :4244
```

Directly, for development:

```bash
bun capabilities/knowledge-graph/server.ts   # serve on SJEL_PORT (default 4244)
```

It starts on demand: opening a unit on the dashboard's self-model page asks
sjel-status to bring it up, so a stopped knowledge-graph is the normal state rather
than a fault.

## Ownership boundary

This capability owns the serving layer only — the API and the service manifest.
graphify owns the extraction, community detection and graph data format. The
`graphify-out/` directory is gitignored and machine-specific (absolute paths in node
IDs); this capability reads it at runtime and never commits it.
