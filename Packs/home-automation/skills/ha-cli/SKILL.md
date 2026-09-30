---
name: ha-cli
description: Queries and controls an explicitly configured Home Assistant instance and can write a private inventory into the active overlay. Use for Home Assistant state, configuration, integration, service-call, reload, or inventory operations. Do not use for Lovelace dashboard deploys or backups (use ha-dashboard or ha-dashgen), for automation-template materialization and component vendoring (use homectl), or for router, DNS and LAN queries (use fritz, pihole or netmon). Never place returned instance data in public Sjel.
allowed-tools: Bash
---

# ha-cli

Generic Home Assistant REST tooling. Non-secret connection facts come from the active overlay;
the API token comes from the configured private secret mechanism at runtime.

## Commands

```sh
scripts/ha-cli config
scripts/ha-cli domains
scripts/ha-cli states [DOMAIN]
scripts/ha-cli get ENTITY_ID
scripts/ha-cli automations
scripts/ha-cli unavailable
scripts/ha-cli entries
scripts/ha-cli call DOMAIN SERVICE [JSON]
scripts/ha-cli reload-automations
scripts/ha-inventory [--overlay PATH] [--output PATH]
```

Read operations may reveal private entity IDs, locations, devices, URLs, and integration names.
Keep their output local. `ha-inventory` writes to the active overlay by default and must never
target the Sjel repository.

Service calls and reloads are writes. Confirm the target and requested action before invoking
them. A missing token or endpoint is an error; do not substitute remembered deployment facts.
