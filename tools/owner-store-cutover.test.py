#!/usr/bin/env python3
"""Test the cutover failure boundary without starting or stopping a deployed service."""

import os
from pathlib import Path
import runpy
import shutil
import socket
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch
from urllib.request import ProxyHandler, build_opener

SCRIPT = Path(__file__).with_name("owner-store-cutover.py")


class HealthCutoverTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.apply = runpy.run_path(str(SCRIPT))["apply"]
        self.command = self.apply.__globals__
        self.legacy = self.command["LEGACY_BUILD"]
        self.native = self.command["OWNER_BUILD"]
        self.manifest = self.root / "service.toml"
        self.binary = self.root / "health"
        self.marker = self.root / "owner/health.db"
        self.manifest.write_bytes(self.legacy)
        self.binary.write_bytes(b"shared")
        self.lifecycle = []
        self.fail_build = False
        self.fail_shared_restore = False
        self.fail_probe = False
        self.fail_install = False
        self.fail_post_activation = False

    def simulate(self, program, *args, **kwargs):
        match Path(program).name, args:
            case "cargo", ("build", *flags, "--target-dir", target):
                self.assertFalse(self.lifecycle)
                self.assertEqual(flags, ["--locked", "--release", "--no-default-features", "--features",
                                         "turso-wal", "--bin", "health", "--package", "sjel-health"])
                self.assertFalse(target.is_relative_to(SCRIPT.resolve().parent.parent))
                self.assertEqual(kwargs["environ"]["CARGO_BUILD_BUILD_DIR"], str(target))
                if self.fail_build:
                    raise RuntimeError("compiler failed")
                (target / "release").mkdir()
                (target / "release/health").write_bytes(b"native")
            case "service-runner.sh", ("status", "health"):
                return "health process stopped, held no answer"
            case "service-runner.sh", (event, "health"):
                self.lifecycle.append(event)
            case _, (helper, "health", source, destination) if Path(helper).name == "owner-store-split.py":
                destination.write_bytes(b"health rows")
            case "health", ("check-owner",):
                if self.fail_probe:
                    raise RuntimeError("candidate failed")
            case _:
                self.fail(f"unexpected command: {program}, {args}")

    def restore(self, capability, overlay, machine, scratch, _environ):
        self.assertEqual(capability, "store")
        self.assertTrue(scratch.is_relative_to(overlay.resolve() / "data/.restore-scratch"))
        self.assertEqual(self.lifecycle, ["stop"])
        if self.fail_shared_restore:
            raise RuntimeError("shared restore failed")

    def put(self, path, content, mode):
        if path == self.binary and self.fail_install:
            raise OSError("disk write failed")
        self.original_put(path, content, mode)

    def verify(self, *_):
        self.assertEqual(self.lifecycle, ["stop", "resume"])
        self.assertEqual(self.manifest.read_bytes(), self.native)
        self.assertEqual(self.binary.read_bytes(), b"native")
        self.assertEqual(self.marker.read_bytes(), b"health rows")
        if self.fail_post_activation:
            raise RuntimeError("owner backup failed")

    def execute(self):
        self.original_put = self.command["put_file"]
        overrides = {
            "HEALTH_MANIFEST": self.manifest, "run": self.simulate, "put_file": self.put,
            "prove_legacy_source": lambda *_: None, "backup_restore": self.restore,
            "compare_health": lambda *_, **__: None, "verify_owner": self.verify,
        }
        with patch.dict(self.command, overrides):
            self.apply(self.root, self.root / "machine.toml", self.root / "shared.db",
                       self.marker, self.binary, "http://127.0.0.1:1", self.legacy)

    def assert_legacy_resumed(self):
        self.assertEqual(self.lifecycle, ["stop", "resume"])
        self.assertEqual(self.manifest.read_bytes(), self.legacy)
        self.assertEqual(self.binary.read_bytes(), b"shared")
        self.assertFalse(self.marker.exists())

    def test_candidate_ready_before_service_resumes(self):
        self.execute()
        self.verify()

    def test_failed_build_leaves_service_running(self):
        self.fail_build = True
        with self.assertRaisesRegex(RuntimeError, "compiler failed"):
            self.execute()
        self.assertEqual(self.lifecycle, [])
        self.assertEqual(self.manifest.read_bytes(), self.legacy)
        self.assertEqual(self.binary.read_bytes(), b"shared")
        self.assertFalse(self.marker.exists())

    def test_failed_shared_restore_resumes_legacy_and_cleans_up(self):
        self.fail_shared_restore = True
        with self.assertRaisesRegex(RuntimeError, "shared restore failed"):
            self.execute()
        self.assert_legacy_resumed()
        self.assertEqual(list((self.root / "data/.restore-scratch").iterdir()), [])

    def test_failed_candidate_read_resumes_legacy(self):
        self.fail_probe = True
        with self.assertRaisesRegex(RuntimeError, "candidate failed"):
            self.execute()
        self.assert_legacy_resumed()

    def test_failed_binary_install_restores_legacy_manifest(self):
        self.fail_install = True
        with self.assertRaisesRegex(OSError, "disk write failed"):
            self.execute()
        self.assert_legacy_resumed()

    def test_failed_backup_after_activation_does_not_fall_back(self):
        self.fail_post_activation = True
        with self.assertRaisesRegex(RuntimeError, "owner backup failed"):
            self.execute()
        self.assertEqual(self.lifecycle, ["stop", "resume"])
        self.assertEqual(self.manifest.read_bytes(), self.native)
        self.assertEqual(self.binary.read_bytes(), b"native")
        self.assertTrue(self.marker.exists())


