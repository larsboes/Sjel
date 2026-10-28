import contextlib
import io
from pathlib import Path
import runpy
import tempfile
import unittest


tool = runpy.run_path(str(Path(__file__).with_name("vault-archive-intake")))


class VaultArchiveIntakeTest(unittest.TestCase):
    def test_copies_and_verifies_only_allowed_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            (source / "Archive").mkdir(parents=True)
            (source / "Knowledge").mkdir()
            (source / "secrets").mkdir()
            (source / "Archive" / "note.md").write_text("saved")
            (source / "Knowledge" / "note.md").write_text("knowledge")
            (source / "secrets" / "token").write_text("excluded")
            (source / "Archive" / "ignored.env").write_text("excluded")
            (source / "Archive" / "link").symlink_to(source / "Archive" / "note.md")
            destination = root / "intake"
            with contextlib.redirect_stdout(io.StringIO()) as output:
                tool["run"](source, destination)
            self.assertEqual((destination / "Archive" / "note.md").read_text(), "saved")
            self.assertFalse((destination / "Knowledge").exists())
            self.assertFalse((destination / "secrets").exists())
            self.assertFalse((destination / "Archive" / "ignored.env").exists())
            self.assertFalse((destination / "Archive" / "link").exists())
            self.assertIn("verified files: 1", output.getvalue())
            self.assertIn("symlinks: 1", output.getvalue())
            with contextlib.redirect_stdout(io.StringIO()) as verified:
                tool["verify"](source, destination)
            self.assertIn("missing: 0; extra: 0; changed: 0", verified.getvalue())
            (destination / "Archive" / "note.md").write_text("changed")
            with contextlib.redirect_stdout(io.StringIO()) as difference:
                with self.assertRaises(ValueError):
                    tool["verify"](source, destination)
            self.assertIn("changed: 1", difference.getvalue())
            with self.assertRaises(ValueError):
                tool["run"](source, destination)

    def test_rejects_nested_destination_and_unicode_collisions(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "source"
            source.mkdir()
            with self.assertRaises(ValueError):
                tool["run"](source, source / "archive")
            (source / "é.txt").write_text("one")
            other = source / "e\N{COMBINING ACUTE ACCENT}.txt"
            if not other.exists():
                other.write_text("two")
                with self.assertRaises(ValueError):
                    tool["run"](source, Path(tmp) / "out")


if __name__ == "__main__":
    unittest.main()
