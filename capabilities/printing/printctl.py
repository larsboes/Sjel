#!/usr/bin/env python3
"""
printctl - local, safety-first CLI for the home 3D printer.

One core, two halves:
  * SLICE  : wraps OrcaSlicer CLI (headless) -> .gcode
  * CONTROL: talks to Moonraker REST on the LAN (Fluidd is just a UI on top of it)

Safety layer (always on):
  * hard temperature ceilings enforced before any heat command AND on sliced gcode
  * heat/move commands are gated: they refuse unless a job was armed with `arm`
    (full-auto-once-you-say-go: `arm` once, then start/monitor runs unattended)

Stdlib only. No pip installs. Python 3.9+.
"""
from __future__ import annotations
import argparse, json, os, sys, glob, subprocess, time, zipfile, urllib.request, urllib.parse, urllib.error

# ------------------------------------------------------------------ config
# Axon doctrine: this file is public. No personal value (printer IP, preset
# names) lives here — those come from the private overlay at runtime. The
# config is resolved in order:
#   1. $SJEL_PRINTING_CONFIG (explicit override)
#   2. $SJEL_PERSONAL_ROOT/config/printing.json (the overlay; SJEL_PERSONAL_ROOT
#      is exported by tools/lib/paths.sh / ~/.zshrc)
#   3. printctl.config.json next to this file (local, gitignored — dev fallback)
# See capabilities/printing/printctl.config.example.json for the shape.
HOME = os.path.expanduser("~")
_HERE = os.path.dirname(os.path.abspath(__file__))

# OrcaSlicer's binary and data dir are OS-specific. These are only defaults — either can
# be overridden in overlay config (orca_bin / orca_datadir). macOS uses the .app bundle;
# on Linux/WSL OrcaSlicer ships as a distro package or AppImage on PATH.
if sys.platform == "darwin":
    _ORCA_BIN = "/Applications/OrcaSlicer.app/Contents/MacOS/OrcaSlicer"
    _ORCA_DATADIR = os.path.join(HOME, "Library/Application Support/OrcaSlicer")
else:
    _ORCA_BIN = "orca-slicer"
    _ORCA_DATADIR = os.path.join(
        os.environ.get("XDG_CONFIG_HOME", os.path.join(HOME, ".config")), "OrcaSlicer"
    )

DEFAULT_CFG = {
    "printer_host": "",       # REQUIRED via overlay config; empty = not configured
    "moonraker_port": 7125,
    # descriptive setup facts — set per your printer, or discover live via `doctor`.
    # printctl works with any Klipper/Moonraker printer; these just label the setup.
    "printer_model": "",      # e.g. "Elegoo Neptune 4"; blank -> discovered from Moonraker
    "nozzle_mm": None,        # e.g. 0.4
    "material": "",           # default loaded filament, e.g. "PETG"
    "orca_bin": _ORCA_BIN,
    "orca_datadir": _ORCA_DATADIR,
    # safety ceilings — a print whose temps exceed these is refused, full stop.
    "nozzle_max_c": 260,
    "bed_max_c": 90,
    # user preset names to slice against (saved once from the OrcaSlicer GUI, see README)
    "preset_printer": None,   # e.g. "Elegoo Neptune 4 0.4 nozzle"
    "preset_process": None,   # e.g. "VanMoof-PETG-strength"
    "preset_filament": None,  # e.g. "Elegoo PETG @EN4 Series"
    # where an armed job token lives — resolved to the overlay in load_cfg so
    # transient runtime state never lands in the public Axon repo.
    "arm_file": "",
}

def _arm_file():
    overlay = os.environ.get("SJEL_PERSONAL_ROOT")
    base = os.path.join(os.path.expanduser(overlay), "data", "printing") if overlay \
        else os.path.join(HOME, ".cache", "printctl")
    os.makedirs(base, exist_ok=True)
    return os.path.join(base, ".armed.json")

def _cfg_path():
    env = os.environ.get("SJEL_PRINTING_CONFIG")
    if env:
        return env
    overlay = os.environ.get("SJEL_PERSONAL_ROOT")
    if overlay:
        return os.path.join(os.path.expanduser(overlay), "config", "printing.json")
    return os.path.join(_HERE, "printctl.config.json")

