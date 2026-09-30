# vaultwarden

Reusable Vaultwarden runtime contract. Sjel owns the pinned image, lifecycle integration, public
environment template, storage contract, and coherent backup mechanics. The active overlay owns
users, network exposure, certificates, signup policy, secrets, data, and recovery evidence.

## Safe public default

The public service binds to loopback. A deployment that needs remote access must declare its
private transport and port override in the owning overlay. Do not widen the public manifest to
encode one host's LAN, Tailnet, certificate, or reverse-proxy design.

```sh
tools/service-runner.sh start vaultwarden
tools/service-runner.sh status vaultwarden
tools/service-runner.sh stop vaultwarden
tools/service-runner.sh resume vaultwarden
```

## Configuration boundary

- `vaultwarden.env.example` documents public keys and neutral defaults.
- `<overlay>/config/vaultwarden.env` contains deployment values and secret references.
- `<overlay>/data/vaultwarden/` contains the private database, attachments, and certificate data.
- User enrollment and master-password operations remain explicit human actions.

Vault clients require a secure context. The selected overlay owns how TLS and authenticated remote
access are provided; Sjel does not publish a host-specific certificate or access recipe.

## Recovery boundary

The backup contract takes a coherent cold SQLite copy while the capability is held, resumes the
service before shipment, and supports additive no-prune recovery rehearsals. Actual archives,
receipts, destinations, hashes, and restore evidence remain private to the deployment that runs
Vaultwarden.

For a manually pulled backup, `tools/backup.sh --stream vaultwarden` writes archive bytes only to
stdout and diagnostics to stderr. The receiving host must pipe stdout directly into an encrypted
repository; writing it to a normal file creates a plaintext backup and defeats the mode. Stream
mode performs no destination lookup, remote retention, or receipt write on the source host.

## Attribution

Vaultwarden is adopted as a pinned upstream in `upstreams.toml`. Public security gates own image
and dependency review; the owning overlay owns operational rollout and recovery proof.
