# Place Sjel changes

Choose the owner before choosing a directory.

| Change | Owner and location |
| --- | --- |
| Bounded domain, external system, or data store | `capabilities/<name>/` |
| Command shipped by one capability | That capability, named for the capability when user-facing |
| UI serving one capability | `capabilities/<name>/ui/` |
| Shared code with no domain and multiple consumers | `libs/<name>/` |
| Shared contract | `schemas/` |
| Repository identity, install, wiring, or operator machinery | Root manifests or `tools/` |
| Agent workflow over a capability | `Packs/<pack>/skills/<name>/` |
| External system identity | `systems.toml` |
| Host executable assumption | `toolchain.toml` |
| External code or adopted influence | `upstreams.toml` plus consuming README provenance |
| Machine or personal fact | Active private overlay |

Do not create a residual `utils/`, a grouping folder based only on deployment host, or another
private repository for one physical instance. When a proposed change fits no owner, revise the
boundary rather than inventing a fourth architectural noun.
