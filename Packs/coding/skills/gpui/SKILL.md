---
name: gpui
description: Guides native desktop UI development with Zed Industries' GPUI Rust framework, including Sjel's evaluation app. Use when creating or changing GPUI views, entities, events, actions, platform setup, or framework dependencies. Do not use for general Rust backend work, Svelte interfaces, browser-only UI, or product design decisions; use ui-craftsmanship for those.
---

# GPUI

Build native desktop surfaces with GPUI's views, entities, elements, and platform crates. GPUI is pre-1.0; APIs and platform support can change between revisions.

For product and visual design decisions, read `ui-craftsmanship` and keep shared UX guidance
there. Treat Sjel's future Mac and iPhone surfaces as one product with platform-specific
interaction patterns. The iPhone path is experimental: read "Native chrome and iOS" in
`references/platform-and-build.md` before any iOS work.

## Workflow

1. **Locate the app and pin.** Read its `Cargo.toml` and lockfile. For Sjel, the desktop app is `apps/gpui`; it has its own Cargo workspace and lockfile. Run `python3 scripts/show_revision.py <Cargo.toml>` to print the selected Zed source revision and matching GitHub links. The helper requires Python 3.11+; do not assume `main` matches the app's API.
2. **Load only the relevant reference.** Read `references/views-and-events.md` for UI structure and input; `references/async-and-data.md` for state updates and I/O; or `references/platform-and-build.md` for OS features and verification.
3. **Follow the local revision.** Prefer examples and API definitions under that revision's `crates/gpui` and `crates/gpui_platform`. Use the upstream `main` links as navigation only. Do not update GPUI pins as part of an unrelated UI change.
4. **Keep the app boundary clear.** In Sjel, use existing capability APIs or reviewed local interfaces. Do not give the desktop UI direct access to capability databases or copy private overlay values into source.
5. **Verify the smallest affected target, and do not take the user's focus.** Before building in Sjel, run `tools/toolchain-check --workflow build`. Then run `cargo check --manifest-path ...`. Do not test by launching the app in the foreground, by opening a `sjel://` link, or by sending keys to its window: the user is working on the same Mac. Use the focus-free checks in `references/platform-and-build.md` (Local verification).

## Error handling

- If the pinned source link is unavailable locally, inspect the Git revision from `Cargo.lock`; do not silently substitute upstream `main`.
- If a GPUI async callback fails to compile, inspect the executor and context APIs at the pinned revision before changing ownership or adding another runtime.
- If a network client panics about a missing Tokio runtime, keep that I/O on an executor/runtime that owns its timers. GPUI's integrated executor does not automatically provide Tokio's runtime context.
- If a platform build fails, check the matching `gpui_platform` feature and OS toolchain in `references/platform-and-build.md`.

## Change scope

Keep rendering pure and fast. Move blocking I/O off the UI thread, return results into GPUI state, and notify the entity so the view renders again. Preserve the current dependency pin until a deliberate upgrade is reviewed against the local app.
