# printing

Home 3D printing, driven safely from the terminal: slice, upload, arm, print, monitor an
Elegoo Neptune 4 (Klipper/Moonraker) without touching the vendor cloud.

## Verdict

**Build (CLI), adopt the stack under it.** Moonraker is already a full local REST API and
Fluidd is just a UI over it, so an MCP-server layer on top would have been a wrapper of
wrappers. A ~small stdlib-only Python CLI (`printctl`) over OrcaSlicer's headless slicer +
Moonraker's API covers the whole loop. Considered and declined: mcp-3D-printer-server
(multi-vendor + Bambu MQTT baggage), KlipperMCP (config-write features worth cherry-picking
later), klipper-config-mcp (subset of the former).

## What I run

- **Slicer:** [OrcaSlicer](https://github.com/SoftFever/OrcaSlicer), headless CLI
- **Firmware/API:** Klipper + [Moonraker](https://github.com/Arksine/moonraker), Fluidd as UI
- **CLI:** `printctl` (doctor / slice / upload / arm / start / monitor / estop), safety-first:
  hard temp caps enforced on the gcode at slice time and again at arm time, `arm` refusing
  outright if it cannot read the file to check it, and heating refusing unless the job was
  explicitly armed first

## Experience (the parts nobody tells you)

- **OrcaSlicer's CLI rejects raw system presets** (`process not compatible with printer`).
  I tried seven workarounds; all fail its compatibility gate. The fix: save presets once in
  the GUI as *user* presets, then load those by name. Plan for this on day one.
- A printer tool that runs gcode and shells out to a slicer IS the prompt-in-shell-out threat
  class. Invoke the slicer with an arg array (no shell string), cap temperatures in code, and
  gate heat behind an explicit arm step: "full-auto once you say go" is the right amount of auto.
- PETG is hygroscopic; dry the spool before a print that matters.

## Run it

`printctl.py` lives here (stdlib-only Python, no installs). Config is resolved from the overlay at runtime — copy `printctl.config.example.json` to `$SJEL_PERSONAL_ROOT/config/printing.json` and fill in the printer host + saved OrcaSlicer preset names there; nothing personal is stored in Sjel. The `3d-printing` pack (`Packs/3d-printing/`) is the agent-facing runbook over this tool.

```bash
python3 capabilities/printing/printctl.py selftest   # offline safety check → ALL PASS ✓
python3 capabilities/printing/printctl.py doctor      # printer reachable? presets? caps?
```

## Links

- [Moonraker API docs](https://moonraker.readthedocs.io/): the actual integration surface
- [mcp-audit](https://github.com/apisec-inc/mcp-audit): run its source-scan before ever
  exposing this capability as an MCP server
