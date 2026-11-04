# Architecture and ownership

Use this reference only for Sjel-wide structure or ownership decisions. Derive the current unit
inventory and wiring with `tools/axon-context on [target]` and `tools/self`; do not copy those
facts here.

## Stable model

Sjel has three architectural nouns:

- **Spine**: repository identity, contracts, shared libraries, tools, and the native GPUI client.
- **Capability**: one bounded domain, external system, or data store under `crates/` or `services/`.
- **Pack**: public, harness-neutral agent know-how that drives capabilities through contracts.

The public repository owns reusable code and doctrine. Exactly one active private overlay owns
the selected personal, work, or family deployment state. A physical instance is not a reason to
fork public capability code.

## Dependency boundaries

- Capabilities depend downward on shared schemas and libraries.
- Cross-capability runtime use goes through a declared service contract, not a source-code import.
- Promote shared code to libraries only when it has multiple consumers and owns no domain.
- Keep the UI presentation-only; domain state and behavior remain capability-owned.
- Prefer integrating a self-authored project unless product identity, collaboration, device-sync,
  or overlay ownership gives it an independent lifecycle.

Read the root `README.md` for the human-facing architecture and generated `ARCHITECTURE.md` for
the manifest-derived current graph. Never hand-edit the generated file.
