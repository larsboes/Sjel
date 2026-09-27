# sjel-config

Shared overlay/config resolution for Axon's Rust capabilities: tilde expansion, overlay
paths (`SJEL_PERSONAL_ROOT`), the store's location, the deployment's home timezone, and the
runner's port contract (`SJEL_PORT` first, capability escape hatch second, config file
third, shipped default last).

## Where the store lives

`database_path()` — `SJEL_DB_PATH`, else `<overlay>/data/axon/axon.db`, else a scratch file
under the temp directory. One file for every capability after PRD Q45 (2026-08-27), so it
takes no capability argument: cross-capability joins are why the shared instance existed,
and a file per capability would have dropped them. The last resort is deliberately obvious
scratch, because the Postgres fallback it replaces named the real database and the demo
overlay resolved straight to it.

`postgres_conn_from_shared_env()`, `database_url_override()` and `redact_dsn()` are gone.
They went with their last consumer: the six capabilities that still held a DSN moved to the
shared file, and a path is not a credential, so there is nothing left to build or to mask.

## Why this exists

transit/config.rs once argued the duplication was cheaper than a shared crate, and at two
copies it was. By five capabilities the repo held six copies of `expand_tilde` and five
shared-Postgres DSN builders in two diverging forms; three crates still built the
`postgresql://user:password@…` URL form that comms had already documented as an auth trap
(the instance's real password was base64 and could contain `/`, `+`, `=`, which URL userinfo
silently mangles). One implementation ended both the drift and the divergence — and then
PRD Q45 ended the DSN itself, which is the version of that argument that finally holds:
`database_path()` has one caller shape and nothing to diverge about.

Zero external dependencies on purpose: std-only, so consuming it never changes a
capability's dependency resolution, and it needs no crate universe of its own.

## Consumers

Every Rust capability uses this directly or through the server helper. Consumers
declare the workspace path dependency in Cargo. The library is compiled once per
build, so its API is an ordinary crate boundary rather than copied source.
