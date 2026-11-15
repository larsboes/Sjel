"""Exercise release lookup and direct-Zed compatibility without network access."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("show_revision.py")


class RevisionLookupTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.manifest = self.root / "Cargo.toml"
        self.cargo_home = self.root / "cargo"

    def run_lookup(self):
        return subprocess.run(
            [sys.executable, str(SCRIPT), str(self.manifest)],
            env={**os.environ, "CARGO_HOME": str(self.cargo_home)},
            capture_output=True,
            text=True,
            check=False,
        )

    def write_kit(self, source="registry+https://github.com/rust-lang/crates.io-index"):
        self.manifest.write_text('[workspace.dependencies]\ngpui-kit = "0.9"\n')
        (self.root / "Cargo.lock").write_text(
            '[[package]]\nname = "gpui-kit"\nversion = "0.9.1"\n'
            f'source = "{source}"\n'
        )

    def test_locked_release_uses_cached_publication_not_manifest_requirement(self):
        self.write_kit()
        vcs_path = self.cargo_home / "registry/src/example/gpui-kit-0.9.1/.cargo_vcs_info.json"
        vcs_path.parent.mkdir(parents=True)
        revision = "a" * 40
        vcs_path.write_text(json.dumps({"git": {"sha1": revision}, "path_in_vcs": "crates/kit"}))
        result = self.run_lookup()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("locked version: 0.9.1", result.stdout)
        self.assertIn(f"/blob/{revision}/skills/gpui-kit/references/gpui/test.md", result.stdout)
        self.assertNotIn("/blob/main/", result.stdout)

    def test_missing_cache_keeps_versioned_fallback(self):
        self.write_kit()
        result = self.run_lookup()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("https://docs.rs/crate/gpui-kit/0.9.1/source/", result.stdout)
        self.assertIn("publication revision unavailable", result.stdout)
        self.assertNotIn("publication revision:", result.stdout)

    def test_missing_lockfile_is_not_a_resolved_publication(self):
        self.manifest.write_text('[dependencies]\ngpui-kit = "0.9"\n')
        result = self.run_lookup()
        self.assertEqual(result.returncode, 2)
        self.assertIn("Cannot resolve GPUI Kit publication", result.stderr)

    def test_patched_source_cannot_borrow_registry_metadata(self):
        self.write_kit("git+https://example.org/kit#abcdef")
        result = self.run_lookup()
        self.assertEqual(result.returncode, 2)
        self.assertIn("patched or uses another source", result.stderr)
        self.assertNotIn("publication revision:", result.stdout)

    def test_direct_zed_pins_keep_revision_specific_links(self):
        self.manifest.write_text(
            '[dependencies]\ngpui = { git = "https://github.com/zed-industries/zed", rev = "abc123" }\n'
        )
        result = self.run_lookup()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("/tree/abc123/crates/gpui", result.stdout)

    def test_inconsistent_zed_pins_fail(self):
        self.manifest.write_text(
            '[dependencies]\ngpui = { git = "https://github.com/zed-industries/zed", rev = "one" }\n'
            'gpui_platform = { git = "https://github.com/zed-industries/zed", rev = "two" }\n'
        )
        result = self.run_lookup()
        self.assertEqual(result.returncode, 1)
        self.assertIn("different Zed revisions", result.stderr)


if __name__ == "__main__":
    unittest.main()
