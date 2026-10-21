# libsql-auth

Mint the JWT a libSQL server (`sqld`) authenticates, and hold the Ed25519 pair it is signed with.

A shared library, not a capability: no domain of its own, no CLI.

## Why this is its own crate

It could live in [`sjel-turso`](../sjel-turso/README.md), which is where a replica is opened, and it
must not: `tools/sjel-cli` has to mint — `sjel setup` provisions a primary — and that binary already
links `sjel-store` (`rusqlite`). A binary that links both SQLite engines fails at link time, so the
minting cannot sit behind `sjel-turso`'s dependency graph. This crate carries `ring`, `base64` and
`serde_json` and nothing else.

## The claims are `sqld`'s, not a choice here

`libsql-server` decodes a JWT into `Token { id, a, p, exp }` with `jsonwebtoken` pinned to
`Algorithm::EdDSA` (tag `libsql-server-v0.24.32`):

| claim | meaning |
| --- | --- |
| `a` | `"ro"` or `"rw"` |
| `id` | a namespace; may be absent on a `--db-path` server |
| `p` | fine-grained scopes (unused here) |
| `exp` | unix seconds |

An absent `a` defaults to *write* upstream, which is why [`Access`] is never omitted.

## The two halves, and where each belongs

```rust
let key = PrimaryKey::generate()?;            // Ed25519, PKCS#8
std::fs::write(private_path, key.pkcs8())?;   // the OS keychain, not a file in the overlay
sqld.auth_key_file(&key.public_key_base64url()); // 32 bytes, base64url, no padding

let token = key.mint_for_days(None, Access::Write, 30)?;
```

The public half is what `sqld --auth-jwt-key-file` reads; it is not a secret. The private half is a
PKCS#8 document and belongs in the operating system's keychain — `sjel setup` keeps it in the macOS
Keychain and never writes it to the overlay, an argument vector or a log line. `Debug` prints the
public half only, because a panic message is a log line.

Regenerating the key invalidates every token ever minted, so nothing here does it implicitly.

## Measured

Against `sqld 0.24.32` on 2026-10-21, over the HTTP API:

| credential | `POST /v2/pipeline` |
| --- | --- |
| none | 401 |
| minted here (`a=rw`, `exp`) | 200 |
| signed by a different key | 401 |

and an embedded replica carrying a minted token pulls and writes end to end
(`libs/sjel-turso/tests/live_primary.rs`).

One thing this crate cannot fix and callers must: libsql's client reports a refused credential as
`Timeout performing handshake with primary`, so a wrong token looks like a network fault. `sjel setup
verify` probes over HTTP first for exactly that reason.

[`Access`]: src/lib.rs
