# libs/route-manifest

A self-describing HTTP surface. Spine-owned shared code with no domain of its
own — see [Three architectural nouns](../../CONTRIBUTING.md#three-architectural-nouns).

Every capability serves `GET /routes` beside `/health`, listing method, path and
a one-line summary. `sjel-status` fans out across the enabled capabilities and
aggregates them at `GET /api/sjel-status/routes`.

## Why this and not a rename

Sjel's HTTP surface carries five conventions across seven capabilities:

| Shape | Capabilities |
|---|---|
| `/api/…` behind an API-only proxy | `calendar`, `trips` |
| bare paths | `comms`, `scouting`, `punctuality` |
| self-prefixed `/api/<name>/…` | `soundscape`, `sjel-status` |
| both `/health` and `/api/health` | `transit` |

Converging those means renaming public routes and rewiring every caller — a
large blast radius for a modest payoff. The part that actually costs time is
**not being able to find out what exists**, and that is fixable without touching
a single existing path. Naming can converge later, incrementally, without a flag
day; this endpoint keeps working either way because each capability reports its
own paths rather than a convention anyone has to remember.

## Drift is the whole risk

A hand-maintained endpoint list is wrong the first time someone adds a route and
forgets it, and a stale manifest is worse than no manifest because it gets
believed. So the manifest is checked against the router that serves it:

```rust
#[test]
fn the_manifest_covers_every_served_route() {
    assert!(route_manifest::undeclared_routes(include_str!("server.rs"), ROUTES).is_empty());
}
```

`include_str!` reads the server's own source at compile time and the check
reports any path the router serves that the manifest does not mention. It is
text matching, not parsing: it cannot see a route built from a runtime string,
and it says so rather than claiming to be exhaustive. Every router in this repo
passes a literal.

The check is one-directional on purpose. A manifest entry with no matching
`.route()` is allowed — a capability may describe a path it mounts indirectly.
A *served* path nobody documented is the failure worth failing on.

## Consumers

Every Rust capability with an HTTP server.

Consumers declare the workspace path dependency in Cargo. The served JSON remains the
external contract; the crate gives every server one implementation of it.
