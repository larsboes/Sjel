#!/usr/bin/env python3
"""Compare media's exact lookup with an independent SQLite/hashlib implementation.

Usage: python3 capabilities/media/acceptance/labelled.py MEDIA_BIN AUDIT_TRAIL_DIR [LIBRARY_ROOT]
Reads only the 2026-09-29 labelled TSVs. Creates a temporary synthetic database;
never opens the operator's shared store or writes to either library volume.
"""
import csv
import hashlib
import json
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path


def rows(path):
    with path.open(newline="", encoding="utf-8") as handle:
        return list(csv.DictReader(handle, delimiter="\t"))


def relocations(trail):
    """Recorded renames: old relative path -> current relative path.

    A fixture that stores a path is stale by construction, and this one proved it: the
    2026-09-29 relayout renamed `by-date/2019-07/IMG_5791 (1).MP4` to `IMG_5791.MP4`,
    and the live comparison silently degraded to 3 of 4 the moment it did. That is the
    same failure the capability exists to prevent -- a pathname is not an identity.

    relayout-library.py recorded all 5,723 moves in the audit trail this script already
    reads, so the fixture follows the record instead of being hand-edited every time a
    path changes. The durable fix is to resolve by digest once F0's live index exists;
    until then this keeps the four live comparisons real rather than quietly fewer.
    """
    moves = {}
    path = trail / "relayout-report.tsv"
    if path.is_file():
        for row in rows(path):
            moves[row["old"]] = row["new"]
    return moves


def main(binary, trail, library=None):
    redundant = rows(trail / "verify-redundant.tsv")
    recovered = rows(trail / "recover-dedup.tsv")
    corrupt = rows(trail / "guard-exceptions.tsv")
    assets = rows(trail / "guard3-exceptions.tsv")
    positives = {r["sha256"] for r in redundant if r["match"] == "yes"}
    exceptions = [r for r in redundant if r["match"] == "NO"]
    assert len(redundant) == 17583 and len(exceptions) == 4
    assert len(recovered) == 4943 and len(corrupt) == 4 and len(assets) == 29
    assert all(r["reason"] == "corrupt-copy" and "ffprobe" in r["evidence"] for r in corrupt)
    assert all(r["reason"] == "game-asset" for r in assets)
    assert {r["redundant_path"] for r in exceptions} == {r["redundant_path"] for r in corrupt}
    targets = {}
    for row in redundant:
        if row["match"] == "yes":
            targets.setdefault(row["canonical_target"], set()).add(row["sha256"])
    for row in exceptions:
        assert row["sha256"] not in targets[row["canonical_target"]], row["canonical_target"]

    # This historical ledger is a SUBSET of the pre-merge library, not a snapshot
    # of every byte in it. The recovered set is now in the live library, so asking
    # the live database today whether it is new would test the opposite proposition.
    with tempfile.TemporaryDirectory(prefix="media-labelled-") as scratch:
        scratch = Path(scratch)
        db = scratch / "media.db"
        subprocess.run([str(binary), "status", "--db", str(db)], check=True, capture_output=True)
        with sqlite3.connect(db) as conn:
            conn.executemany("INSERT INTO media_files(digest,size) VALUES(?,0)", ((d,) for d in positives))
        inputs = [r["sha256"] for r in recovered] + [r["sha256"] for r in exceptions] + sorted(positives)[:10]
        hashes = scratch / "hashes.txt"
        hashes.write_text("\n".join(inputs) + "\n")
        output = subprocess.check_output([str(binary), "classify", "--db", str(db), "--digests-file", str(hashes)], text=True)
        got = [json.loads(line) for line in output.splitlines()]
        assert len(got) == len(inputs)
        for sent, actual in zip(inputs, got):
            # Independent implementation: SQLite membership, not media's own lookup.
            with sqlite3.connect(db) as conn:
                expected = conn.execute("SELECT 1 FROM media_files WHERE digest=?", (sent,)).fetchone() is not None
            assert actual == {"digest": sent, "present": expected}, sent
        assert not any(r["present"] for r in got[:len(recovered)]), "recovered classified as historical duplicate"
        assert not any(r["present"] for r in got[len(recovered):len(recovered)+4]), "corrupt pair classified as duplicate"
        assert all(r["present"] for r in got[-10:]), "known duplicates not classified as present"
    if library is not None:
        moves = relocations(trail)
        checked = 0
        for row in exceptions:
            # Resolve through the recorded renames; `targets` stays keyed by the name
            # the fixture recorded, so the digest assertion is unchanged.
            target = moves.get(row["canonical_target"], row["canonical_target"])
            canonical = library / target
            if not canonical.is_file():
                print(f"live comparison unavailable: {target}")
                continue
            digest = hashlib.sha256()
            with canonical.open("rb") as source:
                for chunk in iter(lambda: source.read(1024 * 1024), b""):
                    digest.update(chunk)
            assert digest.hexdigest() != row["sha256"], target
            assert digest.hexdigest() in targets[row["canonical_target"]], target
            checked += 1
        print(f"live canonical comparison: {checked}/4 accessible (missing paths are NOT passes)")
    print(f"labelled PASS: {len(recovered)} recovered rows new vs historical cohort; 4 corrupt pairs distinct by recorded SHA-256; 10 positive controls; 4 ffprobe and 29 asset decisions accounted for")
    print(f"note: recovered has {len(recovered)-len({r['sha256'] for r in recovered})} repeated digests within its own set; 0% refers to the pre-merge library, not to itself")


if __name__ == "__main__":
    if len(sys.argv) not in (3, 4):
        sys.exit(__doc__)
    main(Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve(), Path(sys.argv[3]) if len(sys.argv) == 4 else None)
