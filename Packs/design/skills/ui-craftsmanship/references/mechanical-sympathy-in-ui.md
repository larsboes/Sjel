# Mechanical sympathy in UI

The interface respects the renderer the way systems code respects the cache. These rules hold
in a browser and in a GPU-native toolkit such as GPUI. Browser specifics are marked.

## No layout shift

A shift during a live update breaks the reader's trust in what they see.

- Reserve the final size of charts, maps, images and lazy content before the data arrives.
- A placeholder has the size of the content it replaces.
- Browser: set `scrollbar-gutter: stable` on scroll containers, so content does not jump when a
  scrollbar appears.

## Motion

- Animate only transform and opacity. Width, height, margin, padding, top and left force a new
  layout of every sibling.
- Durations come from the motion tokens (`--motion-fast`, `--motion-base`, `--motion-slow`).
  Hover and focus feedback uses the fast token. Panels use the base token. Nothing everyday
  goes past 300 ms.
- Name the property you transition. `transition: all` is a defect
  (`tools/dashboard-disclosure.test.ts` blocks it).
- Respect reduced motion. Browser: a `@media (prefers-reduced-motion: reduce)` block that
  removes the animation.

## Contrast

- Body text: at least 4.5:1 against its surface. Large text (18 px and up): at least 3:1.
- Control borders, icons and focus rings: at least 3:1.
- `tools/dashboard-contrast.test.ts` checks the text and tone tokens. A colour outside the
  tokens is not checked, which is one more reason not to use one.

## Depth in the dark theme

- Show elevation with a lighter surface and a 1 px rule, not a coloured glow.
- Use the surface tokens (`--page-bg`, `--surface`, `--card-bg`) for the steps. Never pure black
  for every layer.

## Render less

- Content that is not visible does not render: closed panels, collapsed sections and popovers
  build their content when they open.
- Long lists render in pages or virtualize.
- Prefer a native platform element to a script that rebuilds it.
