#!/usr/bin/env python3
"""Cut over a shared-store owner from a regular operator terminal."""

import argparse
from contextlib import closing
from dataclasses import dataclass
import fcntl
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parent.parent
TOOLS = ROOT / "tools"
SOURCE_KEYS = {b"SJEL_DB_PATH", b"SJEL_PERSONAL_ROOT", b"HOME"}


class Refused(RuntimeError):
    pass


@dataclass(frozen=True)
class Owner:
    """One capability that moves from the shared store to its own Turso WAL file."""

    name: str
    label: str
    manifest: Path
    package: str
    binary: str
    serve_args: tuple
    destination: str
    snapshot: str
    tables: tuple
    checks: tuple
    note: str

    @property
    def backup(self):
        return f"{self.name}-wal-backup"

    @property
    def env(self):
        return f"SJEL_{self.name.upper()}_DB_PATH"

    @property
    def legacy_build(self):
        return f'["cargo", "build", "--locked", "--release", "--bin", "{self.binary}"]'.encode()

    @property
    def owner_build(self):
        return (f'["cargo", "build", "--locked", "--release", "--no-default-features", "--features", '
                f'"turso-wal", "--bin", "{self.binary}"]').encode()


# Each check runs after activation; "@binary" names the owner's release binary.
OWNERS = {
    "health": Owner(
        name="health", label="Health", manifest=ROOT / "crates/health/service.toml",
        package="sjel-health", binary="health",
        serve_args=(b"serve",), destination="data/health/health.db",
        snapshot="data/health/snapshots/health.db", tables=("health_days", "health_imports"),
        checks=(("@binary", "metrics"), ("@binary", "summary")),
        note="Pause manual Health imports.",
    ),
    "projects": Owner(
        name="projects", label="Projects", manifest=ROOT / "crates/projects/service.toml",
        package="sjel-projects", binary="projects-server",
        serve_args=(), destination="data/projects-index/projects.db",
        snapshot="data/projects-wal-backup/projects.db",
        tables=("projects_inventory_links", "projects_mail_applications",
                "projects_mail_proposals", "projects_records"),
        checks=(("crates/projects/projects", "verify"), ("crates/projects/projects", "projects")),
        note="Close editors on Projects records.",
    ),
}


def run(*args, capture=False, quiet=False, environ=None):
    return subprocess.run(
        args, cwd=ROOT, check=True, text=True, env=environ,
        stdout=subprocess.PIPE if capture else subprocess.DEVNULL if quiet else None,
    ).stdout


def deployment():
    paths = run(
        "bash", "-c",
        'source "$1" && printf "%s\\0%s" "$SJEL_OVERLAY_ROOT" "$SJEL_MACHINE_TOML"',
        "_", TOOLS / "lib/paths.sh", capture=True,
    )
    return map(Path, paths.split("\0"))


def table(path):
    with path.open("rb") as f:
        return tomllib.load(f)


def local_target(overlay, machine, capability):
    override = table(machine).get("capability", {}).get(capability, {})
    contract = table(ROOT / "services" / capability / "service.toml")
    ident = override.get("backup_target") or contract.get("backup_target")
    target = table(overlay / "config/systems.local.toml").get(ident, {})
    if target.get("kind") != "local":
        raise Refused(f"{capability} requires a configured local backup target for this command")
    root = Path(target["path"]).expanduser()
    if not root.is_absolute() or not root.is_dir() or not os.access(root, os.W_OK):
        raise Refused(f"{capability} backup target is not an accessible local directory")
    return ident, root


def private_restore_workspace(overlay):
    scratch = overlay.resolve() / "data/.restore-scratch"
    if scratch.resolve() != scratch or scratch.is_relative_to(ROOT.resolve()):
        raise Refused("the private restore workspace must be inside overlay data, outside the checkout")
    scratch.mkdir(mode=0o700, parents=True, exist_ok=True)
    return tempfile.TemporaryDirectory(prefix="cutover-", dir=scratch)


