# Architecture overview

Sjel separates the runtime, domain services, and agent workflows. The
[generated architecture](../../ARCHITECTURE.md) lists their declared wiring; the
[engineering doctrine](../../CONTRIBUTING.md#architecture-and-ownership) defines placement.

## Architectural nouns

### 1. The Spine

The Spine contains root manifests, `schemas/`, operator tools in `tools/` and shared Rust crates
in `crates/`. The native client in [`apps/gpui`](../../apps/gpui/README.md) presents capability
surfaces and owns no domain state.

### 2. Capabilities

Rust capabilities live in `crates/<name>/`; supervised third-party software, scripts and
backup-only units live in `services/<name>/`. Private capabilities may live in the active overlay.
The [manifest guide](../capabilities/manifest.md) describes how `service.toml` declares runtime
and backup contracts.

### 3. Packs

Packs contain portable agent skills and workflows under `Packs/<name>/`. Their `pack.toml`
manifests declare the capabilities and external systems they use. See the
[Pack schema](../../schemas/pack.toml.example) and [Pack directory](../../Packs/).

## Boundary invariants

- Shared contracts belong in `schemas/`; reusable Rust implementation belongs in `crates/`.
- Capabilities use owner interfaces for cross-domain access. The [storage architecture](storage.md)
  describes the shared database that remains during migration.
- Public code and schemas stay in this repository. Private configuration and data stay outside
  the public checkout and follow the [security boundary](security.md).

Read [storage](storage.md) before changing a store, [security](security.md) before changing a
listener or authentication policy, [data processing policy](data-processing-policy.md) before
changing classification or model-provider gates, and [projects UI](projects-ui.md) for projects,
figure embeds, and generative UI.
