# Native app

The [native UI contract](../architecture/native-ui-boundaries.md) owns reuse, platform and test
boundaries. The [application README](../../apps/gpui/README.md) owns current interfaces and
[UI-stack research](../../research/ui-stack.md) owns renderer evaluations.
[PLAN.md](../../PLAN.md) governs ordering; phone-only work remains Priority 4.

<a id="components-from-ely"></a>
<a id="resume-checklist-ely"></a>

## Priority 1 contracts and controls

- [Make note rename and saves lossless](../tasks/gpui-note-rename.md).
- [Isolate NSH behind an app-owned facade](../tasks/gpui-nsh-facade.md).
- [Measure NSH Mail before desktop convergence](../tasks/gpui-nsh-measurement.md).
- [Accept migrated desktop controls](../tasks/gpui-ely-acceptance.md).
- [Accept native table accessibility and editing](../tasks/gpui-table-acceptance.md).


## Derived views

- [Accept horizontal registry navigation](../tasks/gpui-registry-mac.md).
- [Choose a validated view document format](../tasks/gpui-view-format.md).
- [Reload saved views without a model](../tasks/gpui-saved-views.md).
- [Render live views in the agent panel](../tasks/gpui-live-views.md).
- [Accept governed offline object browsing](../tasks/gpui-registry-phone.md).

<a id="open"></a>
<a id="4-work-packages"></a>

<a id="1-route-matrix"></a>

## Mac daily workflows

- [Accept the Mac Documents workflow](../tasks/gpui-documents-mac.md).
- [Complete reviewable document decisions](../tasks/gpui-documents-decisions.md).
- [Accept evidence-backed document proposals](../tasks/gpui-document-proposals.md).
- [Accept native Mail review and actions](../tasks/gpui-mail-mac.md).
- [Accept native Calendar interactions](../tasks/gpui-calendar-mac.md).
- [Accept consolidated Mac Travel](../tasks/gpui-travel-mac.md).
- [Choose safe historical place resolution](../tasks/gpui-travel-backfill.md).
- [Accept Mac People mutations](../tasks/gpui-people-writes.md).
- [Accept Finance import and review](../tasks/gpui-finance-import.md).
- [Accept native Discover workflows](../tasks/gpui-discover.md).
- [Accept Inventory and wishlist mutations](../tasks/gpui-inventory-writes.md).
- [Accept task and project controls](../tasks/gpui-project-tasks.md).
- [Accept Feed jobs and classification changes](../tasks/gpui-feed-actions.md).

<a id="notes-editor-live-preview-open"></a>
<a id="9-frame-cost"></a>

## Editor, layout and performance

- [Review the single-block preview prototype](../tasks/gpui-editor-preview.md).
- [Measure Finder search misses](../tasks/gpui-search-quality.md).
- [Measure remaining Packs matrix cost](../tasks/gpui-packs-performance.md).
- [Accept desktop demo switching](../tasks/gpui-dev-acceptance.md).
- [Accept native chart interactions](../tasks/gpui-chart-acceptance.md).
- [Prove read-only Ely Mail parity](../tasks/gpui-mail-adapter.md).
- [Evaluate block editor round-trip parity](../tasks/gpui-editor-adapter.md).
- [Accept persisted desktop layout controls](../tasks/gpui-shell-acceptance.md).

<a id="5-finder-and-right-edge"></a>

## Deferred expansion

- [Specify agent-driven navigation](../tasks/gpui-agent-follow.md).
- [Verify additional desktop platforms](../tasks/gpui-platforms.md).
- [Show sourced person interaction history](../tasks/gpui-people-history.md).

<a id="2-phone-priority-areas"></a>
<a id="what-may-live-on-the-phone"></a>
<a id="11-ui-stack"></a>

## Phone acceptance

- [Accept offline phone Documents](../tasks/gpui-documents-phone.md).
- [Accept phone Mail replay and attachments](../tasks/gpui-mail-phone.md).
- [Accept native phone Calendar gestures](../tasks/gpui-calendar-phone.md).
- [Accept offline phone Travel](../tasks/gpui-travel-phone.md).
- [Add phone trip edits through the owner](../tasks/gpui-travel-phone-writes.md).
- [Capture phone shares exactly once](../tasks/gpui-share-extension.md).
- [Unify phone domain replay](../tasks/sync-domain-shape.md).
- [Accept shared phone navigation and lifecycle](../tasks/gpui-phone-navigation.md).
- [Register phone deep links](../tasks/gpui-phone-links.md).
- [Pair the phone to synthetic demo data](../tasks/gpui-demo-phone.md).
