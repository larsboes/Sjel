"""Run the reviewed scanner on one synthetic leaf and return its report."""

import json
import pathlib
import subprocess
import sys

report = pathlib.Path("/tmp/scan.json")
scan = subprocess.run(
    ["skill-scanner", "scan", sys.argv[1], "--policy", "balanced", "--cel-mode", "off",
     "--format", "json", "--output", str(report), "--fail-on-severity", "medium"],
    capture_output=True, text=True, timeout=45,
)
if not report.exists():
    print(json.dumps({"status": "failed", "exit": scan.returncode, "error": scan.stderr[-2000:]}))
    sys.exit(2)
result = json.loads(report.read_text())
print(json.dumps({"status": "reported", "exit": scan.returncode, "report": result}))
