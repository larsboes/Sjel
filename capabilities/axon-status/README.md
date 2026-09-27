

# axon-status

What this machine has enabled, what is up, and the one thing allowed to start it.
Not LifeOS Pulse (`~/.claude/LIFEOS/PULSE/pulse.ts`, port `31337`); see
"Why this shape: the name and the port" below for why this is a
separate capability with its own name and port.

It knows no capability names. `tools/capability.sh registry` renders the
`service.toml` manifests as JSON and this process reads that, so
`tools/lib/toml.sh` stays the only TOML parser (README.md#one-manifest-per-concern) and a port
literal lives in exactly one file.

`src/main.rs` is the composition root for configuration, route assembly, bind,
and shutdown. The `src/status/` modules own registry/cache invalidation, health
aggregation and HTTP projection, lifecycle commands, backup contracts/run
state, and idle-panel reaping. The reaper retains its task handle and stops
with the server instead of becoming detached process work.

## Port and binding

Default: `8082`. Resolution order: `SJEL_PORT` (exported by tools/service-runner.sh from
the manifest plus any machine override) — `SJEL_STATUS_PORT` (manual escape hatch when
running outside the runner) — `8082`.

Bound to **127.0.0.1**, not `0.0.0.0`: this process starts and stops the machine's
capabilities, so it answers to this machine only. The dashboard reaches it through
Vite's proxy, which runs here too.

The bind was the whole boundary until `libs/sjel-server` grew the inbound gate, and for
`POST /api/axon-status/capabilities/:name/start|stop` that was never enough on its own —
it is process control. When the deployment declares `SJEL_INBOUND_TOKEN_FILE`
(`schemas/deployment.env.example`), every route here except `/health` asks for that
token. `/api/axon-status/routes` then presents it when polling siblings, because a gated
capability that refused this process would otherwise be reported as not running.

## Endpoints

- `GET /health` — basic liveness check for axon-status itself
- `GET /api/axon-status/health` — `{ok, version, uptime_seconds, capabilities:
  {<name>: {up, url}}}`, one entry per enabled capability that declares a health
  surface, each backed by a live 2s-timeout `GET`. `ok` means the **autostart set**
  is up. Once capabilities start on demand, a stopped on-demand capability is the
  normal state, not a fault.
- `GET /api/axon-status/capabilities` — the registry plus live state: kind, scope,
  port, `requires`, `up` (`null` when nothing is declared to poll, meaning unknown
  rather than down) and `panel_url` for a capability that serves its own UI.
There was a `GET /api/axon-status/upstreams` here until 2026-08-28. It shelled out to
`tools/upstream-checker --json --offline` and served a per-entry `ok`/`na`/`warn`/`fail`
status. PRD Q41 retired that script; the status field *was* its verdict, so recomputing it
in this process would rebuild the plumbing the item deleted. Dependabot
(`.github/dependabot.yml`) reports drift on GitHub now, and the verdict half of that feed was never the checker's anyway —
`/self` already carries it, straight from `upstreams.toml` via `tools/self generate`.
- `POST /api/axon-status/capabilities/:name/start` — bring one up (via
  `service-runner.sh resume`, which also lifts a maintenance hold)
- `POST /api/axon-status/capabilities/:name/stop` — take one down and hold it

### Why a process-control endpoint is safe here, and when it stops being safe

The only reachable names are the ones the registry already lists, so a request names
a **capability**, never a command. `service-runner.sh` receives a name and looks up
what to run itself. That plus the 127.0.0.1 bind is the whole security model, and it
is sufficient only while the port stays local. Exposing this over Tailscale means
adding real authentication first: "reachable from the phone" and "unauthenticated
process control" cannot both be true.

## Running

Needs `SJEL_ROOT` set, because it shells out to the repo's own tools and its own path is
`target/release/`, which locates a build output rather than the checkout. `tools/lib/paths.sh`
exports it, so the normal path needs nothing:

```bash
tools/service-runner.sh start axon-status
```

## Why this shape: the name and the port

Migrated from its dissolved `decisions/` entry on 2026-07-28: this governs one
thing, so it lives with that thing (README.md#decisions-live-with-their-owner).

**Decision:** the health-aggregation capability for the root `dashboard` is `capabilities/axon-status`,
binary `axon-status`, default port `8082`. Not `pulse`, not `31337`.

**Why:** the shelved bulk-port attempt preserved in Git history named its capability `pulse` and
defaulted to port `31337` — both already belong to the real LifeOS
Life Dashboard (`~/.claude/LIFEOS/PULSE/pulse.ts`, documented everywhere in LifeOS's own
`CLAUDE.md`/system prompt as canonical). Two unrelated systems claiming the same name and port
is a collision waiting to happen the moment both run at once, and it was never a deliberate
migration — the shelved capability was three stub routes returning hardcoded static data
(`uptime_seconds: 0`, a fake `{"pulse": "up"}` capabilities map), not a real port of LifeOS
Pulse's actual functionality. `capabilities/axon-status` replaces it with real aggregation:
live `GET /health` calls out to every enabled capability with a health surface — read from
the registry, not a hand-list — reporting actual reachability, not a canned response. `8082`
sits next to scouting's `8081`... — actually next to nothing taken (`8081` was never real;
scouting's true default is `8084`, since `8080` belongs to vaultwarden — see
`dashboard/README.md`) — chosen simply as the next free, unclaimed port.

**Forecloses:** no Axon capability may claim port `31337` or the name "Pulse"/"pulse" — that
identity belongs to LifeOS, whether or not any machine still runs it. The root `dashboard`
proxied LifeOS Pulse as a distinct upstream from `axon-status`, never conflated into one
capability; that proxy and the panel behind it were deleted on 2026-08-25 with the rest of
the LifeOS delta (PRD D6). A reader who wants that data again adds it back as an upstream,
not as a route on this capability.

## The SPA fallback stops at the build's own directory

This capability serves the dashboard bundle, and `adapter-static` emits one entry point that
routes client-side — so an unknown *path* is a client route, and answering it with `index.html`
is correct. It is not correct for a **build artifact**, and the difference is not cosmetic.

`/_app/` is SvelteKit's build-output root: every URL under it is generated and content-hashed by
the build, and none of it is a page. A miss there is now a **404** (`is_build_output` in
`src/proxy.rs`), because the alternative is a failure nobody can see. Measured 2026-09-06: Vite
never emitted MapLibre's worker, the browser asked for
`/_app/immutable/chunks/maplibre-gl-worker.mjs`, this server answered **200 `text/html`** with the
app shell, and `new Worker` was handed HTML and died silently. Every map in the dashboard rendered
a blank canvas and sat on "Loading map…" — for a month, with no error and no failed request in the
network log. A 404 would have named it in seconds.

The rule is deliberately narrow: one directory the build owns outright, not "any path with a file
extension", which would break a client route containing a dot. A related trap is documented at
`fallback` in `src/proxy.rs` — a capability path with no route also lands on the shell and answers
HTML, which is why a capability endpoint returning HTML means "restart axon-status" rather than
"the endpoint is healthy".

## Considered and declined

- **A `/*path` catch-all 404 handler** — the shelved first attempt had one
  returning a hint JSON payload. Axum's default 404 already covers this;
  the handler added nothing a real client needs. Dropped.
- **Parsing `service.toml` in Rust** — would have meant a second TOML parser
  against README.md#one-manifest-per-concern, and a `toml` crate as a new upstream for facts the shell
  already reads. The registry subcommand emits JSON instead; `serde_json` was
  already a dependency.
- ~~**A generic capability list read from config**~~ — declined on 2026-07-18 while
  only two fixed targets needed aggregating. Additional capability consumers and the
  dashboard proxy met the decision's own flip condition, so the hardcoded
  transit/scouting literals are gone.
