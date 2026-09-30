# home-assistant

Reusable Home Assistant runtime contract. Sjel owns the pinned container definition, public
configuration shape, lifecycle integration, and generic operator tooling. It does not own a
home's devices, entity IDs, automations, helpers, custom components, dashboards, or evidence.

## Public boundary

- `service.toml` declares the host-portable runtime and backup contract.
- `home-assistant.env.example` documents the non-secret connection shape.
- `Packs/home-automation/` provides generic API, materialization, validation, and deployment
  mechanisms.

The selected private overlay owns all instance material, including templates, component pins,
host placement, and Home Assistant state.

## Runtime

The public manifest is intentionally instance-free. The active overlay selects the enabled host,
ports, paths, environment values, backup target, and access policy. Secrets are resolved through
the private secret boundary and never stored here.

Use the shared lifecycle commands:

```sh
tools/service-runner.sh start home-assistant
tools/service-runner.sh status home-assistant
tools/service-runner.sh stop home-assistant
tools/service-runner.sh resume home-assistant
```

## Automation and component workflows

`homectl` reads templates, variables, helpers, and component pins from the active overlay. The
token grammar and reliability doctrine live with that public Pack workflow, while the files that
describe a residence remain private.

## Attribution

Home Assistant Core is adopted as a pinned upstream in `upstreams.toml`. Instance integrations
and packages are declared and reviewed in the owning overlay because their set is a device
inventory even when every individual source is public.