class LegacySourceTest(unittest.TestCase):
    def test_launch_settings_take_priority_and_preserve_spaces(self):
        cutover = runpy.run_path(str(SCRIPT))
        settings = cutover["source_settings"]([
            b"SJEL_PERSONAL_ROOT=/fallback overlay", b"SJEL_DB_PATH=~/custom store.db",
            b"HOME=/operator", b"UNRELATED=ignored SJEL_DB_PATH=/do-not-read.db",
        ])
        self.assertNotIn("UNRELATED", settings)
        self.assertEqual(cutover["launched_database"](settings), Path("/operator/custom store.db"))
        settings["SJEL_DB_PATH"] = " "
        self.assertEqual(cutover["launched_database"](settings), Path("/fallback overlay/data/sjel/sjel.db"))
        settings["SJEL_DB_PATH"] = "relative.db"
        with self.assertRaises(cutover["Refused"]):
            cutover["launched_database"](settings)

    @unittest.skipUnless(os.environ.get("SJEL_LEGACY_HEALTH_BIN"), "requires an isolated libSQL Health binary")
    def test_idle_legacy_source_is_proved_without_an_open_descriptor(self):
        cutover = runpy.run_path(str(SCRIPT))
        artifact = Path(os.environ["SJEL_LEGACY_HEALTH_BIN"])
        with tempfile.TemporaryDirectory() as work, socket.socket() as reserve:
            executable = Path(work) / "health"
            shutil.copy2(artifact, executable)
            overlay = Path(work) / "overlay with spaces"
            expected = overlay / "data/sjel/sjel.db"
            reserve.bind(("127.0.0.1", 0))
            port = reserve.getsockname()[1]
            reserve.close()
            base = f"http://127.0.0.1:{port}"
            settings = {"SJEL_PERSONAL_ROOT": str(overlay), "SJEL_PORT": str(port)}
            client = build_opener(ProxyHandler({}))
            with subprocess.Popen([executable, "serve"], env=settings, stdout=subprocess.DEVNULL) as process:
                try:
                    for _ in range(40):
                        try:
                            with client.open(base + "/ready", timeout=0.5) as response:
                                self.assertEqual(response.status, 200)
                            break
                        except OSError:
                            time.sleep(0.05)
                    else:
                        self.fail("fixture Health never became ready")
                    replacement = executable.with_suffix(".new")
                    shutil.copy2(artifact, replacement)
                    replacement.replace(executable)
                    cutover["prove_legacy_source"](base, expected, executable)
                    wrong = expected.with_name("other.db")
                    wrong.touch()
                    with self.assertRaisesRegex(cutover["Refused"], "different shared database"):
                        cutover["prove_legacy_source"](base, wrong, executable)
                finally:
                    process.terminate()


if __name__ == "__main__":
    unittest.main()