def load_cfg():
    cfg = dict(DEFAULT_CFG)
    path = _cfg_path()
    if os.path.isfile(path):
        try:
            cfg.update(json.load(open(path)))
        except Exception as e:
            die(f"bad config {path}: {e}")
    if not cfg.get("arm_file"):
        cfg["arm_file"] = _arm_file()
    return cfg

def die(msg, code=1):
    print(f"✗ {msg}", file=sys.stderr)
    sys.exit(code)

def ok(msg):
    print(f"✓ {msg}")

# ------------------------------------------------------------------ moonraker
class Moonraker:
    def __init__(self, cfg):
        if not cfg.get("printer_host"):
            die("printer_host not configured. Set it in the overlay config "
                f"({_cfg_path()}) — see capabilities/printing/printctl.config.example.json.")
        self.base = f"http://{cfg['printer_host']}:{cfg['moonraker_port']}"
        self.timeout = 6

    def _req(self, path, method="GET", data=None):
        url = self.base + path
        body = None
        headers = {}
        if data is not None and not isinstance(data, (bytes, bytearray)):
            body = json.dumps(data).encode()
            headers["Content-Type"] = "application/json"
        elif isinstance(data, (bytes, bytearray)):
            body = data
        req = urllib.request.Request(url, data=body, method=method, headers=headers)
        try:
            # self.base is hardcoded to an "http://" prefix (line 93) and path is
            # always a literal at each call site -- the scheme can never become
            # file:// regardless of printer_host/moonraker_port's (trusted, local
            # overlay config) values, so the file-read risk this rule flags can't occur.
            with urllib.request.urlopen(req, timeout=self.timeout) as r:  # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected
                raw = r.read()
                try:
                    return json.loads(raw)
                except Exception:
                    return {"raw": raw.decode(errors="replace")}
        except urllib.error.URLError as e:
            raise ConnectionError(f"Moonraker unreachable at {self.base} ({e.reason}). "
                                  f"Printer off/asleep or not on this LAN?")
        except Exception as e:
            raise ConnectionError(f"Moonraker error at {url}: {e}")

    def reachable(self):
        try:
            self._req("/server/info")
            return True
        except Exception:
            return False

    def discover(self):
        """Best-effort live identity of the printer (hostname, klippy state)."""
        out = {}
        try:
            out["hostname"] = self._req("/machine/system_info").get(
                "system_info", {}).get("cpu_info", {}).get("hostname")
        except Exception:
            pass
        try:
            out["klippy"] = self._req("/printer/info").get("state")
        except Exception:
            pass
        return out

    def status(self):
        q = "/printer/objects/query?" + urllib.parse.urlencode({
            "print_stats": "", "heater_bed": "", "extruder": "",
            "display_status": "", "toolhead": "",
        }).replace("=&", "&").rstrip("=")
        return self._req(q).get("result", {}).get("status", {})

    def files(self):
        return self._req("/server/files/list?root=gcodes").get("result", [])

    def upload(self, local_path, remote_name=None):
        remote_name = remote_name or os.path.basename(local_path)
        boundary = "----printctl" + str(int(time.time()))
        with open(local_path, "rb") as f:
            content = f.read()
        parts = []
        parts.append(f"--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; "
                     f"filename=\"{remote_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n".encode())
        parts.append(content)
        parts.append(f"\r\n--{boundary}--\r\n".encode())
        payload = b"".join(parts)
        req = urllib.request.Request(self.base + "/server/files/upload", data=payload, method="POST",
                                     headers={"Content-Type": f"multipart/form-data; boundary={boundary}"})
        # same reasoning as Moonraker._req above -- hardcoded "http://" prefix, literal path.
        with urllib.request.urlopen(req, timeout=30) as r:  # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected
            return json.loads(r.read())

    def print_start(self, filename):
        return self._req("/printer/print/start", "POST", {"filename": filename})

    def pause(self):  return self._req("/printer/print/pause", "POST", {})
    def resume(self): return self._req("/printer/print/resume", "POST", {})
    def cancel(self): return self._req("/printer/print/cancel", "POST", {})
    def estop(self):  return self._req("/printer/emergency_stop", "POST", {})

