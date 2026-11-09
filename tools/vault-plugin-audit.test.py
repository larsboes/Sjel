"""Synthetic fixtures for the read-only plugin inventory."""

import importlib.machinery
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

LOADER = importlib.machinery.SourceFileLoader("vault_plugin_audit", str(Path(__file__).with_name("vault-plugin-audit")))
SPEC = importlib.util.spec_from_loader(LOADER.name, LOADER)
audit = importlib.util.module_from_spec(SPEC)
LOADER.exec_module(audit)


class PluginAuditTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.plugins = self.root / ".obsidian/plugins"
        self.plugins.mkdir(parents=True)
        (self.root / ".obsidian/community-plugins.json").write_text('["one", "missing"]')
        self.plugin("one")

    def plugin(self, name):
        folder = self.plugins / name
        folder.mkdir()
        (folder / "manifest.json").write_text(json.dumps({"id": name, "version": "1.0"}))

    def test_inventory_never_reads_plugin_settings_or_writes(self):
        (self.plugins / "one/data.json").symlink_to("/not/a/readable/file")
        before = {str(p): p.read_bytes() for p in self.root.rglob("*") if p.is_file()}
        rows = audit.audit(self.root, {"one": "owner/one"})
        self.assertEqual(rows[0]["version"], "1.0")
        self.assertEqual(rows[1]["status"], "enabled-but-missing")
        self.assertEqual(before, {str(p): p.read_bytes() for p in self.root.rglob("*") if p.is_file()})

    def test_online_failure_is_visible_and_repositories_are_cached(self):
        self.plugin("two")
        calls = []
        def query(repo):
            calls.append(repo)
            return {"upstream_status": "unavailable"}
        rows = audit.audit(self.root, {"one": "owner/shared", "two": "owner/shared"}, True, query)
        self.assertEqual(calls, ["owner/shared"])
        self.assertEqual(rows[0]["upstream_status"], "unavailable")

    def test_symlink_and_mismatched_manifests_are_refused(self):
        (self.plugins / "linked").symlink_to(self.plugins / "one", target_is_directory=True)
        self.plugin("wrong")
        (self.plugins / "wrong/manifest.json").write_text('{"id":"other","version":"1"}')
        rows = {row["id"]: row for row in audit.audit(self.root, {})}
        self.assertEqual(rows["linked"]["status"], "refused-path")
        self.assertEqual(rows["wrong"]["status"], "invalid-manifest")

    def test_registry_rejects_untrusted_repository_arguments(self):
        path = self.root / "registry.json"
        path.write_text('[{"id":"one","repo":"--help"}]')
        with self.assertRaises(ValueError):
            audit.registry(path, False)
        path.write_text('[{"id":"one","repo":"owner/repo"}]')
        self.assertEqual(audit.registry(path, False), {"one": "owner/repo"})

    def test_missing_enabled_list_is_not_guessed_as_all_disabled(self):
        (self.root / ".obsidian/community-plugins.json").unlink()
        with self.assertRaises(FileNotFoundError):
            audit.audit(self.root, {})

    def test_no_registry_and_no_network_still_gives_installed_inventory(self):
        self.assertEqual(audit.registry(None, False), {})
        self.assertEqual(audit.audit(self.root, {})[0]["upstream_status"], "unmapped")


if __name__ == "__main__":
    unittest.main()
