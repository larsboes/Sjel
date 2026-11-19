---
id: soundscape-context
title: "Shape Soundscape sessions around listening intent"
state: verify
area: product
kind: feature
priority: 3
spec: docs/plans/product.md
evidence:
  - "The conductor passes 20 tests, including the Bedside morning deadline, remembered bedtime volume and repeat after restart."
  - "The native core passes 59 Soundscape tests and both offline signal quality checks. Bedside tests cover the three rooms, wind-down, dawn and the ending fade."
  - "Nine Mac drawer tests pass, including pointer reveal from the dark Bedside player."
  - "Appearance settings preserve the mini player preference across serialization and show it by default for existing settings."
---

# Shape Soundscape sessions around listening intent

## Outcome

Soundscape offers distinct Focus, Relax and Sleep arrangements and a player centred on the current session.

## Acceptance

- Local time and active committed calendar blocks suggest a timed scenario. Playback starts explicitly.
- The optional adaptive mix responds to the same signals within the selected mode. Manual mode selection holds throughout the session.
- Timed Focus sessions ease in, hold a steady arrangement and thin out near the end. Pausing preserves the timer checkpoint.
- The desktop drawer supports drag and keyboard resizing. The optional sidebar mini player exposes playback and session progress.
- The mini player repeats the last timed session from its original duration and starting mix.
- Bedside renders three distinct rooms with a 25-minute wind-down, a timed or overnight ending,
  and an optional dawn transition. Back to sleep starts from the settled room and ends after 20 minutes.
- Bedside recalls the selected room and volume. The dark player reveals its controls on click.
- Accept the mix in a level-matched listening comparison and verify the player in the installed Mac app.