def backup_restore(capability, overlay, machine, scratch, environ=None):
    target, target_root = local_target(overlay, machine, capability)
    receipts = overlay / "backup/receipts"
    latest = receipts / f"{capability}.json"
    previous = latest.read_bytes() if latest.is_file() else None
    run(TOOLS / "backup.sh", "--no-prune", "--target", target, capability, environ=environ)
    current = latest.read_bytes()
    if current == previous:
        raise Refused(f"{capability} did not produce a fresh backup receipt")
    entry = json.loads(current)
    archive = entry["tarball"]
    if entry["target"] != target or not re.fullmatch(rf"{capability}-\d{{8}}T\d{{6}}Z\.tar\.gz", archive):
        raise Refused(f"{capability} backup receipt does not match the requested target and archive")
    receipt = receipts / "history" / capability / f"{archive}.json"
    if json.loads(receipt.read_bytes()) != entry:
        raise Refused(f"{capability} current and per-archive receipts differ")
    run(TOOLS / "restore.sh", capability, target_root / capability / archive,
        "--receipt", receipt, "--private-scratch", "--destination", scratch)


def stage_shared(owner, overlay, machine, source, staged):
    with private_restore_workspace(overlay) as work:
        restore = Path(work) / "shared-restore"
        backup_restore("store", overlay, machine, restore, {**os.environ, "SJEL_DB_PATH": str(source)})
        run(sys.executable, TOOLS / "owner-store-split.py", owner.name, restore / "data/sjel/sjel.db", staged)


def compare_owner(owner, source, candidate, *, closed):
    for path in (candidate, source) if closed else (candidate,):
        wal = path.with_name(path.name + "-wal")
        if wal.is_file() and wal.stat().st_size > 32:
            raise Refused("cannot compare an owner file with uncheckpointed WAL frames")
    with (
        closing(sqlite3.connect(source.resolve().as_uri() + ("?immutable=1" if closed else "?mode=ro"), uri=True)) as source_db,
        closing(sqlite3.connect(candidate.resolve().as_uri() + "?immutable=1", uri=True)) as candidate_db,
    ):
        source_db.execute("BEGIN")
        for name in owner.tables:
            if rows(source_db, f"SELECT * FROM {name}") != rows(candidate_db, f"SELECT * FROM {name}"):
                raise Refused(f"{owner.label} rows changed; refusing to use the candidate")
        if sequences(source_db, owner.tables) != sequences(candidate_db, owner.tables):
            raise Refused(f"{owner.label} sequences changed; refusing to use the candidate")


def rows(db, query):
    return sorted(db.execute(query).fetchall(), key=repr)


def sequences(db, tables):
    if not db.execute("SELECT 1 FROM sqlite_master WHERE name = 'sqlite_sequence'").fetchone():
        return []
    names = ", ".join("?" for _ in tables)
    return db.execute(f"SELECT name, seq FROM sqlite_sequence WHERE name IN ({names}) ORDER BY name",
                      tables).fetchall()


def put_file(path, content, mode):
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as f:
        pending = Path(f.name)
        try:
            f.write(content)
            f.flush()
            os.fchmod(f.fileno(), mode)
            os.fsync(f.fileno())
            pending.replace(path)
        finally:
            pending.unlink(missing_ok=True)


def listener_pid(base):
    port = base.removeprefix("http://127.0.0.1:")
    if not port.isascii() or not port.isdecimal():
        raise Refused("the owner is not a local loopback service")
    listeners = set(run("lsof", "-nP", "-t", f"-iTCP:{port}", "-sTCP:LISTEN", capture=True).split())
    if len(listeners) != 1:
        raise Refused("cannot identify the single running owner process")
    return int(listeners.pop())


def prove_running_source(base, source):
    pid = str(listener_pid(base))
    try:
        opened = run("lsof", "-a", "-t", "-p", pid, "--", source, capture=True).split()
    except subprocess.CalledProcessError as error:
        raise Refused("the running owner did not prove it has the expected database open") from error
    if pid not in opened:
        raise Refused("the running owner did not prove it has the expected database open")


