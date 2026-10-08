# Views and events

## Contents
- Application and view structure
- Elements and interactive controls
- Entity ownership and updates
- Keyboard actions and accessibility

## Application and view structure

Start a standalone application with `gpui_platform::application().run(...)`, open a window with `App::open_window`, and register a root view. A view is an entity that implements `Render`; its `render` method builds an element tree for a frame. Use ordinary high-level elements first. Implement a custom `Element` only when a view needs low-level layout or rendering control.

For source-level questions, inspect the pinned revision's `crates/gpui/README.md`, `crates/gpui/examples`, and the relevant module under `crates/gpui/src`. GPUI docs describe a hybrid immediate and retained model: views produce element trees, while GPUI owns entity state and lifecycle.

## Elements and interactive controls

- Give interactive elements stable, unique IDs. Attach click handlers with the event API available at the pinned revision.
- Make affordances visible: pointer cursor, hover/pressed treatment, disabled/loading state, and clear labels.
- Keep handlers short. Update state, start async work, and return; never perform blocking I/O inside `Render` or an input callback.
- Use GPUI actions for keyboard shortcuts and command-style operations. Read the pinned `action` module and examples before choosing key-binding APIs.
- Provide keyboard focus and accessibility metadata for custom controls. Consult the pinned accessibility guide and existing Zed patterns instead of assuming that a clickable `div` is accessible.

## Entity ownership and updates

Use a view/entity for state that GPUI must own and observe. Use the context APIs to update an entity and notify it after a result changes visible state. Do not keep raw references to a view across asynchronous work; capture owned inputs, then return through the appropriate GPUI update/context API.

Separate durable domain data and service clients from transient view state. GPUI owns presentation lifecycle; the existing Sjel service or capability owns domain behavior and persistence.

## Keyboard actions and accessibility

Inspect the pinned revision before copying an action or accessibility example from `main`. In particular, verify action registration, key dispatch, focus behavior, and element accessibility APIs against the exact dependency revision.

Upstream navigation: [GPUI source](https://github.com/zed-industries/zed/tree/main/crates/gpui), [GPUI examples](https://github.com/zed-industries/zed/tree/main/crates/gpui/examples), [GPUI docs](https://github.com/zed-industries/zed/tree/main/crates/gpui/docs).
