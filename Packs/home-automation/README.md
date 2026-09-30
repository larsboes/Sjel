# home-automation pack

Reusable operator workflows for Home Assistant and related private-network services. The Pack
contains generic mechanisms only; the active overlay supplies every residence fact, endpoint,
entity ID, automation, device inventory, component pin, and secret reference.

## Skills

- `homectl` materializes overlay-owned automation templates and vendors overlay-owned component
  pins.
- `ha-cli` queries and controls Home Assistant and can create a private overlay inventory.
- `ha-deploy` validates, reloads, and verifies an existing Home Assistant deployment.
- `ha-dashboard` and `energy-dashboard` operate Home Assistant presentation surfaces.
- `ha-dashgen` generates whole dashboards from an overlay-owned house model and deploys them.
- `fritz`, `netmon`, and `pihole` inspect explicitly configured private-network services.
- `esphome` operates an explicitly configured ESPHome environment without declaring one public
  deployment.

## Activate

```sh
"$SJEL_ROOT/tools/packs-codex" deploy home-automation
```

## Deployment status

Not deployed to any harness as of 2026-09-10 — deferred to the `home` profile decision (Phase 2).
The `home-assistant` capability it wraps is declared for this machine and the skill set is
harness-neutral, so this is a selection decision, not a build or wiring decision.

## Ownership boundary

Sjel owns these harness-neutral workflows. The active family overlay owns its host, household
devices, automations, network topology, deployed configuration, and recovery evidence. Another deployment
can reuse the same Pack with a different overlay without copying family state into Sjel.

External dependencies and adopted influences are recorded in `upstreams.toml`; private component
sets remain in the overlay that runs them.
