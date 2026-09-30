---
name: homectl
description: Materializes overlay-owned Home Assistant templates and vendors overlay-owned pinned components through a public, host-neutral CLI. Use for template coverage, materialization, or reviewed component vendoring. Do not use for validating or reloading a deployment already placed (use ha-deploy), for querying or controlling entities (use ha-cli), or for generating dashboards (use ha-dashgen). Real devices, automations, entity IDs, component selections, and output stay in the active private overlay.
allowed-tools: Read, Write, Edit, Bash
---

# homectl

Operate the public materialization mechanism over a private Home Assistant definition. Sjel owns
the parser, validation, and vendoring behavior. The selected overlay owns the templates, variable
map, component lockfile, helpers, and generated output.

## Commands

```sh
scripts/homectl doctor
scripts/homectl materialize
scripts/homectl components [--only NAME] [--dry-run]
```

Defaults resolve from `SJEL_OVERLAY_ROOT`, `SJEL_HOME_ROOT`, or the overlay declared by
`axon.local.toml`:

- capability input: `<overlay>/capabilities/home-assistant`
- variables: `<overlay>/config/home-assistant.vars`
- generated output: `<overlay>/build/home-assistant`
- component destination: `<overlay>/config/home-assistant`

Every path remains overridable for isolated tests.

## Boundary

- Never point generated output into public Sjel.
- Never place a real entity ID, hostname, component selection, or device name in this Pack.
- Treat the set of templates and components as a private inventory even when their contents use
  placeholders and public Git references.
- Review component diffs before replacement; runtime fetching is not allowed.

Read `references/templating.md` for the token contract and
`references/automation-doctrine.md` for reusable reliability rules.
