"""Read Notability archives without OCR, network access, or filesystem writes."""

import argparse
import base64
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
import hashlib
import io
import json
from pathlib import Path
import plistlib
import stat
import sys
import zipfile
import zlib
from xml.parsers.expat import ExpatError


class ArchiveError(ValueError):
    """The archive is unsupported, malformed, or exceeds the extraction limits."""


@dataclass(frozen=True)
class Limits:
    """Resource bounds applied before reading archive members."""

    input_bytes: int = 32 * 1024 * 1024
    expanded_bytes: int = 64 * 1024 * 1024
    member_bytes: int = 16 * 1024 * 1024
    plist_bytes: int = 4 * 1024 * 1024
    members: int = 2048
    object_visits: int = 100_000
    depth: int = 32
    decoded_bytes: int = 8 * 1024 * 1024


class DecodedBudget:
    def __init__(self, remaining):
        self.remaining = remaining

    def consume(self, value):
        size = len(value.encode("utf-8")) if isinstance(value, str) else len(value)
        self.remaining -= size
        if self.remaining < 0:
            raise ArchiveError("decoded property-list content exceeds the byte limit")


class PlistReader:
    def __init__(self, data, limits, budget):
        if len(data) > limits.plist_bytes:
            raise ArchiveError("property list exceeds the byte limit")
        self.value = plistlib.loads(data)
        if not isinstance(self.value, dict):
            raise ArchiveError("property list must contain a dictionary")
        self.objects = self.value.get("$objects", [])
        if not isinstance(self.objects, list):
            raise ArchiveError("archive objects must be an array")
        self.remaining = limits.object_visits
        self.depth = limits.depth
        self.budget = budget

    def resolve(self, value, depth=0):
        self.remaining -= 1
        if depth > self.depth or self.remaining < 0:
            raise ArchiveError("property-list reference limit exceeded")
        if isinstance(value, plistlib.UID):
            if value.data >= len(self.objects):
                raise ArchiveError("property-list reference is out of range")
            return self.resolve(self.objects[value.data], depth + 1)
        if isinstance(value, list):
            return [self.resolve(item, depth + 1) for item in value]
        if isinstance(value, dict):
            if any(not isinstance(key, str) for key in value):
                raise ArchiveError("property-list dictionary keys must be strings")
            if "NS.time" in value:
                seconds = value["NS.time"]
                if isinstance(seconds, bool) or not isinstance(seconds, (float, int)):
                    raise ArchiveError("Apple date must contain seconds")
                return datetime(2001, 1, 1, tzinfo=timezone.utc) + timedelta(seconds=seconds)
            for key in ("NS.objects", "NS.string", "NS.data"):
                if key in value:
                    return self.resolve(value[key], depth + 1)
            return {
                key: self.resolve(item, depth + 1)
                for key, item in value.items() if not key.startswith("$")
            }
        if isinstance(value, (str, bytes)):
            self.budget.consume(value)
        return None if value == "$null" else value

    def root(self):
        if "$objects" not in self.value:
            return self.resolve(self.value)
        top = self.value.get("$top")
        if not isinstance(top, dict) or "root" not in top:
            raise ArchiveError("keyed archive has no root reference")
        return self.resolve(top["root"])


def text_value(value):
    if value is None:
        return None
    if isinstance(value, bytes):
        return value.decode("utf-8").rstrip("\x00")
    if not isinstance(value, str):
        raise ArchiveError("text field must contain a string")
    return value


def date_value(value):
    if value is None:
        return None
    if not isinstance(value, datetime):
        raise ArchiveError("date field must contain a date")
    return value.replace(tzinfo=timezone.utc).isoformat()


def archive_members(archive, limits):
    entries = archive.infolist()
    if len(entries) > limits.members:
        raise ArchiveError("archive exceeds the member-count limit")
    members = {}
    expanded = 0
    for entry in entries:
        name = entry.orig_filename
        parts = name.rstrip("/").split("/")
        if (not name or "\\" in name or "\x00" in name
                or any(part in ("", ".", "..") or ":" in part for part in parts)):
            raise ArchiveError("archive contains an unsafe member path")
        if name in members:
            raise ArchiveError("archive contains duplicate member paths")
        if entry.flag_bits & 1 or stat.S_ISLNK(entry.external_attr >> 16):
            raise ArchiveError("encrypted members and symbolic links are unsupported")
        expanded += entry.file_size
        if entry.file_size > limits.member_bytes or expanded > limits.expanded_bytes:
            raise ArchiveError("archive exceeds the expanded-byte limit")
        members[name] = entry
    return members


def note_root(members):
    roots = set()
    for name in members:
        for marker in ("metadata.plist", "Session.plist", "HandwritingIndex/index.plist"):
            if name == marker:
                roots.add("")
            elif name.endswith("/" + marker):
                prefix = name[:-len(marker)]
                if prefix.count("/") == 1:
                    roots.add(prefix)
    if len(roots) != 1:
        raise ArchiveError("expected one Notability note root")
    return roots.pop()


