---
name: anatomy
description: Build crafted, highly detailed, interactive isometric SVG figures that explain a technical idea as a real physical object (a test rig, a dial gauge, a cabinet of drawers, a plotter, a lock), with many small precise solid parts, four-tone shading, correct painter's order, a framed card with title, hint and live readout, and calm eased motion. Use when someone wants an isometric illustration, an explanatory diagram or figure for docs, a blog post, a landing page or an article, a "Linear-style" or "premium" technical drawing, an interactive explainer, or an SVG that shows how a system, algorithm or component works, even if they never say "isometric". Works framework-free (TypeScript/JS that writes SVG and HTML) and in React/Next.js, and can add realistic WebGL shaders (fire, exhaust, water, caustics, lamp light) that live inside the drawing in the same isometric perspective. Do not use for data plots, slide decks (use slide-deck), or standard Mermaid diagrams (use diagrams).
license: MIT
compatibility: Written for Claude Code; also runs in Antigravity, OpenCode, Codex and MiniMax Code. Needs Node.js 20.10+, and Chrome or Chromium for the checks.
metadata:
  opencode/slash: "true"
---

# Anatomy

This skill makes figures like a small instrument on a bench: a heavy base on feet, rails with tick rulers, a carriage on bushings, knurled knobs, coil springs, a dial gauge whose needle moves, screws as tiny dots. The reader understands the idea because they recognise the machine. Every number the figure shows comes from the same model that moves its parts.

Eight complete examples ship with the skill. Open the closest one before you start:
- `examples/dial-indicator/`: smallest gauge stand (~300 lines).
- `examples/desk-computer/`: framework-free SVG CRT and keyboard (85 solids, 850 paths).
- `examples/arcade-cabinet/`: hero cutaway with coin chute physics and CRT warmup.
- `examples/raptor-engine/`: dense lathe/tube pipework with WebGL plume and audit proofs.
- `examples/ripple-tank/`: 44° elevation dispersion optics and caustics.
- `examples/test-rig/`: production React/Next.js carriage and dial gauge.
- `examples/spot-plate/`: morphing glass drops driven by distance fields.
- `examples/turning-dial/`: 3D mode turntable with robotic hand.

## The one idea that matters

**The object is the explanation.** Don't draw boxes, arrows and labels. Invent a real, well-made physical thing whose working *is* the concept.

- **Read the real thing**: every screw, seam, bevel, bushing, label plate, cable with connector, and tick ruler.
- **Two looks**: at 1× it reads as one clear machine; at 4× every region has true details to discover.
- **Check it last**: zoom to 4× before finishing to ensure no bare face or empty corner remains.

## Workflow

1. **Size it before you start**:
   - **Figure**: one mechanism, 40–80 parts, SVG with at most one WebGL layer (up to 1h).
   - **Hero**: dense object, multiple WebGL layers or 3D subject, 80–150 parts (2–4h).
   - **Epic**: whole vehicle, plant, or multi-subject 3D (6h+). Propose a Hero first.
2. **Truth first**: Collect real numbers and formulas. Drive motion and readout from a pure model module.
3. **Invent the object**: Pick a machine whose mechanism maps one-to-one to the concept.
4. **Parts list**: Group by base, structure, mechanism, controls, and subject.
5. **Plan the world**: Lay out world units on floor (x, y) and up (z). Fit with `fitProjection()`.
6. **Build geometry once**: Precompute static paths with `kit/lathe.mjs`, `kit/tube.mjs`, and `kit/iso-kit.mjs`.
7. **Paint back to front, bottom to top**: Depth-sort using `depthOf`, `A.recorder`, `A.settle`, and `A.auditOrExit`.
8. **Frame it**: Put the figure in a card with Fig n, title, hint, live readout, and caption.
9. **Make it live**: Calm time-based motion, exponential follow or springs, respecting reduced-motion.
10. **Verify like a critic**:
    - Capture desktop and phone (390px) with `scripts/capture.mjs`.
    - Run geometry audit to zero: `node build.mjs --audit`.
    - Run line check to zero: `node scripts/lines.mjs page.html --out shots/lines.png`.
    - 4× close-ups of every junction in dark and light themes (`--light`).

## Setup and Kit

The core toolkit lives in `kit/`:
- `kit/iso-kit.mjs` / `kit/iso-kit.ts`: Camera, solids, SVG string renderer, card/page builders, and CSS.
- `kit/lathe.mjs`: Turned round parts along any axis, smooth shading, visible ribs and bolt circles.
- `kit/tube.mjs`: Depth-sortable pipe/cable chunks, collars, clamps, fittings, and braids.
- `kit/audit.mjs`: Proves clearances, intersections, loose ends, folds, and painter's order.
- `kit/gl.mjs`: World-space WebGL shaders for fire, caustics, and glow inside the isometric camera.
- `kit/turn.mjs` & `kit/turn-build.mjs`: 3D turning mode with separated planes and orbit audit.

Choose standalone ESM build scripts (`build.mjs`) for HTML/SVG output, or copy `kit/` into React/Next.js projects (`references/react.md`).

## Craft, WebGL, and 3D Doctrine

- **Craft Rules**: See `references/craft.md` for full tone hierarchies, bevels, grooves, screw spacing, and pipe routing rules.
- **WebGL Shaders**: See `references/webgl.md` for world-space camera registration, zero-pop envelopes, and light-theme paper inks.
- **3D Turning Models**: See `references/3d.md` for turntable assemblies, separating planes, and orbit audit verification.
- **Verification Checklist**: See `references/verify.md` for screenshots, line-end checks, and shader contact sheets.
## References

Read these as you need them:

- `references/craft.md`: the look in depth, including the parts vocabulary with code, tone tables, painter's order patterns, round parts, pipes and harnesses, exploded views, screens on faces, and the anti-patterns that got figures rejected. Read it before designing.
- `references/kit.md`: the API, with signatures and examples.
- `references/motion.md`: live behaviour (loops, springs, tours, pointer, keyboard, accessibility, reduced motion), plus envelopes for one-shot events, exploded views, depth re-sorting and arming, in vanilla JS and React.
- `references/react.md`: the React/Next.js server/client split, element props and the large-payload key-warning trap.
- `references/verify.md`: the `capture.mjs` and `drive.mjs` commands, the review checklist (including shaders) and common faults.
- `references/webgl.md`: shaders inside the drawing: when one earns its place, the layer stack, the camera contract, ray-marching in the iso camera, the full API with a minimal page, the craft rules with the bugs that taught them, the light theme (inks on paper, per-phenomenon recipes, the reviewers' faults), and how to verify them.
- `references/walkthrough.md`: how the test rig was designed, step by step, from concept to code.
- `references/3d.md`: the optional 3D mode: groups and poses, live layers, convex parts and their rules, materials, free groups, hinge chains, painter's order, static clutter around a turntable, shaders that follow the turn, the orbit audit, browser checks, performance and limits.
- `references/publish.md`: sharing a finished figure in the community gallery with `scripts/publish.mjs`, only when the user asks; nothing is pushed for them.