# ------------------------------------------------------------------ safety
def check_temps(cfg, nozzle, bed, ctx=""):
    """Refuse anything above ceilings. Returns nothing; dies on violation."""
    if nozzle is not None and nozzle > cfg["nozzle_max_c"]:
        die(f"SAFETY{(' '+ctx) if ctx else ''}: nozzle {nozzle}°C exceeds ceiling {cfg['nozzle_max_c']}°C. Refusing.")
    if bed is not None and bed > cfg["bed_max_c"]:
        die(f"SAFETY{(' '+ctx) if ctx else ''}: bed {bed}°C exceeds ceiling {cfg['bed_max_c']}°C. Refusing.")

def gcode_temps(path_or_bytes):
    """Extract max nozzle (M104/M109 S) and bed (M140/M190 S) from gcode text."""
    if isinstance(path_or_bytes, (bytes, bytearray)):
        text = path_or_bytes.decode(errors="replace")
    else:
        with open(path_or_bytes, errors="replace") as f:
            text = f.read(200000)  # header is enough
    noz = bed = 0
    for line in text.splitlines():
        s = line.strip().upper()
        if s.startswith(("M104", "M109")) and " S" in s:
            try: noz = max(noz, float(s.split(" S")[1].split()[0]))
            except Exception: pass
        elif s.startswith(("M140", "M190")) and " S" in s:
            try: bed = max(bed, float(s.split(" S")[1].split()[0]))
            except Exception: pass
    return noz, bed

def is_armed(cfg, filename):
    af = cfg["arm_file"]
    if not os.path.isfile(af):
        return False
    try:
        d = json.load(open(af))
    except Exception:
        return False
    return d.get("filename") == filename and d.get("expires", 0) > time.time()

# ------------------------------------------------------------------ slicing
def find_user_preset(cfg, kind, name):
    """Locate a saved user preset json in the OrcaSlicer datadir."""
    if not name:
        return None
    roots = [os.path.join(cfg["orca_datadir"], "user")]
    for root in roots:
        for fp in glob.glob(os.path.join(root, "**", f"{name}.json"), recursive=True):
            if f"/{kind}/" in fp or kind in os.path.basename(os.path.dirname(fp)):
                return fp
        hit = glob.glob(os.path.join(root, "**", f"{name}.json"), recursive=True)
        if hit:
            return hit[0]
    return None

def cmd_slice(cfg, args):
    orca = cfg["orca_bin"]
    if not os.path.isfile(orca):
        die(f"OrcaSlicer not found at {orca}")
    stl = os.path.abspath(args.stl)
    if not os.path.isfile(stl):
        die(f"STL not found: {stl}")
    out3mf = os.path.abspath(args.out or (os.path.splitext(stl)[0] + ".3mf"))
    gcode_out = os.path.splitext(out3mf)[0] + ".gcode"

    mp = find_user_preset(cfg, "machine", cfg["preset_printer"])
    pp = find_user_preset(cfg, "process", cfg["preset_process"])
    fp = find_user_preset(cfg, "filament", cfg["preset_filament"])
    if not (mp and pp and fp):
        die("Slicing needs saved OrcaSlicer USER presets (printer/process/filament).\n"
            "  OrcaSlicer's CLI rejects raw system presets ('process not compatible with printer').\n"
            "  One-time fix: in OrcaSlicer GUI pick Neptune 4 + your PETG + a strength process,\n"
            "  tune walls/infill, then 'Save preset' for each, and set preset_printer/process/filament\n"
            "  in printctl.config.json. See README.")

    cmd = [orca, "--datadir", cfg["orca_datadir"],
           "--load-settings", f"{mp};{pp}", "--load-filaments", fp,
           "--arrange", "1", "--slice", "0",
           "--export-3mf", out3mf, "--outputdir", os.path.dirname(out3mf), stl]
    print("→ slicing:", " ".join(f'"{c}"' if " " in c else c for c in cmd))
    r = subprocess.run(cmd, capture_output=True, text=True)
    if not os.path.isfile(out3mf):
        die(f"slice failed (rc={r.returncode}):\n{(r.stdout + r.stderr)[-800:]}")
    # extract embedded plate gcode
    with zipfile.ZipFile(out3mf) as z:
        gc = [n for n in z.namelist() if n.lower().endswith(".gcode")]
        if not gc:
            die("sliced 3mf has no gcode (empty plate?)")
        data = z.read(sorted(gc)[0])
    with open(gcode_out, "wb") as f:
        f.write(data)
    noz, bed = gcode_temps(data)
    check_temps(cfg, noz, bed, ctx="(sliced gcode)")
    ok(f"sliced → {gcode_out}  (nozzle {noz:.0f}°C / bed {bed:.0f}°C, within caps {cfg['nozzle_max_c']}/{cfg['bed_max_c']})")
    return gcode_out

