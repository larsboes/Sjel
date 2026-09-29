# Mechanical Sympathy in UI (CSS Performance & Accessibility)

Just as systems Rust code respects memory and cache lines, frontend interfaces must respect browser layout engines, GPU compositing, and user accessibility.

## 1. Zero Cumulative Layout Shift (CLS)
Unexpected layout shifts during live data polling or asset loading destroy operator trust:
* **Reserve Aspect Ratios**: Always set `aspect-ratio` or explicit `width`/`height` on images, chart containers, and maps:
  ```svelte
  <div class="aspect-video w-full rounded bg-zinc-900/50">
      <Chart data={data} />
  </div>
  ```
* **Fixed Dimensions for Skeletons**: Skeleton loaders must match the exact pixel height and margin of the final content they replace.
* **Scrollbar Stability**: Use `scrollbar-gutter: stable;` on main containers to prevent content jumping when scrollbars appear or disappear.

## 2. 60 FPS Transitions & GPU Compositing
* **The Golden Rule**: Animate *only* `transform` and `opacity`.
  - Properties like `top`, `left`, `width`, `height`, `margin`, and `padding` trigger full layout reflows (recalculating the position of every sibling element on the page).
  - `transform: translate3d(x, y, 0)` and `opacity` are handled directly on the compositor thread on the GPU without repainting the document.
* **Keep Transitions Snappy**:
  - Micro-interactions (hover, active, focus): `100ms–150ms cubic-bezier(0.4, 0, 0.2, 1)`.
  - Dialog / Drawer entry: `200ms–250ms ease-out`.
  - Never add transitions longer than 300ms for everyday operational actions.

## 3. Accessible Contrast & Theme Discipline
* **Contrast Ratios**:
  - Normal text (< 18px): minimum 4.5:1 against its background.
  - Large text (>= 18px): minimum 3:1.
  - Interactive component boundaries and icons: minimum 3:1.
* **Dark Mode Depth Without Neon**:
  - Never use pure black (`#000000`) for all surfaces. Use subtle elevations:
    - Base canvas: `#0d0e11` or `#121316`
    - Card surface: `#18191d`
    - Modal / Popover: `#202227`
  - Separate surfaces with subtle 1px border lines (`border-zinc-800/80`) rather than aggressive drop shadows.

## 4. Respect User Preferences
Always support `prefers-reduced-motion`:
```css
@media (prefers-reduced-motion: reduce) {
  *, ::before, ::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
    scroll-behavior: auto !important;
  }
}
```
In Tailwind: use the `motion-reduce:` and `motion-safe:` variants.
