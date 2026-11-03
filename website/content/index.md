# Sjel technical documentation

Choose a guide by the task you want to complete. Repository doctrine lives in
[CONTRIBUTING.md](../CONTRIBUTING.md#engineering-doctrine); the current declared structure
is generated in [ARCHITECTURE.md](../ARCHITECTURE.md).

## Reading paths

| Task | Start here | Then read |
| --- | --- | --- |
| Prepare a release or a later friend trial | [Release preparation](operations/index.md#release-preparation-and-later-testing) | [Open acceptance work (#235)](https://github.com/larsboes/Sjel/issues/235) |
| Install and verify a minimal shell | [Shell-only node](operations/index.md#install-a-shell-only-node) | [Footprint](operations/index.md#installation-footprint), [service persistence](operations/persistence.md) |
| Understand where a component belongs | [Architecture overview](architecture/index.md) | [Storage](architecture/storage.md), [data classes and security](architecture/security.md) |
| Design the optional feature manager | [Feature manager](architecture/feature-manager.md) | [Release preparation](operations/index.md#release-preparation-and-later-testing) |
| Add or change a capability | [Capability manifests](capabilities/manifest.md) | [Health and freshness contracts](capabilities/contracts.md), the capability's manual |
| Diagnose or manage a host | [Operations guide](operations/index.md) | [Doctor](operations/doctor.md), [patch policy](operations/index.md#patch-first), [service persistence](operations/persistence.md) |
| Projects & Generative UI | [Projects UI](architecture/projects-ui.md) | [Engineering doctrine](../CONTRIBUTING.md#product-rules) |
| Rehearse recovery before a migration | [Storage and recovery](architecture/storage.md#backups-before-migrations) | The owning capability's backup contract |

## Documentation sections

- [Architecture](architecture/index.md): component ownership, storage, and access boundaries.
- [Capabilities](capabilities/index.md): manifest reference and domain manuals.
- [Operations](operations/index.md): diagnosis and service lifecycle procedures.

Read the owning capability or Pack README for its purpose and design choices. Order, open work and
decisions not yet built live in [PLAN.md](../PLAN.md) and [docs/plans/](plans/). Use Sjel tools for current ports, enabled services, and
health; a documentation page cannot establish that a service is running.
