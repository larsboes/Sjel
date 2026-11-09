---
name: interactive-figures
description: Authors self-contained interactive SVG and HTML explainers driven by one explicit model. Use when explaining a mechanism with controls, making an Anatomy-style technical illustration, or building an interactive figure for a note, article or teaching task. Do not use for Mermaid rendering (diagrams), data plots without a mechanism, complete slide decks (slide-deck), general application UI (ui-craftsmanship), or running a lesson (teach).
license: MIT
allowed-tools: Read, Write, Edit, Bash
---

# Interactive Figures

Make the interaction explain one claim. Use enough detail to make the mechanism legible.
Do not require a part count, isometric styling, WebGL or a rendering framework.
Use a physical analogy only when its behavior matches the concept.

## Define the figure

State the claim, inputs, outputs, units and assumptions before drawing.
Separate measured facts from illustrative values and idealized behavior.
Choose a static figure when interaction adds no understanding, unless interaction is requested.
Keep required behavior and accessibility when simplifying the design.

## Build one source of truth

Put the behavior in a small pure model. Derive geometry, captions and readouts from its state.
Test boundary values and one intermediate state. Reject invalid inputs.
Use HTML controls and SVG before adding libraries or shaders.
Deliver a standalone HTML page with inline styles and scripts when portability is required.
Avoid remote assets, network calls and publication unless explicitly requested and authorized.
Follow the artifact's palette or host theme rather than impose an unrelated visual identity.

Use labelled, keyboard-operable controls, visible focus, text descriptions and non-colour cues.
Keep readouts understandable without seeing the drawing.
For animation, respect reduced motion, pause offscreen work and dispose listeners and timers.
A still fallback must explain the same claim when animation or WebGL is unavailable.

## Check the result in its destination

Check the model independently, then compare rendered labels and geometry with that model.
Exercise input changes, narrow and wide layouts, applicable themes and required fallback states.
Read captured images. Browser DOM checks do not establish visual quality.
A browser preview does not establish that an embedded figure works in its actual host.

For an Obsidian destination, use `obsidian` to resolve the vault and writing contract first.
Use the host's documented embedding and lifecycle contract. A full HTML document needs a
renderer that preserves document-level styles and scripts. Do not guess native embed syntax.
When the contract or host is unavailable, deliver the standalone figure and mark embedding
unverified. Do not write vault assets or notes until their placement is authorized.

## Local example

The example is an ideal lever, not a dynamics simulation. One model sets its forces and geometry.
From this skill directory:

```bash
node scripts/build-example.mjs
node scripts/build-example.mjs --check
bun test scripts/example.test.ts
node scripts/check-example.mjs <screenshot-directory>
```

The browser check needs installed Chrome or Chromium, or `CHROME_BIN`. It downloads nothing.
It uses temporary browser profiles and synthetic input events. It does not verify native
keyboard actions or an Obsidian embed. Open `assets/lever.html` for the standalone figure.

## Stop and report

Stop when the claim, requested interaction and relevant checks hold.
Report the delivered files, assumptions, checks that ran and destination checks that did not.
Do not add decorative machinery or broaden the task to a gallery, framework or teaching system.
Upstream influence and licence scope are recorded in the owning Pack README and notices.
