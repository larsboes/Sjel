# Data, secrets, and trust boundaries

Read this before any task involving private state, credentials, external code, or a write across
capability boundaries.

## Data ownership

- Keep reusable code, schemas, doctrine, and reviewed public data in Sjel.
- Keep machine configuration and personal state in the active overlay resolved by
  `tools/lib/paths.sh`.
- Preserve the data classes `c0`/`c1`/`c2`/`c3`. `c0` and `c1` may leave the host; `c2` stays
  local-only; `c3` never enters a prompt.
- A capability owns its records. Cross-capability writes require an explicit reviewed contract
  with provenance and idempotency where retries are possible.

## Secrets

- Store secret values in Vaultwarden. Repository and overlay notes may contain references, never
  values.
- Do not generate, paste, rotate, or expose a secret on a general continuation instruction.
- `tools/setup-secret.sh` requires the user to run the interactive operation after specific
  authorization.
- Treat command output, logs, fixtures, screenshots, and GitHub text as possible disclosure
  surfaces.

## External trust

- Record an external dependency and verdict in `upstreams.toml` before consuming it.
- Preserve its license, provenance, and adopted influence. The register records no version.
- Run the manifest gate and content audit through the repository tools rather than invoking
  scanners ad hoc.
- An installed connector or plugin is not proof of authentication or private-repository access.
