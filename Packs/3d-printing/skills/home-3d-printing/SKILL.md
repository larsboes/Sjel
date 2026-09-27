---
name: home-3d-printing
description: Slices models and drives a home Klipper/Moonraker 3D printer via the local printctl CLI — slice with OrcaSlicer, upload, arm, start, monitor, pause/cancel, e-stop — with hard temperature caps and an arm-before-heat gate. Works with any Klipper/Moonraker printer; the specific model, material and caps come from config (run `printctl doctor`). Use when the user wants to 3D-print at home, slice an STL/3MF, or check/control the printer. Do not use for CAD/mesh modeling, cloud/non-local printers, or non-Klipper firmware.
allowed-tools: Read, Bash
---

# Home 3D Printing

Drive the printer through **`$PC`** (printctl) — the source of truth; this skill is the runbook. Physical hardware that heats and moves: safety gates are not optional, and no step may route around them.

**This setup is not hardcoded.** Printer host, model, nozzle, material and safety caps all live in `$SJEL_PERSONAL_ROOT/config/printing.json` and are surfaced (and, when the printer is on, live-discovered) by `$PC doctor`. **Run `$PC doctor` first** and read the real values off it — never assume a printer model or temp from this document.

## Variables
| var | value | notes |
|-----|-------|-------|
| `$PC` | `scripts/printctl` | launcher → `$SJEL_ROOT/capabilities/printing/printctl.py` (runs via uv) |
| `$HOST` `$MODEL` `$MATERIAL` `$NOZZLE_MAX` `$BED_MAX` | *(from config)* | host, printer model, loaded filament, hard temp caps — all read from the overlay config; `$PC doctor` prints them. Never inline them here. |
| `$STL` | — | model to slice (`.stl`/`.3mf`) |
| `$GCODE` | `${STL%.*}.gcode` | slice output, sits next to `$STL` |
| `$FN` | `$(basename $GCODE)` | filename as the printer sees it (arm/start key) |
| `$WALLS` / `$INFILL` | `4–6` / `40–50% gyroid` | generic strength-part guidance, not a fixed setting |

## Safety model — non-negotiable
- **Caps** (`$NOZZLE_MAX`/`$BED_MAX`) enforced on sliced gcode and at `arm`. Above → refused.
- **Arm-before-heat:** `start`/`resume` refuse unless `$FN` was `arm`ed. Arming IS the human "go"; after it, start+monitor run unattended within caps.
- Runaway temp/motion → `$PC estop` immediately (halts Klipper; needs FIRMWARE_RESTART after).
- Never curl the heat/move endpoints directly to skip the gate.

## Preflight — gate on the output
`$PC doctor`, then branch:
- `moonraker: REACHABLE ✓` → continue.
- `moonraker: unreachable` → printer off/asleep or off-LAN. **Stop, ask the user to power it on.** Do not retry blindly.
- any preset shows `(unset)` or `not found` → headless slicing unconfigured. Read `references/setup.md`, stop at the one-time GUI step (needs the user).

## Print workflow
```
- [ ] 1. doctor green (reachable + presets ✓)
- [ ] 2. slice   → $GCODE, temps ≤ caps
- [ ] 3. upload  → printer
- [ ] 4. arm     → state plan, get user go
- [ ] 5. start + monitor
```
1. `$PC slice $STL` → writes `$GCODE`, prints resolved nozzle/bed. Refuses over caps. "process not compatible with printer" → config points at system presets, not saved user presets → `references/setup.md`.
2. `$PC upload $GCODE`.
3. State part + `$MATERIAL` + expected temps to the user; on their go: `$PC arm $FN --local $GCODE` (re-checks temps).
4. `$PC start $FN` then `$PC monitor`. `start` REFUSED → `$FN` wasn't armed (gate working) → do step 3.

## Other commands
`$PC status` (json) · `$PC files` · `$PC pause` · `$PC resume $FN` · `$PC cancel` · `$PC estop`. Ad-hoc Moonraker beyond printctl → `references/moonraker-api.md`.

## Strength parts (mounts, brackets under load)
Strength process preset, `$WALLS` walls, `$INFILL`, `$MATERIAL` (PETG/ASA, not PLA), orient so layer lines are **not** parallel to the pull force, brim on small parts. Worked example (personal): `$SJEL_PERSONAL_ROOT/documents/vanmoof-s3-phone-mount-3d-print.md`.

## Provenance and maintenance
Verified 2026-07-09: printer at `$HOST` (overlay config), OrcaSlicer 2.4.2, `$PC` → `$SJEL_ROOT/capabilities/printing/printctl.py`. Re-verify on drift:
- reachable + config: `$PC doctor`
- safety gates intact: `$PC selftest` (must end `ALL PASS ✓`)
- printctl path: `test -f "$SJEL_ROOT/capabilities/printing/printctl.py" && echo ok`
- OrcaSlicer preset limitation is a standing constraint, not a bug → `references/setup.md` §3.
