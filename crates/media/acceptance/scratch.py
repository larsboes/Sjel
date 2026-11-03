#!/usr/bin/env python3
"""Black-box F0/F1/F2-absence probes with a disposable tree, never the live library.

Usage: python3 capabilities/media/acceptance/scratch.py MEDIA_BIN
"""
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path


def call(binary, db, *args, expected=0):
    proc = subprocess.run([str(binary), *map(str, args), "--db", str(db)], text=True, capture_output=True)
    assert proc.returncode == expected, (proc.args, proc.returncode, proc.stdout, proc.stderr)
    return json.loads(proc.stdout) if proc.stdout else None


def disk_files(root):
    return sorted(Path(folder) / name for folder, _, names in os.walk(root) for name in names)


def main(binary):
    with tempfile.TemporaryDirectory(prefix="media-acceptance-") as tmp:
        root = Path(tmp)
        library, staging, db = root / "library", root / "staging", root / "store.db"
        (library / "by-date/2020-01").mkdir(parents=True)
        (staging / "originals").mkdir(parents=True)
        (library / "by-date/2020-01/20200101.jpg").write_bytes(b"old!")
        (staging / "originals/20200101.jpg").write_bytes(b"new!")  # same name/size, different bytes
        (staging / "originals/duplicate.jpg").write_bytes(b"old!")
        (staging / "originals/game.avi").write_bytes(b"game asset")
        (staging / "export-manifest.json").write_text(json.dumps({"source": "acceptance", "refusals": [
            {"path": "game.avi", "reason": "not personal media", "evidence": "operator-labelled game asset"}
        ]}))
        uuid = json.loads(subprocess.check_output([str(binary), "volume-id", "--root", str(library)], text=True))["uuid"]
        wrong_db = root / "wrong-mount.db"
        for verb in (("index", "--root", library), ("ingest", "--staging", staging, "--library", library, "--apply")):
            refused = subprocess.run([str(binary), *map(str, verb), "--uuid", "DEAD-BEEF", "--db", str(wrong_db)], capture_output=True)
            assert refused.returncode != 0 and not wrong_db.exists(), refused.stderr
        first = call(binary, db, "index", "--root", library, "--uuid", uuid)
        assert first["hashed"] == 1 and first["disk_files"] == len(disk_files(library))
        assert first["indexed_locations"] == len(disk_files(library))
        second = call(binary, db, "index", "--root", library, "--uuid", uuid)
        assert second["hashed"] == 0 and call(binary, db, "status")["files"] == 1
        poison = root / "poison"
        (poison / "originals").mkdir(parents=True)
        (poison / "originals/new.jpg").write_bytes(b"source")
        (poison / "export-manifest.json").write_text(json.dumps({"source": "symlink fixture"}))
        target = library / "by-date/2020-01/20200101.jpg"
        (poison / "staging-hashes.tsv").symlink_to(target)
        proc = subprocess.run([str(binary), "ingest", "--staging", str(poison), "--library", str(library),
                               "--uuid", uuid, "--db", str(db)], capture_output=True)
        assert proc.returncode != 0 and target.read_bytes() == b"old!"
        (poison / "staging-hashes.tsv").unlink()
        os.link(target, poison / "staging-hashes.tsv")
        dry_hardlink = call(binary, db, "ingest", "--staging", poison, "--library", library, "--uuid", uuid)
        assert dry_hardlink["imported"] == 1 and target.read_bytes() == b"old!"
        audit = call(binary, db, "audit", "--root", library, "--uuid", uuid, "--sample", 1)
        assert audit["disagreements"] == [] and audit["sampled"] == 1
        dry = call(binary, db, "ingest", "--staging", staging, "--library", library, "--uuid", uuid, expected=1)
        assert (dry["imported"], dry["duplicates"], dry["refused"]) == (1, 1, 1)
        assert not (library / "by-date/2020-01/20200101~2.jpg").exists()
        before = hashlib.sha256((library / "by-date/2020-01/20200101.jpg").read_bytes()).hexdigest()
        assert call(binary, db, "classify", "--digest", before.upper()) == {"digest": before, "present": True}
        applied = call(binary, db, "ingest", "--staging", staging, "--library", library, "--uuid", uuid, "--apply", expected=1)
        assert (applied["considered"], applied["imported"], applied["duplicates"], applied["refused"], applied["verified"]) == (3, 1, 1, 1, 1)
        assert sum(applied[x] for x in ("imported", "duplicates", "refused", "failed", "resumed")) == applied["considered"]
        assert hashlib.sha256((library / "by-date/2020-01/20200101.jpg").read_bytes()).hexdigest() == before
        assert (library / "by-date/2020-01/20200101~2.jpg").read_bytes() == b"new!"
        assert len(disk_files(library)) == call(binary, db, "status")["locations"]
        audit = call(binary, db, "audit", "--root", library, "--uuid", uuid, "--sample", 2)
        assert audit["disagreements"] == [] and audit["sampled"] == 2
        fresh = root / "fresh-staging"
        (fresh / "originals").mkdir(parents=True)
        (fresh / "originals/old.jpg").write_bytes(b"old!")
        (fresh / "originals/new.jpg").write_bytes(b"new!")
        (fresh / "export-manifest.json").write_text(json.dumps({"source": "fresh duplicate fixture"}))
        duplicated = call(binary, db, "ingest", "--staging", fresh, "--library", library, "--uuid", uuid, "--apply", "--prune")
        assert (duplicated["considered"], duplicated["imported"], duplicated["duplicates"]) == (2, 0, 2)
        assert not duplicated["pruned"] and (fresh / "originals").exists()
        import sqlite3
        with sqlite3.connect(db) as conn:
            items = conn.execute("SELECT disposition,reason FROM media_ingest_items WHERE ingest_id=(SELECT max(id) FROM media_ingests)").fetchall()
        assert len(items) == 2 and all(kind == "duplicate" and reason for kind, reason in items)
        again = call(binary, db, "ingest", "--staging", staging, "--library", library, "--uuid", uuid, "--apply", "--prune", expected=1)
        assert again["imported"] == 0 and again["resumed"] == 1 and not again["pruned"]
        assert (staging / "originals").exists()
        absent = call(binary, db, "verify-mirror", "--left-root", library,
                      "--left-uuid", first["uuid"], "--right-root", root / "not-mounted", "--right-uuid", "DEAD-BEEF")
        assert absent["availability"].startswith("absent:") and absent["discrepancies"] == [] and absent["checked"] == 0
        wrong_mount = call(binary, db, "verify-mirror", "--left-root", library,
                           "--left-uuid", first["uuid"], "--right-root", library, "--right-uuid", "DEAD-BEEF")
        assert wrong_mount["availability"].startswith("absent:") and wrong_mount["discrepancies"] == []
        (library / "by-date/2020-01/20200101.jpg").write_bytes(b"evil")
        corrupted = call(binary, db, "audit", "--root", library, "--uuid", uuid, "--sample", 2, expected=1)
        assert any("digest differs" in finding for finding in corrupted["disagreements"])
        print("scratch PASS: UUID mismatch before DB open, symlink/hardlink report safety, index idempotence, independent disk count, sampled hashing, digest-only collision, disposition conservation, verified resume, no prune on refusal or duplicates, absent mirror, corruption detection")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(Path(sys.argv[1]).resolve())
