# Moonraker REST — quick reference

For ad-hoc calls beyond printctl. Base: `http://$HOST`. Export `HOST` from the overlay first (keeps the LAN IP out of Sjel):
```bash
HOST=$(uv run --python 3 python -c 'import json,os;c=json.load(open(os.path.expanduser(os.environ["SJEL_PERSONAL_ROOT"])+"/config/printing.json"));print(f"{c[\"printer_host\"]}:{c[\"moonraker_port\"]}")')
```
Fluidd uses the same API. All read endpoints are safe; POST endpoints that heat/move should go through printctl's arm gate, not raw curl.

## Contents
- Status & info
- Files
- Print control
- Emergency

## Status & info
```bash
curl -s http://$HOST/server/info            # is Moonraker up
curl -s http://$HOST/printer/info           # klippy state
# live temps + progress:
curl -s 'http://$HOST/printer/objects/query?print_stats&heater_bed&extruder&display_status&toolhead'
```

## Files (gcodes root)
```bash
curl -s 'http://$HOST/server/files/list?root=gcodes'
# upload (printctl upload wraps this multipart POST):
curl -F 'file=@model.gcode' http://$HOST/server/files/upload
```

## Print control (heat/move — prefer printctl so the arm gate applies)
```bash
curl -X POST 'http://$HOST/printer/print/start?filename=model.gcode'
curl -X POST http://$HOST/printer/print/pause
curl -X POST http://$HOST/printer/print/resume
curl -X POST http://$HOST/printer/print/cancel
```

## Emergency
```bash
curl -X POST http://$HOST/printer/emergency_stop   # halts Klipper; requires FIRMWARE_RESTART after
```
