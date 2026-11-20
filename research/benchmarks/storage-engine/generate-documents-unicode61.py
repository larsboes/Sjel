#!/usr/bin/env python3
"""Generate Documents' tokenizer tables from the bundled SQLite amalgamation."""

import argparse
import subprocess
import tempfile
from pathlib import Path

DRIVER = r"""
int main(void) {
  u8 categories[32] = {0}, ascii[128] = {0};
  for (const char **c = (const char *[]){"L*", "N*", "Co", NULL}; *c; ++c) {
    if (sqlite3Fts5UnicodeCatParse(*c, categories) != 0) return 1;
  }
  sqlite3Fts5UnicodeAscii(categories, ascii);
  for (int c = 0; c <= 0x10ffff; ++c) {
    int letter = c < 128 ? ascii[c] : categories[sqlite3Fts5UnicodeCategory(c)];
    int kind = letter ? 1 : sqlite3Fts5UnicodeIsdiacritic(c) ? 2 : 0;
    if (kind) printf("%x %d %x\n", c, kind, sqlite3Fts5UnicodeFold(c, 2));
  }
}
"""


def generate(amalgamation):
    source = amalgamation.read_text()
    start = source.index("static int fts5_remove_diacritic(int c, int bComplex){")
    end = source.index("\n}", source.index("static void sqlite3Fts5UnicodeAscii(u8 *aArray", start))
    includes = "#include <assert.h>\n#include <stdio.h>\n"
    types = "typedef unsigned char u8; typedef unsigned short u16; typedef unsigned int u32;\n"
    with tempfile.TemporaryDirectory() as directory:
        driver = Path(directory, "unicode61.c")
        binary = Path(directory, "unicode61")
        driver.write_text(includes + types + source[start:end + 2] + DRIVER)
        subprocess.run(["cc", "-O2", str(driver), "-o", str(binary)], check=True)
        data = subprocess.check_output([binary], text=True)

    ranges = []
    folds = []
    for line in data.splitlines():
        character, kind, folded = (int(value, 16) for value in line.split())
        if ranges and ranges[-1][1] + 1 == character and ranges[-1][2] == kind:
            ranges[-1][1] = character
        else:
            ranges.append([character, character, kind])
        if character != folded:
            folds.append((character, folded))
    print("// Generated from SQLite's public-domain unicode61 tables.")
    print("// Regenerate with research/benchmarks/storage-engine/generate-documents-unicode61.py.")
    for name, type_name, rows, row_format in (
        ("CLASSES", "(u32, u32, u8)", ranges, "({:#x}, {:#x}, {})"),
        ("FOLDS", "(u32, u32)", folds, "({:#x}, {:#x})"),
    ):
        print(f"\n#[rustfmt::skip]\npub(super) const {name}: &[{type_name}] = &[")
        for index in range(0, len(rows), 4):
            print("    " + ", ".join(row_format.format(*row) for row in rows[index:index + 4]) + ",")
        print("];")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("amalgamation", type=Path)
    generate(parser.parse_args().amalgamation)
