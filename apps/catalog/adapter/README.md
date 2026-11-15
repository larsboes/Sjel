# Renderer state boundary

`client-state.ts` wraps an immutable JSON store. The renderer can write local form drafts under
`/draft` and presentation state under `/ui`. The host keeps the original store for authorized
snapshot updates. Returned trees are frozen; host updates replace objects rather than mutate
them. Batch path validation completes before any write delegates to the underlying store.

The wrapper has no runtime dependency, transport, capability access or proposal handler. It is
not wired into the derived catalog. Its method surface matches the inspected json-render
StateStore interface; a test against that upstream implementation has not run.

From the repository root:

```sh
bun --no-env-file test apps/catalog/adapter/client-state.test.ts
```

The [research note](../../../research/svelte-adaptive-ui.md) owns the runtime comparison,
source references and adoption checks. [The catalog plan](../../../docs/plans/gpui.md#views-from-the-web-catalog)
owns delivery order. Rust remains responsible for document validation and authorization.
