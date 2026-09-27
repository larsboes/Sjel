# Home 3D Printing — One-time Setup

## Contents
- 1. Config (printer + caps)
- 2. OrcaSlicer user presets (required for headless slicing)
- 3. Why raw system presets fail
- 4. Install the skill

## 1. Config
The printer host, OrcaSlicer binary path, safety caps (nozzle ≤ 260 °C, bed ≤ 90 °C), and the three preset names live in the **private overlay** at `$SJEL_PERSONAL_ROOT/config/printing.json` (shape: `capabilities/printing/printctl.config.example.json`). printctl reads it at runtime — no personal value is stored in Axon. Control commands work immediately; only `slice` needs the presets below.

## 2. OrcaSlicer user presets (required for `slice`)
OrcaSlicer's CLI rejects the bundled system presets — it needs presets **saved as user presets** from the GUI. Once:
1. Open OrcaSlicer, select printer **Elegoo Neptune 4 (0.4 nozzle)**.
2. Pick process **0.20mm Strength @Elegoo N4 0.4 nozzle**; tune for the part (e.g. 4–6 walls, 40–50% gyroid infill, brim on for small parts). Click **Save** → name it e.g. `VanMoof-PETG-strength`.
3. Pick filament **Elegoo PETG @EN4 Series** (or Generic PETG). **Save**.
4. Set the saved names in the overlay config (`$SJEL_PERSONAL_ROOT/config/printing.json`):
   ```json
   "preset_printer": "Elegoo Neptune 4 0.4 nozzle",
   "preset_process": "VanMoof-PETG-strength",
   "preset_filament": "Elegoo PETG @EN4 Series"
   ```
5. Verify: `printctl doctor` shows ✓ next to each preset.

## 3. Why raw system presets fail
OrcaSlicer 2.4.2's CLI compatibility validator rejects dynamically-loaded system profiles with `run 2652: process not compatible with printer`. Flattening the inheritance chain, blanking the compatibility condition, and pointing at the real datadir were all tried and all still trip the gate — verified 2026-07-09. Saved user presets (or slicing a project 3MF that already bundles paired settings) is the working path. printctl's `slice` therefore loads user presets by name from `~/Library/Application Support/OrcaSlicer/user/`.

## 4. Install / discovery
This skill ships in the Axon **3d-printing pack** (`Packs/3d-printing/skills/home-3d-printing/`).
Deploy the pack into your active harness so it loads everywhere:
```bash
"$SJEL_ROOT/tools/packs-claude" deploy 3d-printing   # copies into ~/.claude/skills/
```
`tools/packs-claude status 3d-printing` reports whether the deployed copy is current. It is a
copy, not a symlink — the symlink model was retired 2026-08-09, and the deployer now refuses a
symlinked source outright. Use `tools/harnesses status` to see every pack against every harness
at once.