# ------------------------------------------------------------------ commands
def cmd_doctor(cfg, args):
    print("printctl doctor")
    print(f"  printer      : {cfg['printer_host']}:{cfg['moonraker_port']}")
    m = Moonraker(cfg)
    reach = m.reachable()
    print(f"  moonraker    : {'REACHABLE ✓' if reach else 'unreachable (printer off/asleep or off-LAN)'}")
    model = cfg.get("printer_model") or "(unset)"
    noz = f", {cfg['nozzle_mm']}mm nozzle" if cfg.get("nozzle_mm") else ""
    mat = f", {cfg['material']}" if cfg.get("material") else ""
    print(f"  setup        : {model}{noz}{mat}")
    if reach:
        disc = m.discover()
        if disc.get("hostname") or disc.get("klippy"):
            print(f"  discovered   : host {disc.get('hostname','?')}, klippy {disc.get('klippy','?')}")
    print(f"  orca bin     : {'found ✓' if os.path.isfile(cfg['orca_bin']) else 'MISSING'} {cfg['orca_bin']}")
    print(f"  orca datadir : {'found ✓' if os.path.isdir(cfg['orca_datadir']) else 'MISSING'}")
    for k in ("preset_printer", "preset_process", "preset_filament"):
        v = cfg[k]
        found = find_user_preset(cfg, k.split("_")[1], v) if v else None
        print(f"  {k:13}: {v or '(unset)'} {'✓' if found else ('' if not v else '— not found in datadir')}")
    print(f"  safety caps  : nozzle ≤ {cfg['nozzle_max_c']}°C, bed ≤ {cfg['bed_max_c']}°C")
    if reach:
        st = m.status()
        ps = st.get("print_stats", {})
        ext = st.get("extruder", {})
        bedh = st.get("heater_bed", {})
        print(f"  state        : {ps.get('state','?')}  nozzle {ext.get('temperature','?')}→{ext.get('target','?')}  "
              f"bed {bedh.get('temperature','?')}→{bedh.get('target','?')}")

def cmd_status(cfg, args):
    m = Moonraker(cfg)
    st = m.status()
    ps = st.get("print_stats", {}); ds = st.get("display_status", {})
    ext = st.get("extruder", {}); bed = st.get("heater_bed", {})
    print(json.dumps({
        "state": ps.get("state"),
        "filename": ps.get("filename"),
        "progress_pct": round((ds.get("progress") or 0) * 100, 1),
        "nozzle": [ext.get("temperature"), ext.get("target")],
        "bed": [bed.get("temperature"), bed.get("target")],
    }, indent=2))

def cmd_files(cfg, args):
    for f in Moonraker(cfg).files():
        print(f"  {f.get('path')}  ({round(f.get('size',0)/1024)} KB)")

def cmd_upload(cfg, args):
    r = Moonraker(cfg).upload(args.gcode, args.name)
    ok(f"uploaded: {json.dumps(r.get('item', r))}")

