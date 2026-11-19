---
id: soundscape-context
title: "Shape Soundscape sessions around listening intent"
state: verify
area: product
kind: feature
priority: 3
spec: docs/plans/product.md
evidence:
  - "The conductor passes 18 tests, including pause and resume from the same timer checkpoint."
  - "The native core passes 55 Soundscape tests and both offline signal quality checks."
  - "Eight Mac drawer tests pass, covering session progress, adaptive mix, pointer and keyboard resizing, and Escape dismissal."
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
- Accept the mix in a level-matched listening comparison and verify the player in the installed Mac app.