def source_settings(records):
    selected = {}
    for record in records:
        name, _, value = record.partition(b"=")
        if name not in SOURCE_KEYS:
            continue
        key = os.fsdecode(name)
        if key in selected:
            raise Refused("the running owner process has ambiguous database settings")
        selected[key] = os.fsdecode(value)
    return selected


def darwin_launch_context(pid):
    import ctypes
    import struct

    # CTL_KERN / KERN_PROCARGS2 returns argc, executable, argv and NUL-separated environment.
    # Keep only the path settings; never print the process environment.
    sysctl = ctypes.CDLL(None, use_errno=True).sysctl
    sysctl.argtypes = (ctypes.POINTER(ctypes.c_int), ctypes.c_uint, ctypes.c_void_p,
                       ctypes.POINTER(ctypes.c_size_t), ctypes.c_void_p, ctypes.c_size_t)
    mib = (ctypes.c_int * 3)(1, 49, pid)
    data = ctypes.create_string_buffer(os.sysconf("SC_ARG_MAX"))
    length = ctypes.c_size_t(len(data))
    if sysctl(mib, len(mib), data, ctypes.byref(length), None, 0) != 0:
        raise OSError(ctypes.get_errno(), "cannot inspect the running owner process settings")
    raw = data.raw[:length.value]
    if len(raw) < struct.calcsize("@i"):
        raise Refused("the running owner process did not provide complete launch settings")
    argc = struct.unpack_from("@i", raw)[0]
    executable, separator, tail = raw[struct.calcsize("@i"):].partition(b"\0")
    records = tail.lstrip(b"\0").split(b"\0")
    if not separator or not executable or not 0 < argc < len(records):
        raise Refused("the running owner process did not provide complete launch settings")
    return Path(os.fsdecode(executable)), records[1:argc], source_settings(records[argc:])


def process_launch_context(pid):
    if sys.platform == "darwin":
        return darwin_launch_context(pid)
    process = Path(f"/proc/{pid}")
    return (process / "exe", (process / "cmdline").read_bytes().rstrip(b"\0").split(b"\0")[1:],
            source_settings((process / "environ").read_bytes().split(b"\0")))


def launched_database(settings):
    def path_of(value):
        if value.startswith("~/") and settings.get("HOME"):
            value = str(Path(settings["HOME"]) / value[2:])
        path = Path(value)
        if not path.is_absolute():
            raise Refused("the running owner process does not declare an absolute database path")
        return path

    override = settings.get("SJEL_DB_PATH", "")
    if override.strip():
        return path_of(override)
    return path_of(settings.get("SJEL_PERSONAL_ROOT", "")) / "data/sjel/sjel.db"


def prove_legacy_source(owner, base, source, binary):
    # libSQL closes its connection after a request. Its actual launch settings are the proof.
    pid = listener_pid(base)
    fields = run("lsof", "-nP", "-a", "-d", "txt", "-p", str(pid), "-Fn", capture=True)
    # Cargo can replace the file at this path while the old owner process is still running.
    # Check the executable path, not the inode of the newer artifact on disk.
    expected = binary.resolve()
    text_paths = {Path(line[1:].removesuffix(" (deleted)")).resolve()
                  for line in fields.splitlines() if line.startswith("n/")}
    if expected not in text_paths:
        raise Refused(f"the listener is not the deployed {owner.label} executable")
    executable, args, settings = process_launch_context(pid)
    if args != list(owner.serve_args) or Path(str(executable.resolve()).removesuffix(" (deleted)")) != expected:
        raise Refused(f"the listener is not the deployed {owner.label} service")
    if not launched_database(settings).samefile(source) or listener_pid(base) != pid:
        raise Refused(f"the running {owner.label} service points at a different shared database or changed during preflight")


