#!/usr/bin/env python3
"""A second implementation of `vault journal`, whose job is to disagree with the first.

    python3 capabilities/vault/acceptance/journal-health.py <vault-root>
    vault journal --root <vault-root>

Run both against the same vault at the same moment and compare, the same
contract `link-counts.py` holds for `vault links`. A saved count over a vault a
human edits every day is a test that cannot fail; two implementations that must
agree today do not have that problem.

**It shares no code with the crate on purpose, and it must keep sharing none.**
The crate finds a key line with `str::strip_prefix`, walks the value byte by
byte tracking quote state, and resolves person links through
`graph::targets_for_test`. This uses a line regex, a character loop written
separately, and its own `[[...]]` pattern. Making them agree by making them the
same code would delete the only thing this file is for. Where they disagree,
find out which is wrong and write the reason down.

The one number they are known to differ on is the wikilink pattern, for the
reason `link-counts.py` already documents: this regex cannot cross a `]`, so a
`[[[Name]]]` re-anchors here and does not in the crate. No `Atlas/People` note
in this vault is named that way, so `social` is unaffected today — but if the
person counts ever diverge, that is the first place to look.

Read-only. It opens files under the root and writes nothing anywhere.
"""
import os
import re
import sys

KEYS = ["exercise", "social", "sleep_quality", "energy", "mood", "learning_hours"]
DAILY = os.path.join("Journal", "01. Daily Notes")
PEOPLE = os.path.join("Atlas", "People")

# Deliberately not the crate's scanner. See the module docstring.
LINK = re.compile(r"\[\[([^\]\n]+)\]\]")
KEY_LINE = re.compile(r"^([A-Za-z_][A-Za-z0-9_\-]*):(.*)$")
# The date is the first ten characters of the filename, the same rule the crate
# uses and for the same reason: the filename is what the operator controls.
DATE = re.compile(r"^\d{4}-\d{2}-\d{2}")


def frontmatter(text):
    """The lines between the opening and closing fence, or None."""
    lines = text.split("\n")
    if not lines or lines[0].strip() != "---":
        return None
    for i in range(1, len(lines)):
        if lines[i].strip() in ("---", "..."):
            return lines[1:i]
    return None


def scalar(raw):
    """YAML's comment rule: `#` opens a comment at the start of the value or
    after whitespace, and never inside a quoted scalar."""
    out = []
    quote = None
    for i, c in enumerate(raw):
        if quote:
            out.append(c)
            if c == quote:
                quote = None
        elif c in "\"'":
            quote = c
            out.append(c)
        elif c == "#" and (i == 0 or raw[i - 1] in " \t"):
            break
        else:
            out.append(c)
    return "".join(out).strip().strip('"')


def measure(root):
    daily = os.path.join(root, DAILY)
    people_dir = os.path.join(root, PEOPLE)
    if not os.path.isdir(daily):
        sys.exit("no %s under %s" % (DAILY, root))
    if not os.path.isdir(people_dir):
        sys.exit("no %s under %s" % (PEOPLE, root))

    register = set()
    for dirpath, dirnames, filenames in os.walk(people_dir):
        dirnames[:] = [d for d in dirnames if not d.startswith(".")]
        for name in filenames:
            if name.endswith(".md"):
                register.add(name[:-3].lower())

    census = {k: {"present": 0, "blank": 0, "comment_only": 0, "asserted": 0} for k in KEYS}
    # What the daily template writes into a fresh note, so a value equal to it is
    # not a statement about the day.
    default = {"exercise": "false", "social": "false"}
    days = 0
    not_a_day = 0
    with_link = 0
    stated_true = 0
    agrees = 0
    disagrees = 0
    unfilled_with_evidence = 0
    named = set()
    unrendered = []

    for name in sorted(os.listdir(daily)):
        if not name.endswith(".md"):
            continue
        if not DATE.match(name):
            not_a_day += 1
            continue
        days += 1
        text = open(os.path.join(daily, name), encoding="utf-8", errors="replace").read()
        block = frontmatter(text) or []

        values = {}
        for line in block:
            m = KEY_LINE.match(line)
            if not m:
                continue
            key, raw = m.group(1), m.group(2)
            if "<%" in raw:
                unrendered.append("%s/%s  %s" % (DAILY, name, key))
            if key not in KEYS:
                continue
            value = scalar(raw)
            values[key] = value
            slot = census[key]
            slot["present"] += 1
            if not value:
                slot["blank"] += 1
                if raw.strip():
                    slot["comment_only"] += 1
            elif value != default.get(key):
                slot["asserted"] += 1

        linked = set()
        for m in LINK.finditer(text):
            target = m.group(1).split("|")[0].split("#")[0].strip()
            target = target.rsplit("/", 1)[-1]
            if target.endswith(".md"):
                target = target[:-3]
            if target.lower() in register:
                linked.add(target.lower())
        named |= linked

        social = bool(linked)
        if social:
            with_link += 1
        stated = values.get("social", "")
        if stated == "true":
            stated_true += 1
            if social:
                agrees += 1
            else:
                disagrees += 1
        elif stated == "false" and social:
            unfilled_with_evidence += 1

    print("daily notes            %d" % days)
    print("  not a date           %d" % not_a_day)
    print("people register        %d" % len(register))
    print()
    print("key             present  asserted  comment-only")
    for key in KEYS:
        slot = census[key]
        print(
            "%-14s  %7d  %8d  %12d"
            % (key, slot["present"], slot["asserted"], slot["comment_only"])
        )
    print()
    print("social, produced from Journal person links")
    print("  days naming a person %d" % with_link)
    print("  distinct people      %d" % len(named))
    print("  stored social: true  %d" % stated_true)
    print("    links agree        %d" % agrees)
    print("    links differ       %d" % disagrees)
    print("  template false, links name somebody %d" % unfilled_with_evidence)
    print()
    print("%d frontmatter values are still template expressions:" % len(unrendered))
    for item in unrendered:
        print("  %s" % item)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("usage: journal-health.py <vault-root>")
    measure(sys.argv[1])
