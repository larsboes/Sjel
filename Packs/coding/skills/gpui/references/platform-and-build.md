# GPUI platform and build notes

## Contents
- Dependency revision
- Platform features
- Local verification
- Packaging and runtime evaluation

## Dependency revision

GPUI is pre-1.0 and Zed's upstream README warns that breaking changes are expected. Keep `gpui` and `gpui_platform` on a compatible source revision. For Sjel's current evaluation app, both are Git dependencies pinned to the same Zed commit in `apps/gpui-prototype/Cargo.toml`, with the resolved source recorded in that app's `Cargo.lock`.

Run `scripts/show_revision.py apps/gpui-prototype/Cargo.toml` to print the pinned commit and source links. When changing a pin, read the upstream change, update both dependencies together, compile the app, and launch it on each target platform in scope. Record platform gaps; don't infer production readiness from a macOS-only build.

## Platform features

Read the matching revision's `crates/gpui/README.md` and `crates/gpui_platform/Cargo.toml` before changing features. Current upstream guidance describes:

- macOS uses Metal; visible text needs the `font-kit` feature.
- Linux and FreeBSD require an enabled desktop backend such as `wayland` or `x11`.
- Windows uses Win32 and DirectWrite without extra platform features.

These are revision-specific facts. Confirm them at the selected source revision before relying on them.

## Local verification

In Sjel, follow the repository's build workflow gate before Cargo builds:

```sh
tools/toolchain-check --workflow build
cargo check --manifest-path apps/gpui-prototype/Cargo.toml
```

Launch the app for visual review with `cargo run --manifest-path apps/gpui-prototype/Cargo.toml`. Run focused tests only when asked or when interaction behavior needs automated coverage. Do not use `cargo check --workspace` for the standalone prototype: its manifest declares its own workspace so GPUI and its lockfile stay isolated from Sjel's production workspace.

## Packaging and runtime evaluation

Treat compiling, launching, window behavior, keyboard/mouse interaction, text rendering, and packaging as separate checks. GPUI does not make a native desktop surface available inside the web dashboard; web and mobile surfaces remain separate clients. Capture platform and packaging limitations in evaluation notes rather than generalizing from a single successful build.

Upstream navigation: [GPUI README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md), [platform crate](https://github.com/zed-industries/zed/tree/main/crates/gpui_platform), [GPUI source](https://github.com/zed-industries/zed/tree/main/crates/gpui).