def cmd_arm(cfg, args):
    """Full-auto-once-you-say-go: arm a specific gcode so start/heat is allowed for a window.

    Arming is the only human 'go', so the temperature check is not optional here. Both holes
    below used to arm silently: no --local at all, and a --local that does not resolve to a
    file. The second is the worse one -- a typo'd path looks like a check that happened.
    """
    fn = args.filename
    if not args.local:
        die(f"REFUSED: arming '{fn}' requires the gcode, so its temperatures can be checked.\n"
            f"  Run `printctl arm {fn} --local <gcode>`.\n"
            f"  (caps are enforced on the sliced file, never on the preset that produced it.)")
    if not os.path.isfile(args.local):
        die(f"REFUSED: --local '{args.local}' is not a file. Refusing to arm unchecked.")
    noz, bed = gcode_temps(args.local)
    check_temps(cfg, noz, bed, ctx="(arm check)")
    payload = {"filename": fn, "expires": time.time() + args.minutes * 60,
               "nozzle": noz, "bed": bed}
    json.dump(payload, open(cfg["arm_file"], "w"))
    ok(f"ARMED '{fn}' for {args.minutes} min. `printctl start {fn}` will now run unattended.")

def _guard_heat(cfg, filename):
    if not is_armed(cfg, filename):
        die(f"REFUSED: '{filename}' is not armed. Run `printctl arm {filename} --local <gcode>` first.\n"
            f"  (full-auto-once-you-say-go: arming IS your 'go'.)")

def cmd_start(cfg, args):
    _guard_heat(cfg, args.filename)
    r = Moonraker(cfg).print_start(args.filename)
    ok(f"print started: {args.filename}  {r.get('result','')}")

def cmd_pause(cfg, args):  ok(Moonraker(cfg).pause().get("result"))
def cmd_resume(cfg, args):
    _guard_heat(cfg, args.filename or "")
    ok(Moonraker(cfg).resume().get("result"))
def cmd_cancel(cfg, args): ok(Moonraker(cfg).cancel().get("result"))
def cmd_estop(cfg, args):
    ok(Moonraker(cfg).estop().get("result", "EMERGENCY STOP sent"))

def cmd_monitor(cfg, args):
    m = Moonraker(cfg)
    while True:
        st = m.status(); ps = st.get("print_stats", {}); ds = st.get("display_status", {})
        ext = st.get("extruder", {}); bed = st.get("heater_bed", {})
        pct = round((ds.get("progress") or 0) * 100, 1)
        print(f"[{time.strftime('%H:%M:%S')}] {ps.get('state'):>10}  {pct:5.1f}%  "
              f"noz {ext.get('temperature',0):.0f}/{ext.get('target',0):.0f}  "
              f"bed {bed.get('temperature',0):.0f}/{bed.get('target',0):.0f}  {ps.get('filename','')}")
        if ps.get("state") in ("complete", "cancelled", "error", "standby") and pct in (0, 100):
            break
        time.sleep(args.interval)