def extract_notability(data, limits=Limits()):
    """Return source identity, metadata, stored text, and base64 attachments.

    Raises ArchiveError for unsupported or malformed archives and resource limits.
    Handwritten strokes are not rendered; absence of recognized text stays explicit.
    """
    if len(data) > limits.input_bytes:
        raise ArchiveError("archive exceeds the input-byte limit")
    try:
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            members = archive_members(archive, limits)
            root = note_root(members)
            budget = DecodedBudget(limits.decoded_bytes)

            def read(name):
                entry = members[root + name]
                with archive.open(entry) as source:
                    content = source.read(limits.member_bytes + 1)
                if len(content) != entry.file_size or len(content) > limits.member_bytes:
                    raise ArchiveError("archive member size does not match its header")
                return content

            metadata = {}
            if root + "metadata.plist" in members:
                metadata = PlistReader(read("metadata.plist"), limits, budget).root()
                if not isinstance(metadata, dict):
                    raise ArchiveError("note metadata must be a dictionary")
            title = text_value(metadata.get("noteName"))
            if not title and root + "Session.plist" not in members:
                raise ArchiveError("archive has no note title or session")
            subject = text_value(metadata.get("noteSubject"))
            tags = metadata.get("noteTags", [])
            if not isinstance(tags, list):
                raise ArchiveError("note tags must be an array")
            tags = [text_value(tag) for tag in tags]
            if any(tag is None for tag in tags):
                raise ArchiveError("note tags must contain strings")

            typed = []
            if root + "Session.plist" in members:
                session = PlistReader(read("Session.plist"), limits, budget)
                if "$objects" not in session.value:
                    raise ArchiveError("note session must be a keyed archive")
                if not isinstance(session.root(), dict):
                    raise ArchiveError("note session root must be a dictionary")
                for item in session.objects:
                    if isinstance(item, dict) and "stringKey" in item:
                        text = text_value(session.resolve(item["stringKey"]))
                        if text and text.strip():
                            typed.append(text)

            pages = []
            has_index = root + "HandwritingIndex/index.plist" in members
            if has_index:
                index = PlistReader(read("HandwritingIndex/index.plist"), limits, budget).root()
                if not isinstance(index, dict) or not isinstance(index.get("pages"), dict):
                    raise ArchiveError("handwriting index must contain a pages dictionary")
                numbers = set()
                for key, page in index["pages"].items():
                    if not key.isascii() or not key.isdecimal() or not isinstance(page, dict):
                        raise ArchiveError("invalid handwriting page")
                    number = int(key)
                    if number in numbers:
                        raise ArchiveError("duplicate handwriting page number")
                    numbers.add(number)
                    pages.append({"source_page": number, "text": text_value(page.get("text")) or ""})
                pages.sort(key=lambda page: page["source_page"])

            attachments = []
            for name in sorted(members):
                if not name.startswith(root) or members[name].is_dir():
                    continue
                relative = name[len(root):]
                keep = relative.startswith(("Images/", "PDFs/")) or (
                    relative.startswith("Recordings/")
                    and Path(relative).suffix.lower() in (".m4a", ".mp3", ".wav")
                ) or relative == "thumb6x.png"
                if keep:
                    content = read(relative)
                    attachments.append({
                        "id": hashlib.sha256(relative.encode("utf-8") + b"\x00" + content).hexdigest(),
                        "member": relative,
                        "sha256": hashlib.sha256(content).hexdigest(),
                        "byte_len": len(content),
                        "role": "preview" if relative == "thumb6x.png" else "attachment",
                        "data_base64": base64.b64encode(content).decode("ascii"),
                    })
            warnings = ["handwritten_strokes_not_rendered"]
            if not has_index:
                warnings.append("handwriting_index_missing")
            elif not any(page["text"].strip() for page in pages):
                warnings.append("handwriting_index_empty")
            return {
                "schema_version": 1,
                "producer": "notability-archive",
                "source_sha256": hashlib.sha256(data).hexdigest(),
                "title": title,
                "subject": None if subject == "unsortedNotesKey" else subject,
                "tags": tags,
                "created": date_value(metadata.get("noteCreationDateKey")),
                "modified": date_value(metadata.get("noteModifiedDateKey")),
                "typed_text": "\n\n".join(typed),
                "pages": pages,
                "attachments": attachments,
                "warnings": warnings,
            }
    except (ValueError, TypeError, KeyError, IndexError, OverflowError, RecursionError,
            zipfile.BadZipFile, zlib.error, ExpatError, NotImplementedError,
            RuntimeError, OSError) as error:
        if isinstance(error, ArchiveError):
            raise
        raise ArchiveError("invalid or unsupported Notability archive") from error


def main(argv=None):
    """Read one .note file and emit JSON; failures emit no extraction to stdout."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="local Notability .note archive")
    args = parser.parse_args(argv)
    limits = Limits()
    try:
        if args.source.suffix.lower() != ".note":
            raise ArchiveError("only Notability .note archives are supported")
        with args.source.open("rb") as source:
            data = source.read(limits.input_bytes + 1)
        result = extract_notability(data, limits)
        payload = json.dumps(result, ensure_ascii=True, allow_nan=False)
    except (ArchiveError, OSError) as error:
        print(f"note-extract: {error}", file=sys.stderr)
        return 1
    print(payload)
    return 0


if __name__ == "__main__":
    sys.exit(main())
