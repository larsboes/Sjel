# Verify proportionately

Select checks from the changed ownership boundary. Never claim broader validation than ran.

1. Run the nearest unit tests, type checks, or shell syntax checks for changed code.
2. Run repository gates whose declared inputs include the changed files.
3. Check generated artifacts when their source manifests changed.
4. Re-run `tools/axon-context on <target>` or the relevant API read to verify observable state.
5. Inspect `git diff --check`, the focused diff, and `git status --short` before committing.

Common gates include `tools/self check`, `tools/audit`, and the
architecture freshness gate `tools/check-architecture-fresh.sh`, but only run a gate when its
concern is in scope. Read the nearest `Cargo.toml`, `package.json`, `service.toml` or README to
discover the exact target rather than relying on a static validation list.

Record skipped checks and pre-existing failures separately. A failing unrelated doctor item does
not invalidate a focused change, but it must not be described as passing.

## When the claim is that a UI renders

The dashboard suite is pure functions over view models. Nothing mounts a component, so no test
sees an import-time crash, a rune used outside a compiled file, or a panel that draws empty.
Measured 2026-09-30: 183 tests and `svelte-check` were green while every route served SvelteKit's
own 500 page in a browser, because a module-scope probe read `globalThis.$state` — a property a
browser defines as a getter that throws. When the claim is "the panel shows X", read the DOM at
the tab's real viewport and compare its numbers against the source of truth, not against a 200.

Two Interceptor traps, both measured 2026-09-30, both of which look like a broken page:

- Safari's extension answers roughly ten calls, then goes mute. `interceptor daemon stop` revives
  it, and the extension reconnects within ~20 s on its own; it is a workaround, not a cure.
- `interceptor screenshot` re-renders the DOM at its own viewport width (1400–1464 CSS px for a
  1512 px tab), so its fallback font re-wraps text and squeezes chips. Two layout defects "seen"
  that way did not exist. Measure `clientWidth` against `scrollWidth` in the page before
  reporting one.
