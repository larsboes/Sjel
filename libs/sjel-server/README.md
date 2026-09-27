# sjel-server

The one way a capability server comes up: `resolve_port` (re-exported from
`sjel-config`: `SJEL_PORT` from the runner first, capability escape hatch second, config
third, shipped default last), a loopback-only bind, **the inbound authentication gate**,
uniform startup logging, and a named single-line exit on bind failure instead of a panic
backtrace.

## The inbound gate

| Configured token | `/health`, `/ready`, CORS preflight | Every other route | Reach beyond loopback |
|---|---|---|---|
| yes | served | `401` without a matching token | permitted |
| no | served | served (or `403`, see below) | **refused at bind** |

A token is presented as `Authorization: Bearer <token>` or `X-Axon-Token: <token>`, and
compared byte-by-byte in constant time. Two header forms because two kinds of client
call these ports: proxies and HTTP tooling that already speak `Authorization`, and the
browser extension and `curl` callers for which a dedicated header is one fewer thing to
get wrong.

`/health` and `/ready` answer before the gate. They are what the runner, the dashboard
proxy and axon-status poll to find out whether a process is alive; behind a token they
would report a healthy capability as down, and their answer carries nothing a caller
could not learn by observing that the port accepts a connection. `/routes` is **not**
exempt: a route manifest describes the surface, which is not liveness.

`InboundAuth::refuse_without_token()` closes the non-exempt routes with `403` instead of
serving them when no token is configured. comms is the reason it exists: `POST /ingest`
fetches an attacker-chosen URL, and a page open in the operator's own browser is already
inside the loopback boundary, so `127.0.0.1` was never what contained that route.

### The tailnet identity gate

A shared secret cannot reach the caller the tailnet exists to serve. A browser on the
phone loads the built SPA and issues relative fetches; giving that page the token means
shipping the deployment's secret into a bundle and every cache that touches it. So the
phone gets in on an identity instead.

`<overlay>/config/deployment.env` declares `SJEL_TAILNET_OPERATOR=<login>` — a **value**,
not a file reference, because a login is not a credential. It appears in the tailnet admin
console, in `tailscale status`, and in the header of every request that arrives.

| Declared operator | `Tailscale-User-Login` | Outcome |
|---|---|---|
| no | anything | header ignored, the token rule alone decides |
| yes | absent | the token rule alone decides — a direct loopback caller |
| yes | the operator | served, without a token |
| yes | anyone else | `401` |

**Why the header can be trusted: the proxy overwrites it.** Measured against tailscale
1.102.3 on 2026-09-06 — a request carrying `Tailscale-User-Login: attacker@evil.example`
and `X-Forwarded-For: 9.9.9.9` reached the backend as the authenticated node's own login
and 100.x address. A client cannot inject an identity through `tailscale serve`; it can
only fail to have one.

**What it does not do.** It does not change the loopback trust model — a process on this
machine can write any header, so the identity means something only for requests the proxy
created, and a local process already reaches `127.0.0.1:<port>` directly. And it never
satisfies `refuse_without_token`: a route that opts into that wants the secret, not a
name.

**The failure it is exposed to, and what catches it.** Reconfigure `tailscale serve` as a
raw TCP forward and no identity header is injected, every tailnet request becomes
indistinguishable from a loopback one, and this gate silently stops gating with every
process healthy and every test green. doctor's **Tailnet identity gate** section fails on
exactly that shape, and on funnel being on at all (PRD N3).

### Token sourcing: one token for the deployment

`<overlay>/config/deployment.env` declares `SJEL_INBOUND_TOKEN_FILE=<path>` and the token
is that private file's contents (`schemas/deployment.env.example`). A reference, not a
value, following the pattern comms established for `api_secret_file` — a path is not a
secret, which is why it may live in a tracked-shape file.

Shared rather than per-capability because it gates one thing: whether an inbound request
reached this machine legitimately. Twelve tokens would be twelve secrets for one boundary
and twelve injections in every client that fans out across capabilities — the dashboard's
Vite proxy and axon-status' `/routes` aggregation both do exactly that.

A capability may still pass its own token to `InboundAuth::resolve`, and it wins. comms'
`api_secret_file` is the one caller that does, because the browser extension, `axon-clip`
and the dashboard proxy already hold that value. A deployment converges the two by
pointing both references at one file.

### Why the loopback rule is now a type

`bind_addr_for(Reach::AllInterfaces, port, auth)` returns `Err` when `auth` carries no
token. It is the only constructor of a non-loopback `SocketAddr` in this crate, so
"served beyond this machine without authentication" has no value a caller can obtain
and then use. That is the half this crate can enforce; the other half is below.

## Why this exists

