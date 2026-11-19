# Restructure and sweep

[PLAN.md](../../PLAN.md#priority-1-unify-and-reduce-maintenance) owns priority.
Shared contracts and removal of repeated behavior precede directory moves.

<a id="3-layout"></a>

## Open, in order

- [Remove copied client contracts](../tasks/contracts-client-adapters.md).
- [Remove stale active paths](../tasks/maintenance-live-paths.md).
- [Shorten owner documentation](../tasks/maintenance-owner-docs.md).
- [Add a comment-count ratchet](../tasks/maintenance-comment-ratchet.md).
- [Measure crate-boundary changes](../tasks/maintenance-crate-boundaries.md).

## Compatibility acceptance

[Accept Linux service migration](../tasks/linux-rename-acceptance.md).

Remove compatibility readers only after checking persisted inputs and supported upgrades.
This includes old configuration names, device headers, signing strings, health routes, supervisor
labels, backup names, keychain references, Markdown markers and stored record tags.
Historical research and fixtures preserve their source data.

<a id="4-the-public-page"></a>

## Other owners

- Travel geocoding and native acceptance: [GPUI](gpui.md#open).
- Registry declarations: [data](data.md#registry-derived-project-views).
- Markdown authority and restore acceptance: [storage](storage.md#one-vault-for-prose).
- Dependency discovery and remaining verification: [tooling](tooling.md).
- Public app media: [website](website.md#open).
