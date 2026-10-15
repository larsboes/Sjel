# GPUI platform and build notes

## Contents
- Dependency revision
- Platform features
- Native chrome and iOS
- Local verification
- Packaging and runtime evaluation

## Dependency revision

GPUI is pre-1.0 and Zed's upstream README warns that breaking changes are expected. `apps/gpui-prototype` takes GPUI from crates.io as `gpui = { package = "gpui-pre", version = "=…" }`, the weekly Zed snapshot that gpui-kit depends on. Its `Cargo.toml` holds the exact pin, and its `Cargo.lock` resolves one `gpui-pre` family.

gpui-kit, `gpui-pre-mobile` and the app must use the same `gpui-pre` version. Two versions in one build are two sets of types, and the build fails. When you change the pin, read the upstream change, move all three together, compile the app, and launch it on each target platform in scope. Record platform gaps. Do not infer production readiness from a macOS-only build.

## Platform features

Read the matching revision's `crates/gpui/README.md` and `crates/gpui_platform/Cargo.toml` before changing features. Current upstream guidance describes:

- macOS uses Metal; visible text needs the `font-kit` feature.
- Linux and FreeBSD require an enabled desktop backend such as `wayland` or `x11`.
- Windows uses Win32 and DirectWrite without extra platform features.

These are revision-specific facts. Confirm them at the selected source revision before relying on them.

## Native chrome and iOS

The operating system draws the chrome, and GPUI draws the content (`ISA.md`, Decisions 2026-10-15):

- macOS: AppKit owns the sidebar, toolbar, menus and glass. GPUI renders the views inside the window. `apps/gpui-prototype/src/macos_glass.rs` is the narrow AppKit bridge. The resolved GPUI has no general child-view host, so keep the bridge opt-in with a GPUI fallback.
- iOS: UIKit owns the window, navigation, safe areas and keyboard layout. GPUI renders one embedded view. The platform crate is Longbridge's fork, `gpui-pre-mobile` (https://github.com/longbridge/gpui-mobile). gpui-kit's `website/docs/mobile.md` documents the Swift host and calls mobile experimental.

On iOS, a gpui-kit component is validated only if gpui-kit's mobile docs list it. Treat every other component as untested until it runs in the iOS simulator.

## Local verification

In Sjel, follow the repository's build workflow gate before Cargo builds:

```sh
tools/toolchain-check --workflow build
cargo check --manifest-path apps/gpui-prototype/Cargo.toml
```

Do not take the user's focus. The user works on the same Mac while you test. In
`apps/gpui-prototype`, use these checks:

| Check | Command | Focus |
|---|---|---|
| Logic and keyboard behavior | `cargo test` (headless GPUI window, `#[gpui::test]`) | none |
| Visual review | `./install`, then `./preview [page] [light\|dark]` | none |
| Scroll cost per frame | `./install`, then `./bench-scroll [page]` | none |

- Test keyboard interaction with `VisualTestContext::simulate_keystrokes` in `cargo test`. Keys sent to a background window do not reach GPUI.
- `preview` and `bench-scroll` show a corner window above other apps for a few seconds. GPUI stops painting a window that another window covers, so a hidden window gives no frames and no screenshot.
- Do not run `cargo run`, `open sjel://…` or `interceptor macos keys` against the app unless the user asks for it.
- The `gpui` dev-dependency must pin the `gpui-pre` version in `Cargo.lock`. A different pin re-resolves the lockfile and breaks the gpui-base and gpui-component pair.

Do not use `cargo check --workspace` for the standalone prototype: its manifest declares its own workspace so GPUI and its lockfile stay isolated from Sjel's production workspace.

## Packaging and runtime evaluation

Treat compiling, launching, window behavior, keyboard/mouse interaction, text rendering, and packaging as separate checks. GPUI does not make a native desktop surface available inside the web dashboard; web and mobile surfaces remain separate clients. Capture platform and packaging limitations in evaluation notes rather than generalizing from a single successful build.

Upstream navigation: [GPUI README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md), [platform crate](https://github.com/zed-industries/zed/tree/main/crates/gpui_platform), [GPUI source](https://github.com/zed-industries/zed/tree/main/crates/gpui).
