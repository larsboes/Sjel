# sjel-http

One home for **outbound HTTP**: the blocking client, the user-agent every Axon
request carries, and the timeout no caller can forget.

Before this crate, 33 call sites across nine capabilities and two libs each built
their own `reqwest::blocking::Client`. Reading all 33 turned up three things no
single site showed:

- **Seven had no timeout at all** — `scouting/src/http.rs`, three scouting
  adapters, `scouting/src/calendar_promote.rs`, `comms/src/google.rs` and
  `calendar/src/google_sync/auth.rs`. A `reqwest` client with no timeout waits as
  long as the socket stays open, which is how a scheduled job becomes a stuck
  process.
- **The user-agent disagreed with itself** — six spellings, two of them pointing
  at the GitHub Pages site rather than the repository, and nothing at all in a
  dozen others.
- **Every call rebuilt the client** — a TLS configuration, a connection pool, a
  background thread and a current-thread runtime, thrown away after one request.
  `libs/inference` paid that on every probe, rerank and embedding call.

## Using it

```rust
let client = sjel_http::client(
    sjel_http::Purpose::new("places-geocode"),
    Duration::from_secs(20),
)?;
```

`client(purpose, timeout)` is the whole common case. The timeout is an argument
rather than a default, so there is nothing to forget. The result is cached per
`(purpose, timeout)` and `reqwest::blocking::Client` is a handle over shared
state, so repeated calls share one pool, one TLS configuration and one thread.

`builder(purpose, timeout)` returns the same thing unbuilt, for the five sites
that need an option this crate does not decide: gzip, a cookie store, a redirect
policy, and `scouting/adapters/meetup.rs`, which overrides the user-agent with a
browser string because the site refuses anything else.

The agent is `Axon-<purpose>/<version> (+https://github.com/larsboes/Sjel)`. The
version is this crate's, which is the workspace version, so a server sees one
product rather than nine. The URL is the intended remote, not a live one — Axon
has no public GitHub remote yet (PROJECTS.md).

**Not for use from an async runtime worker.** A blocking client driven from inside
a Tokio worker panics at run time. Every caller in Axon is either a CLI or inside
`tokio::task::spawn_blocking`, whose threads are not runtime workers.

## Why its own crate

`libs/sjel-config` was the alternative and is the wrong host: `host-net`,
`interior`, `soundscape`, `vault` and `tools/storage` depend on sjel-config and
make no outbound request, and `reqwest` brings a TLS stack with it. A lib here is
spine-owned shared code with no domain of its own (ARCHITECTURE.md, "Libs"), and
"how Axon talks to the network" is exactly that.

Consumers: `calendar`, `comms`, `finance`, `places`, `punctuality`, `scouting`,
`transit`, `trips`, `libs/inference`, `libs/summarize`.
