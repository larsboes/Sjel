# Dependencies and build decisions

## Dependency choice

Apply this order: adopt, contribute upstream, overlay, temporary fork, build. Record the verdict,
pin, license, and reason before consumption. Never use an unpinned floating release for code.

## Implementation defaults

- Add backend logic in Rust unless the existing owner has a justified different runtime.
- Use `uv` for Python execution and `bun` for TypeScript; do not add `pip`, `npm`, or bare `node`
  commands to Axon code or documentation.
- Keep shell compatible with macOS Bash 3.2. Avoid associative arrays and Bash 4 features.
- Express container-backed capabilities as manifests consumed by shared runners, not new bespoke
  lifecycle scripts.
- Import shared schemas and declare cross-capability service requirements; do not import another
  capability's implementation.

## Build decision

`cargo` builds and tests the Rust workspace: one root workspace, one `Cargo.lock`, each member
manifest owning its direct dependencies. `bun` owns TypeScript and the UI bundles. A capability
declares its own build in `service.toml` (`build = ["cargo", "build", ...]`) and
`tools/service-runner.sh` runs that before starting the binary named by `command`.
Generated-architecture freshness is `tools/check-architecture-fresh.sh`, and
`tools/generate-architecture.sh` fixes a stale one.

Any build layer above those two is argued per case: name what it buys and what toolchain cost it
adds. An interpreted tool stays interpreted when wrapping it adds machinery without improving
correctness. Record the reason beside the tool that enforces the decision. See
CONTRIBUTING.md#cargo-and-bun-are-the-build-path.