Server binaries carried the same ~10 startup lines with three divergences none of which
was a decision: three bound `0.0.0.0` while the others argued `127.0.0.1` in a comment,
punctuality exited cleanly on a bind failure while the rest panicked, and comms had
stopped honouring the runner's port contract.

The last of those, `scout-server`, mattered more than the tidiness: it bound `0.0.0.0`
with permissive CORS in front of a mutating `POST /opportunities/:id/status`, so any
device on the LAN could write opportunity state without auth.

The gate arrived for the same reason one level up. Exactly one of twelve Rust
capabilities authenticated an inbound request — comms, on its mutating routes only. The
other eleven treated the loopback bind as the whole boundary, axon-status among them,
which serves `POST /api/axon-status/capabilities/:name/start|stop`: process control.
"Reachable from the phone" and "unauthenticated process control" cannot both be true, so
the check belongs in the crate all twelve already route their startup through rather
than in twelve copies that drift.

CORS is deliberately not in here. Whether a server carries `CorsLayer::permissive()` is
a per-capability security decision that stays visible in that capability's source —
axon-status, which can start and stop the machine's capabilities, correctly carries
none.

## The browser-origin refusal (`origin`)

A second, narrower gate, for the capabilities that serve C2 data to a browser.
`origin::refuse_foreign_origins` answers **403** when a request carries an `Origin`
header this deployment does not serve the dashboard from. Refusing the request — rather
than merely omitting CORS response headers — is what also stops a hostile page's
"simple" cross-site POST, which a browser sends before it reads any response header.

A request with **no** `Origin` passes. That is not a hole: it is how every
server-to-server caller works (curl, the runner's probes, `capabilities/calendar`'s POST
into trips), and a browser always sends one on a cross-origin request.

| | |
|---|---|
| Allowed by default | `localhost`, `127.0.0.1`, `[::1]`, any `*.ts.net` host |
| Env var | `SJEL_<CAPABILITY>_ALLOWED_ORIGIN_HOSTS` — comma-separated exact hosts, which **replaces** the `.ts.net` suffix and closes the Tailscale Funnel gap |
| Applied by | `places` (the companion register, README D4 / ISA PLC-7) and, since 2026-09-05, `trips` (the plan-search body carries the operator's feasible windows and a companion hint; its router had ended in `CorsLayer::permissive()`, which made every route above it readable cross-origin) |

```rust
.layer(axum::middleware::from_fn_with_state(
    "places",
    sjel_server::origin::refuse_foreign_origins,
))
```

**axum applies a layer only to routes registered before it**: "Additional routes added
after `layer` is called will not have the middleware added" (axum 0.7,
`src/docs/routing/layer.md`). A route appended below that call silently loses the
refusal, and a test that exercises `origin_allowed_by` alone still passes. Each consumer
therefore drives its **wired** `Router` with a foreign `Origin` in its own test —
`places::server::tests::a_foreign_origin_cannot_read_people_presence` and
`trips::server::origin_tests::a_foreign_origin_cannot_read_a_plan_search_result`.

This module was moved out of `capabilities/places/src/server.rs` on 2026-09-05, when a
second capability needed it (PRD Q91). A second copy of a security predicate is drift; one
home is the point. Applying it to the whole trips router closed an existing leak as a side
effect: `GET /api/flights/when` had been serving calendar entry titles cross-origin.

## What actually enforces this

`serve_local` alone enforces nothing: a server that ignores it and builds its own
listener compiles fine, and so does one that skips the gate by never calling this crate.
The check that makes both policies real is doctor's **Server bind policy** section, which
fails when any `capabilities/*/src/*.rs` that builds a `Router`
also constructs its own `axum::serve` or `TcpListener::bind`. It lives in doctor rather
than a repo gate because half the servers it has to cover are in the overlay, outside this
repo, and a gate that globs Axon alone would report a clean policy while an overlay server
binds the LAN (README.md#documentation-stays-owned-and-current, same reasoning as the decision path-rot sweep).

The identity gate's other half is not in this repository at all: it is the shape of
`tailscale serve` on the host. doctor's **Tailnet identity gate** section is what reads
it, because a declaration whose truth lives outside the tree is the one that rots
unobserved (PRD §13, the pattern recorded four times).

## Build boundary

This is a normal workspace crate. Consumers declare an `sjel-server` path dependency,
and the one root `Cargo.lock` keeps the `axum::Router` type identical across the library
and every consumer. `cargo tree` is what exposes the architectural edge.

## Consumers

Every capability server. Measured 2026-09-06, twelve capabilities declare the path
dependency: `comms-server`, `calendar-server`, `axon-status`, `finance-server`,
`interior`, `places-server`, `soundscape`, `transit-server`, `punctuality-server`,
`scout-server`, `trips-server` and `vault-server`. `cargo tree` is the current answer;
this list is a snapshot.
