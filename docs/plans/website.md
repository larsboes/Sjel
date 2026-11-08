# Public website

`website/build` generates the site from tracked TypeScript and Markdown. Build and layout
contracts live in `website/README.md`; generated output is not edited directly.

## Shipped

- Landing page, connected scenarios, privacy illustration, theme control, brand assets and
  contrast checks: `40a916c4`. Implementation detail stays in the website sources.
- The build launcher disables environment-file loading for every Bun generator and server.
  Verification: scratch generation, internal links and payload checks pass. Browser rendering
  was not checked in this change.

## Open

1. Verify navigation and horizontal overflow at 768px and 800px in both color themes.
   Done when neither viewport overflows and navigation remains reachable.
2. Select app screenshots and animations after native acceptance, using the seeded demo.
   Done when published media shows synthetic data and matches the current app.

## Acceptance

The build's link and payload checks pass. Navigation and content work without JavaScript;
reduced-motion preference disables animation. Public claims match implementation or a
reproducible measurement, and illustrative examples are identified as such.
