# 3d-printing pack

Slice + drive a home Klipper/Moonraker 3D printer safely from the terminal.

- **`home-3d-printing`** — runbook over the `printing` capability (`capabilities/printing/printctl.py`): doctor → slice (OrcaSlicer) → upload → arm → start → monitor, with hard temp caps and an arm-before-heat gate.

Works with **any** Klipper/Moonraker printer. Printer host, model, nozzle, material and safety caps live in the overlay (`$SJEL_PERSONAL_ROOT/config/printing.json`, shape: `capabilities/printing/printctl.config.example.json`) — `printctl doctor` surfaces them and live-discovers the printer when it's on. Nothing printer-specific is baked into the skill.

## Activate
```bash
"$SJEL_ROOT/tools/packs.sh" link 3d-printing   # → ~/.claude/skills/home-3d-printing
"$SJEL_ROOT/tools/packs-codex" deploy 3d-printing  # → ~/.agents/skills/home-3d-printing
cp "$SJEL_ROOT/capabilities/printing/printctl.config.example.json" \
   "$SJEL_PERSONAL_ROOT/config/printing.json"   # then fill in your printer
```

## Attribution
Original work. The safety framing (Prompt-In-Shell-Out mitigation: arg-array slicer invocation, temp caps, arm gate) was informed by two MCP audit tools surveyed pre-adoption (mcp-audit, mcpserver-audit — reviewed locally, archived outside this repo) — no code from them is vendored here. License: MIT.

## MCP servers considered and declined

Surveyed 2026-07-09 while deciding how to drive the printer, before `printctl` was built. Verdict: **CLI-first — no MCP layer.** Fluidd is just a UI over Moonraker, and Moonraker is already a full local REST API; `printctl` talks to it directly (`capabilities/printing/printctl.py`), so there's no wrapper-of-a-wrapper to maintain, no extra process to keep alive, and no third-party code with shell/gcode execution paths sitting between the agent and the printer. All three repos were pulled locally for review, evaluated, and removed once the verdict was recorded here — nothing is vendored.

| Repo | What it does | Why declined |
|---|---|---|
| [mcp-3D-printer-server](https://github.com/DMontgomery40/mcp-3D-printer-server) (GPLv2) | Multi-vendor printer control + STL editing + slicing + Bambu MQTT, as an MCP server | Broad multi-vendor surface we don't need (single Klipper printer); Bambu-specific parts are dead weight; GPLv2 is a worse license fit than building our own MIT tool |
| [KlipperMCP](https://github.com/mikehatch/KlipperMCP) (MIT) | Klipper config read/write + macro/Jinja template eval + raw gcode execution, as an MCP server | Config-write + arbitrary gcode + Jinja eval from an LLM tool call is real Prompt-In-Shell-Out surface (untrusted input → shell-adjacent execution) with no arm-gate or temp-cap equivalent to `printctl`'s; read-only status/control is all we actually need |
| [klipper-config-mcp](https://github.com/grego33/klipper-config-mcp) (MIT) | Read-only Klipper config/log/docs access, as an MCP server | Strict subset of KlipperMCP's read path — redundant even if KlipperMCP had been adopted |

If an MCP layer is ever justified (e.g. a second printer, or multi-agent concurrent access), reconsider `KlipperMCP`'s macro/gcode surface first — cheapest cherry-pick — and gate any addition through an `upstreams.toml` verdict plus `tools/audit` (README.md#dependency-verdicts-and-provenance) before it touches a live printer.