def preflight(owner, overlay, machine, source, destination, binary, base):
    try:
        with source.open("rb") as f:
            if f.read(16) != b"SQLite format 3\x00":
                raise Refused("the shared source is not a SQLite database")
    except OSError as error:
        raise Refused("cannot read the live shared store; run outside Pi or Claude Code in a Mac owner terminal") from error
    prove_legacy_source(owner, base, source, binary)
    if os.path.lexists(destination):
        raise Refused(f"{owner.label} owner path already exists; inspect it before attempting a cutover")
    machine_env = table(machine).get("capability", {}).get(owner.name, {}).get("env", [])
    declaration = overlay / "config/deployment.env"
    deployment_keys = set()
    if declaration.is_file():
        with declaration.open() as f:
            deployment_keys = {line.partition("=")[0] for line in f}
    if (owner.env in os.environ or owner.env in deployment_keys
            or any(value.startswith(owner.env + "=") for value in machine_env)):
        raise Refused(f"this command requires the default {owner.label} owner path, without an override")
    before = owner.manifest.read_bytes()
    if before.count(owner.legacy_build) != 1:
        raise Refused(f"{owner.label} service build is not the reviewed legacy declaration")
    try:
        run("git", "diff", "HEAD", "--quiet", "--", owner.manifest)
    except subprocess.CalledProcessError as error:
        raise Refused(f"{owner.label} service manifest has local changes; review them before applying") from error
    credential = run(TOOLS / "capability-auth/capability-auth", "--check", owner.name, capture=True)
    if credential.strip() != "configured":
        raise Refused("the owner credential needed for verification is not available in this terminal")
    for contract in ("store", owner.backup):
        local_target(overlay, machine, contract)
    return before


def verify_owner(owner, overlay, machine, destination, binary, base):
    prove_running_source(base, destination)
    run("curl", "-qfsS", "--noproxy", "*", "--max-time", "10", "-o", "/dev/null", base + "/ready")
    for program, *args in owner.checks:
        run(binary if program == "@binary" else ROOT / program, *args, quiet=True)
    with private_restore_workspace(overlay) as work:
        restore = Path(work) / "owner-restore"
        backup_restore(owner.backup, overlay, machine, restore)
        snapshot = Path(owner.snapshot)
        compare_owner(owner, overlay / snapshot, restore / snapshot, closed=True)
    run(TOOLS / "capability.sh", "enable", owner.backup)
    print(f"{owner.label} ready. Live checks passed. Owner backup restored with exact row parity.", flush=True)


def build_candidate(owner, target):
    target = target.resolve()
    if target.is_relative_to(ROOT.resolve()) or any(target.iterdir()):
        raise Refused(f"{owner.label} requires an empty build directory outside the checkout")
    build_context = {**os.environ, "CARGO_BUILD_BUILD_DIR": str(target)}
    # This is an owner release build, like distribution preparation. The live target is not an output.
    run(*json.loads(owner.owner_build), "--package", owner.package, "--target-dir", target, environ=build_context)
    return target / "release" / owner.binary


def apply(owner, overlay, machine, source, destination, binary, base, before):
    with tempfile.TemporaryDirectory(prefix=f"sjel-{owner.name}-build-", dir="/tmp") as target:
        candidate = build_candidate(owner, Path(target))
        activate(owner, overlay, machine, source, destination, binary, base, before, candidate)


