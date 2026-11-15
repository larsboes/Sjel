# GPUI Kit headless UI tests

## Resolve the test API

Use the revision helper's testing-reference link, then inspect the installed package's
`src/test.rs` and re-exports. Keep `gpui-kit` development and normal dependencies on the same
source and version; enable `test-support` on the development dependency. Do not add a second
GPUI dependency to obtain test types. Existing `VisualTestContext` tests remain valid; use the
Kit helpers where they make a changed interaction clearer rather than rewriting unrelated tests.

Use ordinary Rust tests for pure logic and GPUI context tests for entity/event behavior.
Use headless UI integration tests when a change depends on a rendered control, input dispatch,
focus, scrolling or layout. Import test types explicitly: a glob can import GPUI's `test` macro
and shadow Rust's ordinary `#[test]`. Sjel aliases `gpui_kit` as `gpui`; follow the crate's imports.

## Exercise the production view

1. Initialize the library, construct the real view with its retained state and subscriptions,
   and open a headless window with an explicit size. Preserve the production root contract.
2. Render a frame and find stable control IDs with `TestWindowExt`. Custom elements may need
   `TestSupportExt::test_support`; verify registration and focus-binding order in the installed API.
3. Send pointer or keyboard events through the window helpers. Setting a model field or calling
   a handler directly does not test control dispatch. Use the focused input helper for text;
   key transitions alone do not insert text.
4. Assert the resulting application state and the relevant focus or geometry. Include a
   negative case when it matters, such as disabled input rejecting an action. Query a fresh
   snapshot after updates; render after external state changes or resizing.
5. Release the window update before draining queued effects or waiting for deferred work.
   Use the framework's bounded test-time wait where supported. Assert timers before and after
   their deadline; advancing test time does not advance wall-clock animations.

## Interpret the evidence

Accessibility snapshots are not pixels. A missing disabled flag does not prove that a control
accepts input; activate it and check the result. Visibility and bounds do not prove an element
receives clicks through an overlapping surface. Combine properties with actual interactions.

Headless input and clipboard adapters do not verify OS IME windows, the system clipboard,
packaged-app behavior or rendered pixels. Use the focus-free visual and platform checks in
[platform-and-build.md](platform-and-build.md) when those claims are in scope. Do not launch
the foreground app to verify an interaction covered by a headless test.
