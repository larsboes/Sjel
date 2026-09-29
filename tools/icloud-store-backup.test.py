#!/usr/bin/env python3
"""Retention must never precede the new upload and the preceding restore."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

TOOLS = Path(__file__).resolve().parent
STAMPS = ["20260101T000000Z", "20260102T000000Z", "20260103T000000Z", "20260104T000000Z"]


class StoreCloudTest(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix="sjel-cloud-test-")
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.destination = self.root / "cloud" / "store"
        self.destination.mkdir(parents=True)
        self.overlay = self.root / "overlay"
        self.overlay.mkdir()
        self.tools = self.root / "tools"
        self.tools.mkdir()
        (self.tools / "icloud-item.swift").touch()
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        for name in ("swift", "restore.sh"):
            script = bin_dir / name
            script.write_text(
                "#!/bin/sh\n"
                f"printf '%s %s\\n' '{name}' \"$*\" >> \"$MOCK_LOG\"\n"
                f"case \"{name} $*\" in *\"$MOCK_FAIL_MODE\"*) if [ -n \"$MOCK_FAIL_MODE\" ]; then exit 1; fi;; esac\n"
                "exit 0\n"
            )
            script.chmod(0o755)
        (self.tools / "restore.sh").symlink_to(bin_dir / "restore.sh")
        self.log = self.root / "operations.log"
        self.env = os.environ.copy()
        self.env.update({"PATH": str(bin_dir) + os.pathsep + os.environ["PATH"], "MOCK_LOG": str(self.log), "MOCK_FAIL_MODE": ""})

    def archive(self, index):
        name = f"store-{STAMPS[index]}.tar.gz"
        path = self.destination / name
        path.write_bytes(f"store backup {index}".encode())
        return path

    def run_gate(self, archive, keep=2, no_prune="0", fail_mode=""):
        env = dict(self.env, MOCK_FAIL_MODE=fail_mode)
        return subprocess.run(
            ["python3", str(TOOLS / "icloud-store-backup.py"), str(self.destination),
             archive.name, str(keep), no_prune, "backup-target", str(self.overlay),
             str(self.tools), hashlib.sha256(archive.read_bytes()).hexdigest(),
             str(archive.stat().st_size), "1"],
            env=env, text=True, capture_output=True,
        )

    def history(self, archive):
        return self.overlay / "backup" / "receipts" / "history" / "store" / f"{archive.name}.json"

    def test_old_restores_before_prune_even_after_offloading(self):
        first = self.archive(0)
        self.assertEqual(self.run_gate(first).stdout.strip(), "true")
        second = self.archive(1)
        self.assertEqual(self.run_gate(second).returncode, 0)
        third = self.archive(2)
        result = self.run_gate(third)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "true")
        self.assertFalse(first.exists())
        self.assertTrue(second.exists())
        self.assertTrue(third.exists())
        operations = self.log.read_text().splitlines()
        self.assertLess(next(i for i, line in enumerate(operations) if "wait-download" in line),
                        next(i for i, line in enumerate(operations) if "restore.sh" in line))
        self.assertLess(next(i for i, line in enumerate(operations) if "restore.sh" in line),
                        next(i for i, line in enumerate(operations) if "check-upload" in line))
        self.assertEqual(json.loads(self.history(third).read_text())["tarball"], third.name)

    def test_upload_failure_leaves_old_and_records_produced_digest(self):
        first = self.archive(0)
        self.run_gate(first)
        second = self.archive(1)
        result = self.run_gate(second, keep=2, fail_mode=f"wait-upload {second}")
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(self.history(second).exists())
        self.assertTrue(first.exists())
        self.assertNotIn("restore.sh", self.log.read_text())

    def test_missing_prior_receipt_refuses_deletion(self):
        first = self.archive(0)
        second = self.archive(1)
        third = self.archive(2)
        self.assertNotEqual(self.run_gate(third).returncode, 0)
        self.assertTrue(first.exists())
        self.assertTrue(second.exists())

    def test_bad_digest_or_failed_restore_refuses_deletion(self):
        first = self.archive(0)
        self.run_gate(first)
        second = self.archive(1)
        self.run_gate(second)
        third = self.archive(2)
        second.write_bytes(b"altered backup")
        result = self.run_gate(third)
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(first.exists())
        second.write_bytes(b"store backup 1")
        fourth = self.archive(3)
        result = self.run_gate(fourth, fail_mode="restore.sh")
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(first.exists())

    def test_no_prune_never_reads_older_archive(self):
        first = self.archive(0)
        second = self.archive(1)
        result = self.run_gate(second, keep=2, no_prune="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "false")
        self.assertTrue(first.exists())
        self.assertNotIn("wait-download", self.log.read_text())

    def test_renamed_target_does_not_disable_retention(self):
        # An operator renaming the systems.toml entry (or repointing the destination through a
        # symlink) changes the target id, not the archive. Retention must still be able to run
        # once the archive's own bytes are verified; otherwise nothing is ever pruned again and
        # the failure is invisible.
        first = self.archive(0)
        self.run_gate(first)
        second = self.archive(1)
        self.run_gate(second)
        third = self.archive(2)
        result = subprocess.run(
            ["python3", str(TOOLS / "icloud-store-backup.py"), str(self.destination),
             third.name, "2", "0", "renamed-target", str(self.overlay), str(self.tools),
             hashlib.sha256(third.read_bytes()).hexdigest(), str(third.stat().st_size), "1"],
            env=self.env, text=True, capture_output=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(first.exists())


if __name__ == "__main__":
    unittest.main()
