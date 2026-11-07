# Public website

Status: in progress. The website is built by `website/build` from tracked TypeScript and Markdown. Generated `site/` files are not edited directly.

## Direction

- Decided 2026-11-06: added dedicated "Connected intelligence" section on the landing page highlighting Sjel's core differentiator: intentionally scaling across 45 capabilities to connect disparate life data sources (calendar, transit, finance, contacts, notes) via typed joins (`sjel-links`) for combined analysis value that single-purpose apps cannot provide. Added an interactive scenario explorer switcher (`Trip & Transit`, `Doctor & Health`, `Lease & Relocation`) and node focus inspector.
- Decided 2026-11-06: added an interactive Privacy Gate Simulator in the `#privacy` section. Visitors can toggle data classes (`c0`–`c3`) and target models (`Local Mac Model` / `Cloud Model API`) to inspect real-time JSON input, admission evaluation via `content_item::cloud_admission`, and reactive visual state transitions (admitted, pseudonymized derivative, refused, blocked in keychain).
- Decided 2026-11-06: added subtle SVG wire pulses along the system topology paths (`iPhone -> Node`, `Mac -> Node`, `Node -> Gate`) with electric teal glow, completely disabled under `prefers-reduced-motion: reduce`. Switched the hero mark to the crisp official vector `hedgehog.svg` on signature `#16748D` field with subtle card elevation.
- Decided 2026-11-06: restored signature Sjel turquoise (`#16748D`, `#2A93AC`, `#6CC3D8`) as the primary brand and interactive accent across the site, pairing it with a high-contrast neutral framework. Restored brand assets (`favicon.svg` with teal tile, `hedgehog.svg` with official `#16748D` field), highlighted the architecture class gate in rich teal with teal data flow wires, brought back functional privacy category color accents for `c0`–`c3`, and gave interactive elements (buttons, links, active navigation, focus rings) their signature teal punch.
- Decided 2026-11-06: use Zed as a reference for restraint, hairline structure, and editorial hierarchy, not as a template. System fonts only (`--sans`, `--mono`, `--display`); zero webfont dependency. Hairline blueprint corner tick marks (`+`) on card grids and technical frames.
- Decided 2026-11-06: default to operating system color preference (`prefers-color-scheme`). Provide an accessible segmented UI control (System / Light / Dark) that persists choice in `localStorage`, keeps System following OS changes, prevents theme flash via a synchronous inline bootstrap in `<head>`, and scopes `T` cycling to focus inside the theme control. Keep navigation and content fully functional without JavaScript.
- Decided 2026-11-06: dual topology diagram options on the landing page: Vector and ASCII Unicode box-drawing views selected by native toggle buttons with `aria-pressed` and `aria-controls`; retain a `<details>` fallback when scripting is disabled.
- Decided 2026-11-06: keep motion brief, purposeful, and fully disabled under `prefers-reduced-motion: reduce`.
- Decided 2026-11-06: every public claim must be specific and supported by the implementation or a reproducible measurement. Do not present generated examples as real personal data.

## Open

- [x] Refresh the landing page and shared page shell with responsive typography, restrained motion, and one coherent token set.
- [x] Audit and edit copy across all 21 Markdown pages under `website/content/`; preserve API, schema, and implementation facts.
- [x] Implement accessible System / Light / Dark theme control with `localStorage` persistence, OS-following behavior, and `<head>` anti-flash bootstrap.
- [x] Re-introduce signature Sjel turquoise brand palette across tokens, SVG architecture diagram, class gate, data-class badges, focus rings, and vector brand assets (`favicon.svg`, `hedgehog.svg`).
- [x] Replace the unsupported terminal query and unverified output with an explicitly illustrative, non-executed example.
- [x] Build interactive Connected Intelligence scenario switcher and node inspector across 3 cross-domain life workflows.
- [x] Build interactive Privacy Gate test bench allowing dynamic testing of classes `c0`–`c3` against local and cloud model destinations.
- [x] Add SVG wire pulses along topology paths with electric teal glow, disabled under `prefers-reduced-motion: reduce`.
- [x] Add component foreground/background contrast-ratio checks for light and dark theme colors (minimum 4.5:1); verify generated pages build and internal links resolve.
- [ ] Verify rendered navigation and horizontal overflow at 768px and 800px in a browser, in both color themes. Browser viewport testing was unavailable during the review fix.

## Done when

- `website/build` and its link and payload checks pass.
- Landing, docs, reference, and roadmap share the same visual system at desktop and mobile widths.
- The page remains usable with reduced motion enabled and without JavaScript for navigation or content.
- Every claim changed during the copy pass matches its source, and no illustrative example is presented as real user data.
