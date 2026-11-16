"""Regression tests for locked component discovery; no Cargo or network required."""

import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path
from subprocess import CompletedProcess
from unittest.mock import patch

from find_component import emit, matches, package_source, test_candidates


class ComponentLookupTests(unittest.TestCase):
    def test_public_declarations_and_consumer_locations(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "src" / "tables.rs"
            source.parent.mkdir()
            source.write_text("pub struct DataTable;\npub(crate) struct PrivateTable;\nlet x = DataTable::new();\n")
            self.assertEqual(len(matches(root, "data_table", True)), 1)
            self.assertEqual(len(matches(root, "DataTable")), 2)
            self.assertEqual(matches(root, "PrivateTable", True), [])

    def test_test_candidates_exclude_ordinary_helpers(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "src" / "terminal.rs"
            source.parent.mkdir()
            source.write_text("fn helper() {}\n#[gpui::test]\nfn chunks() {}\n#[test]\n#[ignore]\nfn resume() {}\n")
            found = test_candidates(root, "terminal")
            self.assertEqual(found, [f"{source}:3: fn chunks", f"{source}:6: fn resume"])

    @patch("find_component.subprocess.run")
    def test_metadata_is_locked_offline_and_scoped_to_the_host(self, run):
        run.side_effect = [
            CompletedProcess([], 0, "host: aarch64-apple-darwin\n", ""),
            CompletedProcess([], 0, json.dumps({"packages": [{
                "name": "ely-gpui-component", "version": "0.1.1",
                "source": "git+https://example.org/ely#abcdef",
                "manifest_path": "/cache/ely/Cargo.toml",
            }]}), ""),
        ]
        source, identity = package_source(Path("/app/Cargo.toml"), "ely-gpui-component")
        self.assertEqual(source, Path("/cache/ely"))
        self.assertIn("#abcdef", identity)
        command = run.call_args.args[0]
        self.assertIn("--locked", command)
        self.assertIn("--offline", command)
        self.assertEqual(command[command.index("--filter-platform") + 1], "aarch64-apple-darwin")

    @patch("find_component.subprocess.run")
    def test_missing_or_patched_sources_are_not_reported_as_pinned(self, run):
        for packages in [[], [{"name": "ely-gpui-component", "source": None}]]:
            run.side_effect = [
                CompletedProcess([], 0, "host: test-target\n", ""),
                CompletedProcess([], 0, json.dumps({"packages": packages}), ""),
            ]
            with self.assertRaises(ValueError):
                package_source(Path("/app/Cargo.toml"), "ely-gpui-component")

    def test_truncation_is_explicit(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            emit("Source", ["one", "two", "three"], 1)
        self.assertIn("3 matching locations", out.getvalue())
        self.assertIn("2 locations omitted", out.getvalue())


if __name__ == "__main__":
    unittest.main()
