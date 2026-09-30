#!/usr/bin/env python3
"""Gate store retention on a new iCloud upload and an older isolated restore."""

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile

ARCHIVE = re.compile(r"store-\d{8}T\d{6}Z\.tar\.gz\Z")


def fail(message):
    raise RuntimeError(f"icloud-store-backup: {message}")


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def bounded_digest(path, seconds):
    command = ["shasum", "-a", "256"] if shutil.which("shasum") else ["sha256sum"]
    result = subprocess.run(command + [str(path)], check=True, capture_output=True,
                            text=True, timeout=seconds + 60)
    value = result.stdout.split(maxsplit=1)[0]
    if not re.fullmatch(r"[0-9a-f]{64}", value):
        fail("older archive checksum could not be read")
    return value


def regular_entry(path):
    mode = path.lstat().st_mode
    if not stat.S_ISREG(mode):
        fail("archive path is not a regular file")


def receipt_matches(path, receipt, target):
    # The target id is recorded but NOT required to match. An archive is identified by the
    # bytes its receipt recorded; a renamed systems.toml entry, or a destination now reached
    # through a symlink, would otherwise refuse every older archive forever, which silently
    # disables retention and hides the age problem it was meant to surface.
    if (
        receipt.get("capability") != "store"
        or receipt.get("tarball") != path.name
        or not isinstance(receipt.get("bytes"), int)
        or receipt["bytes"] <= 0
        or not re.fullmatch(r"[0-9a-f]{64}", receipt.get("sha256", ""))
    ):
        fail("older archive does not match its recorded receipt")


def cloud(swift_script, command, path, seconds):
    subprocess.run(
        ["swift", str(swift_script), command, str(path), str(seconds)],
        check=True,
        stdout=subprocess.DEVNULL,
        timeout=seconds + 60,
    )


def receipt_for_old(path, history, latest_receipt, target):
    stored = history / f"{path.name}.json"
    if stored.exists():
        with stored.open(encoding="utf-8") as stream:
            return json.load(stream), stored
    if latest_receipt.exists():
        with latest_receipt.open(encoding="utf-8") as stream:
            receipt = json.load(stream)
        if receipt.get("tarball") == path.name:
            return receipt, latest_receipt
    fail("older archive has no matching production receipt; retaining all archives")


def record_new(history, name, target, size, sha):
    history.mkdir(parents=True, exist_ok=True)
    path = history / f"{name}.json"
    receipt = {
        "capability": "store", "target": target, "tarball": name,
        "bytes": size, "sha256": sha,
    }
    if path.exists():
        fail("an immutable receipt already exists for the new archive")
    fd, temp = tempfile.mkstemp(dir=history, prefix=".store-receipt-")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            os.fchmod(stream.fileno(), 0o600)
            json.dump(receipt, stream)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.link(temp, path)
        directory_fd = os.open(history, os.O_RDONLY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        os.unlink(temp)


def main():
    if len(sys.argv) != 11:
        fail("usage: icloud-store-backup.py <directory> <new-name> <retain> <no-prune> <target> <overlay> <tools> <sha256> <bytes> <timeout>")
    _, directory, name, keep, no_prune, target, overlay, tools, sha, size, timeout = sys.argv
    if not ARCHIVE.fullmatch(name) or not re.fullmatch(r"[0-9a-f]{64}", sha):
        fail("invalid archive identity")
    keep, size, timeout = int(keep), int(size), int(timeout)
    if keep < 2 or size <= 0 or timeout < 1 or no_prune not in ("0", "1"):
        fail("invalid retention or size")
    directory = Path(directory)
    archive = directory / name
    regular_entry(archive)
    if archive.stat().st_size != size or digest(archive) != sha:
        fail("new destination bytes do not match the produced archive")
    tools = Path(tools)
    swift_script = tools / "icloud-item.swift"
    overlay = Path(overlay)
    history = overlay / "backup" / "receipts" / "history" / "store"
    # Preserve the producer's hash even if upload times out. A later run can then
    # re-check this archive without inventing a hash from its possibly changed bytes.
    record_new(history, name, target, size, sha)
    cloud(swift_script, "wait-upload", archive, timeout)
    print("  iCloud upload confirmed for new store archive", file=sys.stderr)
    if no_prune == "1":
        print("false")
        return

    candidates = sorted(
        (path for path in directory.iterdir() if ARCHIVE.fullmatch(path.name)),
        key=lambda path: path.name,
        reverse=True,
    )
    if not candidates or candidates[0] != archive:
        fail("new store archive is not newest; refusing retention")
    if len(candidates) <= keep:
        print("true")
        return

    old = candidates[1]
    regular_entry(old)
    prior, prior_path = receipt_for_old(
        old, history, overlay / "backup" / "receipts" / "store.json", target
    )
    receipt_matches(old, prior, target)
    cloud(swift_script, "wait-upload", old, timeout)
    cloud(swift_script, "wait-download", old, timeout)
    if old.stat().st_size != prior["bytes"] or bounded_digest(old, timeout) != prior["sha256"]:
        fail("older archive bytes do not match its receipt")
    with tempfile.TemporaryDirectory(prefix="sjel-store-rehearsal-") as scratch:
        subprocess.run(
            [str(tools / "restore.sh"), "store", str(old), "--receipt", str(prior_path),
             "--destination", str(Path(scratch) / "restored")],
            check=True,
            stdout=subprocess.DEVNULL,
            timeout=timeout + 60,
        )
    print("  preceding store archive restored in isolation", file=sys.stderr)
    # Removing a dataless item deletes its cloud copy too; never confuse this with eviction.
    for expired in candidates[keep:]:
        regular_entry(expired)
        try:
            cloud(swift_script, "check-upload", expired, timeout)
        except subprocess.SubprocessError as error:
            print(f"  warning: expired archive {expired.name} not confirmed in iCloud ({error}); unlinking anyway", file=sys.stderr)
        expired.unlink()
    print(f"  pruned {len(candidates) - keep} older store archive(s)", file=sys.stderr)
    print("true")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.SubprocessError, RuntimeError) as error:
        print(f"icloud-store-backup: {error}", file=sys.stderr)
        sys.exit(1)
