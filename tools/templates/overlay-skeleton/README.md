# axon-__OVERLAY_NAME__

The private overlay of [Sjel](../Sjel/) for this ownership boundary
(`__OVERLAY_NAME__` — one of personal / work / family). Everything private for this
boundary lives here; nothing here is ever public. Sjel holds the harness; this repo
holds the values.

| Path | Holds |
|---|---|
| `skills/` | private agent skills (override/extend Sjel-shipped ones; overlay wins) |
| `plugins/` | private plugins and patches on upstreams |
| `config/` | private config values rendered into tools (dotfiles values, env, paths) |
| `config/claude-code/` | this deployment's additions to Sjel's managed Claude Code policy — additive only, see the `.example` |
| `documents/` | personal documents |
| `memory/` | agent memory |
| `data/<capability>/` | per-capability databases and state (trips, finance, ...) |
| `secrets/` | references into the secrets store, never plaintext |
| `backup/` | backup configuration; targets off-machine (3-2-1) |

Rules: this repo never gets a public remote; large binaries go through the backup tool, not
git; every directory here is reachable by Sjel only via declared injection contracts.
