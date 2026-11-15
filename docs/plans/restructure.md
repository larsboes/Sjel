# Restructure and sweep

[PLAN.md](../../PLAN.md#priority-1-unify-and-reduce-maintenance) owns priority.
Shared contracts and removal of repeated behavior precede directory moves.
Documentation rules live in
[on-documentation.md](../../Packs/harness/skills/sjel/references/on-documentation.md).

## Shipped

- Capability layout and service integration: `0ee6c884`; live path follow-up: `70ccbfa9`.
- Shared travel contracts and retirement of client shadow types: `cba9a059`.
- README sweep: `ec859210`, `cc44c287`; persisted-name cleanup: `71558dc0`.
- Documentation has one home under `docs/`: `91b7c987`.
- Cross-root maintenance coverage and reviewed dependency refresh: `9ec75e72`.

## Maintenance verification

`tools/maintenance --json --overlay --freshness` covers the configured overlay separately,
including npm workspace packages and private area plans. Missing requested analyzers in either
root produce exit 2. The command reads tracked inputs and does not update dependencies.

The CLI suite passes with serial test execution; CLI Clippy passes. Parallel execution
reported a file-lock test failure that passes in isolation and in the serial suite. Extraction tests pass with Xberg 1.4.0;
the graph-store benchmark compiles with Grafeo 0.6.2. Pi package checks, local adaptation tests
and Clipper checks cover the dependency refresh. Pack unit declarations are unchanged; Pi reads
the maintained package sources in place. Skill deployment changes are outside this refresh.

The repository advisory scan and both Git-history secret scans pass under the existing
exceptions. Installed-tool advisories remain outside repository lockfile coverage. Unused
analysis remains unavailable without the reviewed cargo-machete executable. Live Linux,
native UI and restore acceptance have not run in this maintenance pass. The full architecture
freshness gate detected concurrent Learning manifest edits in that pass and has not been rerun.
The full documentation-link launcher now passes for all 419 documents.

<a id="3-layout"></a>

## Open, in order

1. Inspect remaining client adapters against the existing API crates before extracting another
   contract. `tools/maintenance --json --overlay` inventories both roots; it does not measure
   duplicate behavior. Preserve offline projections and platform-specific state when replacing
   a copied wire model. Done when each changed client imports its capability contract and its
   serialization and offline tests pass. Remaining travel geocoding acceptance belongs to
   [GPUI work package 31](gpui.md#open).
   Reviewed 2026-11-15: no app-core type shares a name with an API crate type. Device pairing now
   sends `devices_api::ClaimRequest` and `JoinRequest` instead of hand-built JSON. The Projects
   client decodes `RecordRead` and sends the `projects_api` request types for every write with
   a typed request. Open: `GET /projects/api/records` answers untyped JSON, so the client's
   note list keeps a local struct; and the client's route paths copy `projects_api`'s
   `ROUTE_*` constants as literals.
2. Remove stale live paths and references to retired services, after distinguishing historical
   fixtures and compatibility readers from active configuration. Done when maintenance path
   candidates have been reviewed and every active path resolves. Proposed paths remain open
   work rather than broken implementations.
3. Shorten comments and owner documentation as their modules change. Move open work out of
   READMEs and manifests into the owning area plan. Keep dependency verdicts in `upstreams.toml`;
   watch and evaluate entries need a revisit condition. Done when touched files follow the
   documentation rules and their links resolve. A repeatable comment-count ratchet in doctor
   or CI remains open; old snapshot counts are not an executable baseline.
4. Review crate splits only where an existing module has independent consumers. Measure build
   time, retained artifacts and process overhead before proposing workspace consolidation,
   package renames, micro-crate merges or a shared daemon. The root and GPUI workspaces remain
   separate. A layout change needs a measured benefit and coordination with concurrent sessions.
   A capability library with a thin binary is useful only where an in-process consumer exists.

## Compatibility acceptance

The rename retains readers for old configuration names, device headers and signing strings,
health routes, supervisor labels, backup archive names, keychain references, Markdown markers
and persisted record tags. Historical research and fixtures preserve their source data.
Remove a reader only after its persisted input and supported upgrades have been checked;
renaming a stored record is not equivalent to changing current configuration.

Open: live Linux acceptance of systemd-unit migration and the `sjel_filter` nftables replacement.
Generated-output tests do not establish live host acceptance.

<a id="4-the-public-page"></a>

## Other owners

- Registry declarations: [data.md](data.md#registry-derived-project-views).
- Derived views and UI component reuse: [gpui.md](gpui.md#derived-views).
- Markdown file authority: [storage.md](storage.md#one-vault-for-prose).
- Dependency and duplication discovery: [tooling.md](tooling.md).
- Public screenshots and animations, including the phone demo connection:
  [website.md](website.md#open). The phone demo needs a LAN listener and device pairing before
  capture; production data must not appear in published media.
