---
project: printing
type: isa
effort: E2
phase: complete
progress: 7/7
mode: build
---

# ISA · printing

The capability is built and running. This file is its current state: what it guarantees, how
each guarantee is probed, and what has not been decided yet. `README.md` beside it holds the
verdict, the stack, and the experience notes; none of that is repeated here.

## Problem

A tool that shells out to a slicer and then commands a machine which heats to 260°C is the
prompt-in-shell-out threat class pointed at physical hardware. The risk is not a bad print. It
is an agent, or a typo, starting an unattended heat cycle nobody authorised.

## Vision

The whole loop runs from the terminal without the vendor cloud, and the dangerous steps cannot
be reached by accident: temperature ceilings are enforced in code rather than trusted from a
preset, and heat is gated behind an explicit human "go" that is a separate command, not a flag.

## Claims

- [x] P1 · A nozzle temperature above the configured ceiling is refused.
      *Probe:* `printctl selftest` → "nozzle 300°C over cap: correctly refused".
- [x] P2 · A bed temperature above the configured ceiling is refused.
      *Probe:* same run → "bed 120°C over cap: correctly refused".
- [x] P3 · Temperatures within the caps are allowed, so the gate is not simply always-off.
      *Probe:* same run → "250/70 within caps: allowed".
- [x] P4 · Caps are enforced against the gcode's real temperatures, not the requested preset.
      *Probe:* same run → "gcode parse -> nozzle 250.0, bed 70.0 (want 250,70)".
- [x] P5 · Starting an unarmed job is refused; arming is the human "go".
      *Probe:* same run → "unarmed start guard: correctly refused".
- [x] P6 · `arm` cannot authorize a job it could not temp-check. Two paths used to arm silently:
      no `--local`, and a `--local` that does not resolve to a file. The second was the worse one,
      because a typo'd path looked like a check that had happened. Both now refuse, exit 1.
      *Probe:* `printctl selftest` → "arm without --local", "arm with a --local path that is not a
      file", "arm with over-cap gcode" all refused, and "arm with within-cap gcode: allowed" so the
      gate is not simply always-off. The positive case writes to a throwaway arm file; the real one
      at `$SJEL_PERSONAL_ROOT/data/printing/.armed.json` was absent before and after.
- [x] P7 · Nothing personal lives in Sjel: host, presets and caps resolve from the overlay.
      *Probe:* `rg '([0-9]{1,3}\.){3}[0-9]{1,3}|[a-z0-9-]+\.local\b|homepi'` over the capability,
      minus `args.local` false positives → no hits; the example config carries the literal
      `PRINTER_LAN_IP` placeholder and `printctl.py` resolves `$SJEL_PERSONAL_ROOT`. Validate the
      pattern against a planted fixture before trusting an empty result: bare `rg -E` is
      `--encoding` and errors out silently-looking, which reads exactly like "clean".

## Anti-claims

- The slicer is never invoked through a shell string. Arg array only.
- No capability code stores a printer host, preset name or cap value.
- "Full auto" never extends to heat. Arming stays a separate, explicit act.

## Not yet specified

- **Where the model comes from.** The loop starts at "you already have an STL", and the
  `3d-printing` pack's own description says *"Do not use for CAD/mesh modeling."* Nothing in
  Sjel owns the step before slicing.

  One lead exists and is not committed work: **VibeCAD**, a working prototype of Lars's from
  June 2026, 15 files and 72 KB, retired off-repo by the 2026-08-19 skill-packs sweep. It has
  no durable home yet, and until it earns one this section is the only record of what it was —
  which is why the description below is written to stand without the code beside it.
  A Bun server, a Three.js/CSG viewer with a parameter panel and STL export, and
  a library where one file is one parametric model exporting `meta` plus a typed `params`
  schema the UI renders as sliders. The agent writes the model file; the browser live-reloads.

  Three things would have to be settled before it is more than a lead: whether it is a
  capability of its own or a UI over this one, whether it hands an STL to `printctl` or stays
  separate, and the fact that it loads Three.js and CSG from a CDN, which needs internet and
  breaks the self-contained rule. Vendoring those is a precondition, not a detail.

- Whether `KlipperMCP`'s config-write features are worth cherry-picking, as `README.md`
  suggested and nothing has revisited.
