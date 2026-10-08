# Visual hierarchy, typography and density

Sjel is read many times a day. Hierarchy tells the reader where to look first. Density lets
them compare without scrolling. Token names below are the dashboard's (`dashboard/src/app.css`).

## Typography

- **UI text**: IBM Plex Sans (`--font-sans`).
- **Data, code, timestamps, amounts**: IBM Plex Mono (`--font-mono`) with
  `font-variant-numeric: tabular-nums`, so digits do not shift when values change.
- **Numbers in a column are right-aligned**, so digits line up by place value.
- **Scale**: use the type tokens (`--text-2xs` to `--text-2xl`). Metadata and table headers sit
  at the bottom of the scale, panel titles near the middle. Workstation views have no
  marketing-size headings.

## Spacing

- Use the spacing tokens (`--space-1` to `--space-8`) on a 4 px base. No arbitrary values.
- Tight spacing groups related items. A larger gap separates groups. Do not use a border where
  a gap already separates.

## Density

- Keep row heights equal inside a table. Compact rows are about 28–36 px.
- Labels and units use secondary or tertiary text. The value uses primary text.
- Move secondary controls down the explicitness ladder (`SKILL.md`, rule 2). Do not shrink them
  to fit.

## Semantic colour

Colour says what a value means. Use the tone tokens, never a raw hue.

| Meaning | Tone |
|---|---|
| Settled, healthy, done | success |
| Needs a decision, degraded | warning |
| Broken, failed, refused | danger |
| Chosen, primary, link | accent / primary |
| Not active | muted (tertiary text, no fill) |
| Everything else | neutral |
