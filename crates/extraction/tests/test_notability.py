"""Synthetic Notability archives; no personal files, OCR, or network access."""

import base64
from dataclasses import replace
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import plistlib
import stat
import subprocess
import sys
import tempfile
import unittest
import warnings
import zipfile

MODULE = Path(__file__).resolve().parents[1] / "notability.py"
SPEC = importlib.util.spec_from_file_location("notability", MODULE)
reader = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = reader
SPEC.loader.exec_module(reader)


def plist(value):
    return plistlib.dumps(value, fmt=plistlib.FMT_BINARY)


def keyed(value):
    return plist({"$objects": ["$null", value], "$top": {"root": plistlib.UID(1)}})


def session(objects):
    return plist({"$objects": objects + [{}], "$top": {"root": plistlib.UID(len(objects))}})


def archive(entries=None, root="Example/", compression=zipfile.ZIP_STORED):
    members = {"metadata.plist": keyed({"noteName": "Example"})}
    members.update(entries or {})
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w", compression=compression) as target:
        for name, content in members.items():
            target.writestr(root + name, content)
    return output.getvalue()


class NotabilityTests(unittest.TestCase):
    def test_metadata_typed_text_and_sorted_recognition(self):
        data = archive({
            "metadata.plist": keyed({
                "noteName": "Geometry ü", "noteSubject": "Mathematics",
                "noteTags": {"NS.objects": ["shapes", "angles"]},
                "noteCreationDateKey": {"NS.time": 0},
            }),
            "Session.plist": session([
                "$null", {"stringKey": plistlib.UID(2)}, "Typed text",
                {"stringKey": {"NS.string": "More text"}},
            ]),
            "HandwritingIndex/index.plist": plist({"pages": {
                "10": {"text": "Last"}, "2": {"text": "First"}, "0": {"text": ""},
            }}),
        })
        result = reader.extract_notability(data)
        self.assertEqual(result["title"], "Geometry ü")
        self.assertEqual(result["subject"], "Mathematics")
        self.assertEqual(result["tags"], ["shapes", "angles"])
        self.assertEqual(result["created"], "2001-01-01T00:00:00+00:00")
        self.assertIsNone(result["modified"])
        self.assertEqual(result["typed_text"], "Typed text\n\nMore text")
        self.assertEqual([page["source_page"] for page in result["pages"]], [0, 2, 10])
        self.assertEqual(result["source_sha256"], hashlib.sha256(data).hexdigest())
        self.assertEqual(result["warnings"], ["handwritten_strokes_not_rendered"])

    def test_attachments_retain_paths_bytes_and_distinct_identities(self):
        result = reader.extract_notability(archive({
            "Images/a/same.png": b"first", "Images/b/same.png": b"first",
            "PDFs/file.pdf": b"pdf", "Recordings/voice.M4A": b"audio",
            "Recordings/state.bin": b"not an audio attachment", "thumb6x.png": b"preview",
        }))
        attachments = result["attachments"]
        self.assertEqual(len(attachments), 5)
        self.assertEqual(len({item["id"] for item in attachments}), 5)
        self.assertEqual(attachments[0]["member"], "Images/a/same.png")
        self.assertEqual(base64.b64decode(attachments[0]["data_base64"]), b"first")
        self.assertEqual(attachments[0]["sha256"], attachments[1]["sha256"])
        self.assertNotEqual(attachments[0]["id"], attachments[1]["id"])
        for item in attachments:
            content = base64.b64decode(item["data_base64"])
            self.assertEqual(item["byte_len"], len(content))
            self.assertEqual(item["sha256"], hashlib.sha256(content).hexdigest())
        self.assertEqual(attachments[-1]["role"], "preview")

    def test_repeat_is_deterministic(self):
        data = archive({"Images/image.png": b"image"})
        self.assertEqual(reader.extract_notability(data), reader.extract_notability(data))

    def test_empty_and_missing_index_are_not_recognized_content(self):
        missing = reader.extract_notability(archive())
        self.assertEqual(missing["pages"], [])
        self.assertIn("handwriting_index_missing", missing["warnings"])
        empty = reader.extract_notability(archive({
            "HandwritingIndex/index.plist": plist({"pages": {"1": {"text": ""}}}),
            "thumb6x.png": b"preview",
        }))
        self.assertIn("handwriting_index_empty", empty["warnings"])
        self.assertEqual(len(empty["attachments"]), 1)

    def test_unwrapped_note_and_plain_xml_metadata(self):
        data = archive({"metadata.plist": plistlib.dumps({
            "noteName": "XML note", "noteSubject": "unsortedNotesKey",
        })}, root="")
        result = reader.extract_notability(data)
        self.assertEqual(result["title"], "XML note")
        self.assertIsNone(result["subject"])

    def test_session_without_metadata_does_not_invent_title(self):
        output = io.BytesIO()
        with zipfile.ZipFile(output, "w") as target:
            target.writestr("Session.plist", session([{"stringKey": "Typed"}]))
        result = reader.extract_notability(output.getvalue())
        self.assertIsNone(result["title"])
        self.assertEqual(result["typed_text"], "Typed")

    def test_unsupported_and_ambiguous_roots(self):
        for data in (b"not a zip", archive(root="a/b/"), archive({
                "Other/metadata.plist": keyed({"noteName": "Other"}),
            }, root="")):
            with self.subTest(data=data[:10]), self.assertRaises(reader.ArchiveError):
                reader.extract_notability(data)

    def test_rejects_unsafe_member_paths(self):
        for name in ("../escape", "/absolute", "a/../escape", "a\\escape", "a//b", "C:/file"):
            with self.subTest(name=name), self.assertRaises(reader.ArchiveError):
                reader.extract_notability(archive({name: b"unsafe"}, root=""))

    def test_rejects_duplicate_members(self):
        output = io.BytesIO(archive())
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", UserWarning)
            with zipfile.ZipFile(output, "a") as target:
                target.writestr("Example/metadata.plist", keyed({"noteName": "Other"}))
        with self.assertRaisesRegex(reader.ArchiveError, "duplicate"):
            reader.extract_notability(output.getvalue())

    def test_rejects_symbolic_links(self):
        output = io.BytesIO(archive())
        with zipfile.ZipFile(output, "a") as target:
            entry = zipfile.ZipInfo("Example/Images/link")
            entry.create_system = 3
            entry.external_attr = (stat.S_IFLNK | 0o777) << 16
            target.writestr(entry, "../../outside")
        with self.assertRaisesRegex(reader.ArchiveError, "symbolic"):
            reader.extract_notability(output.getvalue())

    def test_resource_limits(self):
        data = archive({"Images/file": b"x" * 1024}, compression=zipfile.ZIP_DEFLATED)
        for limits in (
            replace(reader.Limits(), input_bytes=len(data) - 1),
            replace(reader.Limits(), expanded_bytes=1023),
            replace(reader.Limits(), member_bytes=1023),
            replace(reader.Limits(), plist_bytes=1),
            replace(reader.Limits(), members=1),
            replace(reader.Limits(), object_visits=1),
            replace(reader.Limits(), depth=0),
            replace(reader.Limits(), decoded_bytes=1),
        ):
            with self.subTest(limits=limits), self.assertRaises(reader.ArchiveError):
                reader.extract_notability(data, limits)

    def test_cycles_and_invalid_references_fail(self):
        for value in (plistlib.UID(1), plistlib.UID(99)):
            data = archive({"metadata.plist": keyed(value)})
            with self.subTest(value=value), self.assertRaises(reader.ArchiveError):
                reader.extract_notability(data)

    def test_malformed_known_members_are_not_silently_ignored(self):
        for member in ("metadata.plist", "Session.plist", "HandwritingIndex/index.plist"):
            for content in (b"not a plist", b"<?xml version='1.0'?><plist><dict>"):
                with self.subTest(member=member, content=content), self.assertRaises(reader.ArchiveError):
                    reader.extract_notability(archive({member: content}))

    def test_invalid_metadata_types_fail(self):
        for metadata in ({"noteTags": "tag"}, {"noteName": []}, {"noteTags": ["$null"]},
                         {"noteCreationDateKey": "yesterday"}, {"noteName": b"\xff"},
                         {"noteCreationDateKey": {"NS.time": float("inf")}}):
            value = {"noteName": "Example", **metadata}
            with self.subTest(metadata=metadata), self.assertRaises(reader.ArchiveError):
                reader.extract_notability(archive({"metadata.plist": keyed(value)}))

    def test_invalid_page_numbers_and_page_shapes_fail(self):
        for pages in ({"a": {}}, {"-1": {}}, {"1": []}, {"1": {}, "01": {}}, {"²": {}}):
            with self.subTest(pages=pages), self.assertRaises(reader.ArchiveError):
                reader.extract_notability(archive({
                    "HandwritingIndex/index.plist": plist({"pages": pages}),
                }))

    def test_repeated_references_cannot_amplify_decoded_text(self):
        data = archive({"Session.plist": session([
            "$null", "x" * 8192,
        ] + [{"stringKey": plistlib.UID(1)}] * 2000)})
        with self.assertRaisesRegex(reader.ArchiveError, "decoded.*byte limit"):
            reader.extract_notability(data)

    def test_decoded_byte_budget_is_shared_across_property_lists(self):
        data = archive({
            "metadata.plist": keyed({"noteName": "x" * 60}),
            "Session.plist": session([{"stringKey": "y" * 60}]),
        })
        with self.assertRaisesRegex(reader.ArchiveError, "decoded.*byte limit"):
            reader.extract_notability(data, replace(reader.Limits(), decoded_bytes=100))

    def test_encrypted_member_is_refused_before_reading(self):
        data = bytearray(archive())
        data[6] |= 1
        central_header = data.index(b"PK\x01\x02")
        data[central_header + 8] |= 1
        with self.assertRaisesRegex(reader.ArchiveError, "encrypted"):
            reader.extract_notability(data)

    def test_corrupt_attachment_crc_fails_the_extraction(self):
        data = archive({"Images/image.png": b"unique attachment bytes"})
        data = data.replace(b"unique attachment bytes", b"broken attachment bytes", 1)
        with self.assertRaises(reader.ArchiveError):
            reader.extract_notability(data)

    def test_missing_title_and_session_are_not_a_note(self):
        with self.assertRaisesRegex(reader.ArchiveError, "title or session"):
            reader.extract_notability(archive({"metadata.plist": keyed({})}))

    def test_invalid_session_shapes_fail(self):
        for session in ({"text": "not a session"}, {"$objects": "not an array"},
                        {"$objects": [{"stringKey": plistlib.UID(999)}]},
                        {"$objects": ["$null", {"stringKey": "Ghost text"}],
                         "$top": {"root": plistlib.UID(999)}},
                        {"$objects": ["$null"], "$top": {"root": plistlib.UID(0)}}):
            with self.subTest(session=session), self.assertRaises(reader.ArchiveError):
                reader.extract_notability(archive({"Session.plist": plist(session)}))

    def test_invalid_index_shapes_fail(self):
        for index in ({}, {"pages": []}, {"pages": {"1": {"text": []}}}):
            with self.subTest(index=index), self.assertRaises(reader.ArchiveError):
                reader.extract_notability(archive({"HandwritingIndex/index.plist": plist(index)}))

    def test_binary_dictionary_with_non_string_key_fails_cleanly(self):
        metadata = keyed({"noteName": "Example", "zz": "ignored"})
        metadata = metadata.replace(b"\x52zz", b"\x10\x07\x00", 1)
        data = archive({"metadata.plist": metadata})
        with self.assertRaisesRegex(reader.ArchiveError, "dictionary keys"):
            reader.extract_notability(data)
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "malformed.note"
            source.write_bytes(data)
            result = subprocess.run([sys.executable, str(MODULE), str(source)],
                                    capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stdout, "")
            self.assertNotIn("Traceback", result.stderr)
            self.assertIn("dictionary keys", result.stderr)

    def test_cli_reads_only_input_and_emits_structured_json(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "example.note"
            source.write_bytes(archive({"Images/example.png": b"image"}))
            before = source.read_bytes()
            result = subprocess.run([sys.executable, str(MODULE), str(source)],
                                    capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)["title"], "Example")
            self.assertEqual(result.stderr, "")
            self.assertEqual(source.read_bytes(), before)
            self.assertEqual(list(Path(temporary).iterdir()), [source])

    def test_cli_failure_emits_no_partial_json_or_traceback(self):
        with tempfile.TemporaryDirectory() as temporary:
            for name in ("broken.note", "unsupported.goodnotes"):
                source = Path(temporary) / name
                source.write_bytes(b"broken")
                result = subprocess.run([sys.executable, str(MODULE), str(source)],
                                        capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, "")
                self.assertTrue(result.stderr.startswith("note-extract: "))
                self.assertNotIn("Traceback", result.stderr)


if __name__ == "__main__":
    unittest.main()
