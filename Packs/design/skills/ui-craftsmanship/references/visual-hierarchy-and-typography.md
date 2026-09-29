# Visual Hierarchy, Typography & Density

Operational tools like Sjel require clear visual hierarchy to communicate state quickly without fatiguing the operator.

## 1. Typography Discipline (IBM Plex)
* **Body & UI**: `font-family: 'IBM Plex Sans', -apple-system, sans-serif;`
  - Clean, legible, distinct letterforms (e.g. clear distinction between `l`, `1`, and `I`).
* **Data, Code & Metrics**: `font-family: 'IBM Plex Mono', monospace;`
  - Always activate tabular numbers: `font-variant-numeric: tabular-nums;`
  - Right-align numbers in tables so decimal places and digit magnitudes align visually.
* **Typographic Scale**:
  - `text-xs` (11–12px): Metadata, secondary badges, table footers.
  - `text-sm` (13–14px): Standard UI body, table cell text, form labels.
  - `text-base` (15–16px): Section leads, primary interactive elements.
  - `text-lg` / `text-xl` (18–20px): Panel titles, metric summaries.
  - Avoid giant marketing headings (`text-6xl`) in workstation views.

## 2. Spacing Scale (4px/8px Grid)
* Build layouts using a strict modular scale:
  - `4px` (`gap-1`, `p-1`): Inner icon-to-label spacing, compact tag padding.
  - `8px` (`gap-2`, `p-2`): List item spacing, button internal padding.
  - `12px` (`gap-3`): Card internal element separation.
  - `16px` (`gap-4`, `p-4`): Panel padding, grid gutter.
  - `24px` (`gap-6`, `p-6`): Primary section boundary.
* Avoid arbitrary, unconstrained spacing (`p-[17px]`, `gap-[23px]`).

## 3. High Information Density
* Operators scan across multiple data dimensions simultaneously:
  - Keep row heights consistent (e.g. `36px` to `40px` for compact tables).
  - Use muted text colors (`text-zinc-500` / `text-neutral-400`) for labels and units, leaving high-contrast text (`text-zinc-900` / `text-zinc-100`) for the actual data value.
  - Collapse secondary controls into contextual menus or hover actions to keep primary views scannable.

## 4. Semantic Color Tokens
* Colors must communicate meaning, not decoration:
  - **Neutral**: Foundation surfaces, borders, and text hierarchy.
  - **Live / Active / Healthy**: Emerald/Green (`text-emerald-500`).
  - **Warning / Degraded**: Amber/Yellow (`text-amber-500`).
  - **Critical / Refused / Failed**: Rose/Red (`text-rose-500`).
  - **Informational / Primary**: Blue/Cyan (`text-sky-500`).