# ------------------------------------------------------------------ selftest
def cmd_selftest(cfg, args):
    """Verify safety logic without a printer."""
    fails = 0
    # 1. temp ceiling refuses
    import io, contextlib
    def expect_die(fn, label):
        nonlocal fails
        try:
            fn(); print(f"  ✗ {label}: did NOT refuse"); fails += 1
        except SystemExit:
            print(f"  ✓ {label}: correctly refused")
    expect_die(lambda: check_temps(cfg, 300, 60), "nozzle 300°C over cap")
    expect_die(lambda: check_temps(cfg, 240, 120), "bed 120°C over cap")
    # 2. within caps passes
    try:
        check_temps(cfg, 250, 70); print("  ✓ 250/70 within caps: allowed")
    except SystemExit:
        print("  ✗ 250/70 wrongly refused"); fails += 1
    # 3. gcode temp parse
    sample = b"M140 S70\nM104 S250\nM109 S250\nM190 S70\nG1 X0\n"
    n, b = gcode_temps(sample)
    print(f"  {'✓' if (n,b)==(250,70) else '✗'} gcode parse -> nozzle {n}, bed {b} (want 250,70)")
    if (n, b) != (250, 70): fails += 1
    # 4. arm guard refuses unarmed
    expect_die(lambda: _guard_heat(cfg, "definitely-not-armed.gcode"), "unarmed start guard")

    # 5. arm refuses to authorize anything it could not temp-check.
    #    The positive case really writes an arm file, so it gets a throwaway one -- a selftest
    #    must never leave a live job armed on the real printer.
    import tempfile, types
    def arm_args(local, minutes=1):
        return types.SimpleNamespace(filename="selftest.gcode", local=local, minutes=minutes)
    with tempfile.TemporaryDirectory() as td:
        tcfg = dict(cfg); tcfg["arm_file"] = os.path.join(td, "arm.json")
        expect_die(lambda: cmd_arm(tcfg, arm_args(None)), "arm without --local")
        expect_die(lambda: cmd_arm(tcfg, arm_args(os.path.join(td, "nope.gcode"))),
                   "arm with a --local path that is not a file")
        hot = os.path.join(td, "hot.gcode")
        open(hot, "w").write("M104 S300\nM140 S120\nG1 X0\n")
        expect_die(lambda: cmd_arm(tcfg, arm_args(hot)), "arm with over-cap gcode")
        safe = os.path.join(td, "safe.gcode")
        open(safe, "w").write("M104 S250\nM140 S70\nG1 X0\n")
        try:
            cmd_arm(tcfg, arm_args(safe))
            armed = is_armed(tcfg, "selftest.gcode")
            print(f"  {'✓' if armed else '✗'} arm with within-cap gcode: allowed")
            if not armed: fails += 1
        except SystemExit:
            print("  ✗ arm with within-cap gcode: wrongly refused"); fails += 1

    print(f"\n{'ALL PASS ✓' if fails==0 else f'{fails} FAILURE(S) ✗'}")
    sys.exit(1 if fails else 0)

# ------------------------------------------------------------------ cli
def main():
    cfg = load_cfg()
    p = argparse.ArgumentParser(prog="printctl", description="local safety-first 3D print control")
    sub = p.add_subparsers(dest="cmd", required=True)

    sub.add_parser("doctor", help="check printer + slicer + presets + caps").set_defaults(fn=cmd_doctor)
    sub.add_parser("status", help="current printer state (json)").set_defaults(fn=cmd_status)
    sub.add_parser("files", help="list gcode files on printer").set_defaults(fn=cmd_files)
    sub.add_parser("selftest", help="verify safety logic offline").set_defaults(fn=cmd_selftest)

    s = sub.add_parser("slice", help="STL -> gcode via OrcaSlicer (user presets)")
    s.add_argument("stl"); s.add_argument("-o", "--out"); s.set_defaults(fn=cmd_slice)

    s = sub.add_parser("upload", help="upload gcode to printer")
    s.add_argument("gcode"); s.add_argument("-n", "--name"); s.set_defaults(fn=cmd_upload)

    s = sub.add_parser("arm", help="authorize a job for unattended heat/print")
    s.add_argument("filename"); s.add_argument("--local", help="local gcode to temp-check")
    s.add_argument("--minutes", type=int, default=720); s.set_defaults(fn=cmd_arm)

    s = sub.add_parser("start", help="start a print (requires arm)")
    s.add_argument("filename"); s.set_defaults(fn=cmd_start)

    sub.add_parser("pause", help="pause print").set_defaults(fn=cmd_pause)
    s = sub.add_parser("resume", help="resume print (requires arm)")
    s.add_argument("filename", nargs="?"); s.set_defaults(fn=cmd_resume)
    sub.add_parser("cancel", help="cancel print").set_defaults(fn=cmd_cancel)
    sub.add_parser("estop", help="EMERGENCY STOP").set_defaults(fn=cmd_estop)

    s = sub.add_parser("monitor", help="live print monitor")
    s.add_argument("--interval", type=int, default=10); s.set_defaults(fn=cmd_monitor)

    args = p.parse_args()
    try:
        args.fn(cfg, args)
    except ConnectionError as e:
        die(str(e))

if __name__ == "__main__":
    main()
