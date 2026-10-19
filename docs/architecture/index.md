# Architecture overview

Sjel separates the runtime, domain services, and agent workflows. The
[generated architecture](../../ARCHITECTURE.md) lists their declared wiring; the
[engineering doctrine](../../CONTRIBUTING.md#architecture-and-ownership) defines placement.

## Architectural nouns

### 1. The Spine

The spine contains root manifests, `schemas/`, operator tools in `tools/`, shared libraries in
`libs/`, and the `dashboard/` shell. The dashboard presents capability surfaces and owns no domain
state. Its [manifest](../../dashboard/service.toml) declares its service configuration.

### 2. Capabilities

Capabilities own domain behavior under `capabilities/<name>/`. A capability may begin as a README;
its `service.toml` declares runtime and backup contracts when needed. Some run a process or
container, while a data capability owns state without running a service. Private capabilities
may also live in the active overlay.
See the [manifest guide](../capabilities/manifest.md).

### 3. Packs

Packs contain portable agent skills and workflows under `Packs/<name>/`. Their `pack.toml`
manifests declare the capabilities and external systems they use. See the
[Pack schema](../../schemas/pack.toml.example) and [Pack directory](../../Packs/).

## Boundary invariants

- Shared contracts belong in `schemas/`; reusable implementation belongs in `libs/`.
- Capabilities use another capability's declared interface rather than mutating its tables.
  A shared database supports cross-domain reads without transferring table ownership.
- Public code and schemas stay in this repository. Deployment configuration and private data
  stay in the active overlay; secrets follow the [security boundary](security.md).

Read [storage](storage.md) before changing a store, [security](security.md) before changing a
listener or authentication policy, and [projects UI](projects-ui.md) for projects, figure embeds,
and generative UI.
