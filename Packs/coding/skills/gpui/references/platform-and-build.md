# GPUI platform and build notes

## Contents
- Dependency revision
- Platform features
- Native chrome and iOS
- Local verification
- Packaging and runtime evaluation

## Dependency revision

GPUI is pre-1.0 and Zed's upstream README warns that breaking changes are expected. `apps/gpui` reaches GPUI through gpui-kit (`extern crate gpui_kit as gpui;` at each crate root). gpui-kit pins the weekly `gpui-pre` snapshot of Zed, and its default `gpui-fast` feature selects GPUI Fast. `apps/gpui/Cargo.toml` holds the exact pins, and its `Cargo.lock` resolves one `gpui-pre` family.

gpui-kit, `gpui-pre` and `gpui-fast` must resolve to one compatible set. Two versions in one build are two sets of types, and the build fails. NSH's GPUI backend (the `nsh-trial` feature) also requires a compatible `gpui-fast`. When you change a pin, read the upstream change, move the set together, compile the app, and launch it on each desktop platform in scope. Record platform gaps. Do not infer production readiness from a macOS-only build.

## Platform features

Read the matching revision's `crates/gpui/README.md` and `crates/gpui_platform/Cargo.toml` before changing features. Current upstream guidance describes:

- macOS uses Metal; visible text needs the `font-kit` feature.
- Linux and FreeBSD require an enabled desktop backend such as `wayland` or `x11`.
- Windows uses Win32 and DirectWrite without extra platform features.

These are revision-specific facts. Confirm them at the selected source revision before relying on them.

## Native chrome and iOS

The operating system draws the chrome, and GPUI draws the content (`apps/gpui/README.md`, "Native renderers, shared models"):

- macOS: AppKit owns the sidebar, toolbar, menus and glass. GPUI renders the views inside the window. `apps/gpui/src/macos_glass.rs` is the narrow AppKit bridge. The resolved GPUI has no general child-view host, so keep the bridge opt-in with a GPUI fallback.
- iOS: Sjel's phone does not use GPUI. NSH draws every phone screen with UIKit views (`apps/gpui/ui/src/nsh_*.rs`, `docs/plans/gpui.md` section 11). For phone work, read NSH's documentation at the revision `apps/gpui/Cargo.toml` pins, not this skill.

## Local verification

In Sjel, follow the repository's build workflow gate before Cargo builds:

```sh
tools/toolchain-check --workflow build
cargo check --manifest-path apps/gpui/Cargo.toml
```

Do not take the user's focus. The user works on the same Mac while you test. In
`apps/gpui`, use these checks:

| Check | Command | Focus |
|---|---|---|
| Logic and keyboard behavior | `cargo test` (headless GPUI window, `#[gpui::test]`) | none |
| Visual review | `./install`, then `./preview [page] [light\|dark]` | none |
| Scroll cost per frame | `./install`, then `./bench-scroll [page]` | none |

- Test keyboard interaction with `VisualTestContext::simulate_keystrokes` in `cargo test`. Keys sent to a background window do not reach GPUI.
- `preview` and `bench-scroll` show a corner window above other apps for a few seconds. GPUI stops painting a window that another window covers, so a hidden window gives no frames and no screenshot.
- Do not run `cargo run`, `open sjel://…` or `interceptor macos keys` against the app unless the user asks for it.
- The `gpui` dev-dependency must pin the `gpui-pre` version in `Cargo.lock`. A different pin re-resolves the lockfile and breaks the gpui-base and gpui-component pair.

Do not check `apps/gpui` with `cargo check --workspace` from the repository root: its manifest declares its own workspace, so GPUI and its lockfile stay isolated from Sjel's main workspace.

## Packaging and runtime evaluation

Treat compiling, launching, window behavior, keyboard/mouse interaction, text rendering, and packaging as separate checks. GPUI provides native surfaces, not a browser client; capture platform and packaging limitations in evaluation notes rather than generalizing from a single successful build.

Upstream navigation: [GPUI README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md), [platform crate](https://github.com/zed-industries/zed/tree/main/crates/gpui_platform), [GPUI source](https://github.com/zed-industries/zed/tree/main/crates/gpui).
