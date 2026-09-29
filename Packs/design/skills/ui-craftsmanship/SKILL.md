---
name: ui-craftsmanship
description: Guides design, layout, styling, and visual review of Sjel user interfaces (Dashboard, capability UIs, components) per anti-vibe-coding doctrine, high-density operational principles, and mechanical sympathy in CSS. Use when designing new UI components, reviewing visual layouts, refactoring CSS/Tailwind styles, fixing spacing/typography drift, or auditing interfaces for generic AI/SaaS design tells. Do not use for backend Rust APIs, non-visual scripts, or prose writing.
allowed-tools: Read, Write, Edit, Bash
---

# UI Craftsmanship

Design and build dense, glanceable, and mechanically sound user interfaces for Sjel.
Reject generic AI/SaaS landing page tropes in favor of intentional, high-utility operational views.

## Core Invariants

1. **Reject the 12 Vibe-Coded AI Tells**: Never default to the generic AI looks (the cream background + serif display + sage accent; purple/indigo hero gradients; over-rounded `rounded-2xl` corners on everything; neon glow in dark mode; emoji icons).
2. **Operational Density & Glanceability**: Sjel is an operational workstation, not a marketing website. Prioritize information density, clear visual hierarchy, scannable lists, and tabular figures for numerical data.
3. **Mechanical Sympathy in CSS**:
   * **Zero Cumulative Layout Shift (CLS)**: Always reserve aspect ratio or dimensions for dynamic images, charts, and lazy content.
   * **Hardware-Accelerated Motion**: Animate *only* `transform` and `opacity`. Never animate `width`, `height`, `margin`, or `padding` which trigger layout reflow.
   * Respect `prefers-reduced-motion`.
4. **Sjel Typography & Identity**:
   * Font stack: `IBM Plex Sans` for UI copy; `IBM Plex Mono` for code, timestamps, and metrics (with `font-variant-numeric: tabular-nums`).
   * Color tokens: derive from the active overlay or theme system, never hardcode random hex values.

## Workflow

### 1. Identify the View Persona
* Determine the layout's purpose:
  * **Operational Dashboard** (e.g. `AxonGlance`, `HomeHorizon`): High density, tabular alignment, glanceable metric tiles, low visual noise.
  * **Interactive Explorer** (e.g. `TransactionTable`, `OmniSearch`, `MonthGrid`): Clear keyboard navigation, fixed column widths, sticky headers, instant feedback.
  * **Inspector / Modal** (e.g. `AssistantDrawer`, `DecisionEngineModal`): Focused task surface, escape-to-close, trap focus cleanly.

### 2. Audit Against Vibe-Coded Tells
Before committing any UI markup or styling, check against the catalog:
* Are buttons or cards using oversized border radiuses (`rounded-3xl` / `rounded-full`) for rectangular content? -> Use subtle, disciplined radii (`rounded` / `rounded-md` / 4-6px).
* Is text using gradient fills (`bg-clip-text text-transparent bg-gradient-to-r...`)? -> Use solid, high-contrast text.
* Are emojis used as icons? -> Use clean SVG vector icons (`Icon.svelte`).
* Read [`references/anti-slop-tells.md`](references/anti-slop-tells.md).

### 3. Establish Typographic & Spacing Rhythm
* Use a consistent 4px/8px spacing scale (`p-1`, `p-2`, `p-4`, `gap-3`).
* Use `tabular-nums` for timestamps, currency, and quantities so numbers don't jump during live updates.
* Read [`references/visual-hierarchy-and-typography.md`](references/visual-hierarchy-and-typography.md).

### 4. Verify Mechanical Sympathy & Performance
* Verify that hover and entrance transitions run at 60fps without repaints.
* Check contrast ratios for both light and dark themes using WCAG AA standards (minimum 4.5:1 for body text).
* Read [`references/mechanical-sympathy-in-ui.md`](references/mechanical-sympathy-in-ui.md).

## Reference Routing

| Topic | Reference |
| --- | --- |
| The 12 AI design tells, code signatures, and fixes | [`references/anti-slop-tells.md`](references/anti-slop-tells.md) |
| IBM Plex typography, tabular numbers, spacing grid | [`references/visual-hierarchy-and-typography.md`](references/visual-hierarchy-and-typography.md) |
| Zero CLS, 60fps CSS transitions, contrast, reduced motion | [`references/mechanical-sympathy-in-ui.md`](references/mechanical-sympathy-in-ui.md) |