def activate(owner, overlay, machine, source, destination, binary, base, before, candidate_binary):
    new_binary = candidate_binary.read_bytes()
    prove_legacy_source(owner, base, source, binary)
    if owner.manifest.read_bytes() != before or os.path.lexists(destination):
        raise Refused("the deployment changed during the build")

    destination.parent.mkdir(parents=True, exist_ok=True)
    after = before.replace(owner.legacy_build, owner.owner_build)
    old_binary = binary.read_bytes()
    promoted = switched = False
    with tempfile.TemporaryDirectory(prefix=".cutover-", dir=destination.parent) as work:
        work = Path(work)
        staged = work / Path(owner.destination).name
        try:
            run(TOOLS / "service-runner.sh", "stop", owner.name)
            stage_shared(owner, overlay, machine, source, staged)
            run(candidate_binary, "check-owner", environ={**os.environ, owner.env: str(staged)})
            compare_owner(owner, source, staged, closed=False)
            status = run(TOOLS / "service-runner.sh", "status", owner.name, capture=True)
            if "stopped, held" not in status or "no answer" not in status:
                raise Refused(f"the {owner.label} maintenance hold is not active")
            if owner.manifest.read_bytes() != before or binary.read_bytes() != old_binary:
                raise Refused(f"{owner.label} build or binary changed during maintenance")
            os.link(staged, destination)
            promoted = True
            put_file(owner.manifest, after, 0o644)
            put_file(binary, new_binary, 0o755)
            switched = True
            run(TOOLS / "service-runner.sh", "resume", owner.name)
            verify_owner(owner, overlay, machine, destination, binary, base)
        except BaseException:
            if switched:
                print(f"{owner.label} was switched to the owner file. Do not resume the shared build; new writes may exist. Repair the owner and run --verify.", file=sys.stderr)
            else:
                if owner.manifest.read_bytes() == after and promoted:
                    put_file(owner.manifest, before, 0o644)
                if promoted and binary.read_bytes() == new_binary:
                    put_file(binary, old_binary, 0o755)
                if promoted and destination.samefile(staged):
                    destination.unlink()
                if owner.manifest.read_bytes() == before and binary.read_bytes() == old_binary:
                    run(TOOLS / "service-runner.sh", "resume", owner.name)
                    print(f"Cutover stopped before activation. Legacy {owner.label} resumed.", file=sys.stderr)
                else:
                    print(f"Concurrent build changes detected. {owner.label} remains held for operator review.", file=sys.stderr)
            raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("owner", choices=tuple(OWNERS))
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true", help="run preflight without changing anything (default)")
    mode.add_argument("--apply", action="store_true", help="perform the cutover after interactive confirmation")
    mode.add_argument("--verify", action="store_true", help="retry live checks and the first owner backup after cutover")
    args = parser.parse_args()
    owner = OWNERS[args.owner]
    overlay, machine = deployment()
    source = Path(os.environ.get("SJEL_DB_PATH") or overlay / "data/sjel/sjel.db").expanduser().resolve()
    destination = overlay / owner.destination
    binary = ROOT / "target/release" / owner.binary
    base = run(ROOT / "sjel", "capability", "url", owner.name, capture=True).strip()
    if not args.verify:
        before = preflight(owner, overlay, machine, source, destination, binary, base)
        print(f"{owner.label} preflight passed: readable running source, legacy build and local backup targets.", flush=True)
    if not (args.apply or args.verify):
        return
    if not sys.stdin.isatty() or os.environ.get("SJEL_AGENT") == "1":
        raise Refused("use a regular owner terminal, not an agent or a piped session")
    prompt = (f"{owner.note} This holds the service and changes the tracked {owner.label} build to Turso WAL. Type {owner.name} to continue: "
              if args.apply else f"Run and restore a new {owner.label} owner backup. Type {owner.name} to continue: ")
    if input(prompt) != owner.name:
        raise Refused("not confirmed; no cutover or backup started")
    locks = overlay / "backup/locks"
    locks.mkdir(parents=True, exist_ok=True)
    with (locks / f"{owner.name}-cutover.flock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise Refused(f"another {owner.label} cutover or verification is running") from error
        if args.apply:
            before = preflight(owner, overlay, machine, source, destination, binary, base)
            apply(owner, overlay, machine, source, destination, binary, base, before)
        else:
            if owner.owner_build not in owner.manifest.read_bytes() or not destination.is_file():
                raise Refused(f"{owner.label} has not been cut over")
            verify_owner(owner, overlay, machine, destination, binary, base)


if __name__ == "__main__":
    os.umask(0o077)
    try:
        main()
    except (OSError, Refused, ValueError, KeyError, sqlite3.Error, subprocess.CalledProcessError) as error:
        sys.exit(f"owner-store-cutover: {error}")
