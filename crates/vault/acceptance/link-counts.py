#!/usr/bin/env python3
"""A second implementation of `vault links`, whose job is to disagree with the first.

    python3 capabilities/vault/acceptance/link-counts.py <vault-root>
    vault links --root <vault-root>

Run both against the same vault at the same moment and compare. This replaced a
table of seven counts measured in August 2026, retired on 2026-09-08 because it
disagreed with the vault on every line and one of its rows described a
frontmatter key the vault no longer has. A saved count over a vault a human
edits every day is a test that cannot fail: every mismatch reads as "the vault
moved again". Two implementations that must agree today do not have that
problem.

**It shares no code with the crate on purpose, and it must keep sharing none.**
Its walk, its wikilink pattern and its resolution ladder are written separately
from `capabilities/vault/src/graph.rs`. Making them agree by making them the
same code would delete the only thing this file is for. When the two disagree,
find out which is wrong and write the reason down — that is where the crate's
bracket rule and both of `inbound`'s defects came from.

Read-only. It opens files under the root and writes nothing anywhere.
"""
import os
import posixpath
import re
import sys

# Deliberately not the crate's scanner. The crate walks bytes and takes the span
# to the first `]]`; this takes a regex that cannot cross a `]`. The 10-link gap
# between them is `[[[...]]]`, and it is documented rather than tuned away.
LINK = re.compile(r"\[\[([^\]\n]+)\]\]")


def measure(root):
    files = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if not d.startswith(".")]
        for name in filenames:
            if name.startswith("."):
                continue
            rel = os.path.relpath(os.path.join(dirpath, name), root)
            files.append(rel.replace(os.sep, "/"))

    notes = [f for f in files if f.endswith(".md")]
    every_id = {f.lower() for f in files}
    note_ids = {f[:-3].lower() for f in notes}

    by_name = {}
    for note in notes:
        by_name.setdefault(note.rsplit("/", 1)[-1][:-3].lower(), []).append(note)
    attachments = {}
    for f in files:
        if not f.endswith(".md"):
            base = f.rsplit("/", 1)[-1].lower()
            attachments[base] = attachments.get(base, 0) + 1

    total = path_form = dead = path_form_dead = 0
    into_knowledge = set()

    for note in notes:
        text = open(os.path.join(root, note), encoding="utf-8", errors="replace").read()
        folder = posixpath.dirname(note)
        for raw in LINK.findall(text):
            target = raw.split("|")[0].split("#")[0].strip()
            if not target:
                continue
            total += 1
            is_path = "/" in target
            if is_path:
                path_form += 1

            def stem(s):
                return s[:-3].lower() if s.lower().endswith(".md") else s.lower()

            base = target.rsplit("/", 1)[-1]
            joined = posixpath.join(folder, target) if folder else target
            relative = posixpath.normpath(joined)
            escaped = relative.startswith("..")

            # The three rungs, in Obsidian's order.
            hit = None
            if is_path and stem(target) in note_ids:
                hit = stem(target)
            elif not escaped and stem(relative) in note_ids:
                hit = stem(relative)
            elif len(by_name.get(stem(base), [])) == 1:
                hit = by_name[stem(base)][0][:-3].lower()

            if hit is None:
                found_file = (
                    target.lower() in every_id
                    or (not escaped and relative.lower() in every_id)
                    or attachments.get(base.lower()) == 1
                )
                if not found_file:
                    dead += 1
                    if is_path:
                        path_form_dead += 1
            elif hit.startswith("knowledge/") and not note.startswith("Knowledge/"):
                into_knowledge.add(hit)

    return {
        "notes": len(notes),
        "notes under Knowledge/": sum(1 for n in notes if n.startswith("Knowledge/")),
        "wikilinks total": total,
        "path-form wikilinks": path_form,
        "path-form wikilinks dead": path_form_dead,
        "dead wikilinks": dead,
        "ambiguous basenames": sum(1 for v in by_name.values() if len(v) > 1),
        "notes in Knowledge/ linked from outside": len(into_knowledge),
    }


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__.strip().splitlines()[2].strip())
    for label, value in measure(os.path.expanduser(sys.argv[1])).items():
        print(f"{label:<42} {value:>7}")
