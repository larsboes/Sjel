# Verify proportionately

Select checks from the changed ownership boundary. Never claim broader validation than ran.

1. Run the nearest unit tests, type checks, or shell syntax checks for changed code.
2. Run repository gates whose declared inputs include the changed files.
3. Check generated artifacts when their source manifests changed.
4. Re-run `tools/sjel-context on <target>` or the relevant API read to verify observable state.
5. Inspect `git diff --check`, the focused diff, and `git status --short` before committing.

Common gates include `tools/self check`, `tools/audit`, and the
architecture freshness gate `tools/check-architecture-fresh.sh`, but only run a gate when its
concern is in scope. Read the nearest `Cargo.toml`, `package.json`, `service.toml` or README to
discover the exact target rather than relying on a static validation list.

Record skipped checks and pre-existing failures separately. A failing unrelated doctor item does
not invalidate a focused change, but it must not be described as passing.

## Reuse verification output

Run Rust verification through `sjel cargo`. Its default reuses checkout-scoped output and selects
a stable writable fallback when the inherited cache cannot be used. Reuse that cache across
checks, tests and Clippy rather than creating a target per command to avoid Cargo's lock. Use a
separate target only for incompatible build configurations or a measured concurrency need; add
its disposable output to the deployment's guarded retention policy before building.

For a genuinely one-shot check, `CARGO_INCREMENTAL=0 sjel cargo check --locked` avoids retaining
incremental state. Keep incremental compilation for repeated development checks; changing the
setting can require recompilation. Never clean or relink live release trees as cache maintenance.

## Claiming that the gates pass

Say "the gates pass" only after all of these pass: `cargo test --workspace --locked`,
`bun test`, every `tools/check-*.sh` gate, and `tools/check-architecture-fresh.sh`.

`tools/self check` compares tracked inputs only. Verify a gate claim about `origin/main` in a
worktree.

For an overlay sweep, use `rg --no-ignore --hidden --follow`. Plain `rg` skips ignored
configuration.

## When the claim is that a UI renders

GPUI renders natively across macOS and iOS (`apps/gpui`). Verify view models and components
through native integration tests or headless render passes. When testing against a demo stack,
use `sjel demo up` and inspect actual layout metrics.

Two Interceptor traps, both measured 2026-09-30, both of which look like a broken page:

- Safari's extension answers roughly ten calls, then goes mute. `interceptor daemon stop` revives
  it, and the extension reconnects within ~20 s on its own; it is a workaround, not a cure.
- `interceptor screenshot` re-renders the DOM at its own viewport width (1400–1464 CSS px for a
  1512 px tab), so its fallback font re-wraps text and squeezes chips. Two layout defects "seen"
  that way did not exist. Measure `clientWidth` against `scrollWidth` in the page before
  reporting one.
