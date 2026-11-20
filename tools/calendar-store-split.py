#!/usr/bin/env python3
"""Stage Calendar's owner database from a restored shared-store backup."""

import argparse
from contextlib import closing
import os
import sqlite3
import tempfile
from pathlib import Path

TABLES = {
    "calendar_contexts",
    "calendar_entries",
    "calendar_entry_tombstones",
    "calendar_google_exports",
    "calendar_rhythms",
    "calendar_trip_materializations",
}


def identifier(name):
    return '"' + name.replace('"', '""') + '"'


def stage(source_path, destination):
    if not source_path.is_file():
        raise ValueError("a restored shared database file is required")
    if destination.exists():
        raise FileExistsError(destination)
    with tempfile.TemporaryDirectory(dir=destination.parent) as temporary:
        candidate = Path(temporary) / "calendar.db"
        with (
            closing(sqlite3.connect(source_path.resolve().as_uri() + "?mode=ro", uri=True)) as source,
            closing(sqlite3.connect(candidate)) as target,
        ):
            source.execute("BEGIN")
            catalog = source.execute(
                "SELECT type, name, tbl_name, sql FROM sqlite_schema "
                "WHERE tbl_name GLOB 'calendar_*' AND sql IS NOT NULL ORDER BY type, name"
            ).fetchall()
            tables = {name for kind, name, _, _ in catalog if kind == "table"}
            if tables != TABLES:
                raise ValueError("the shared database has an unexpected Calendar schema")
            target.execute("PRAGMA journal_mode=WAL")
            target.execute("BEGIN IMMEDIATE")
            for kind, _, _, sql in catalog:
                if kind == "table":
                    target.execute(sql)
            counts = {}
            column_counts = {}
            for name in sorted(tables):
                table = identifier(name)
                rows = source.execute(f"SELECT * FROM {table}")
                columns = ",".join(identifier(column[0]) for column in rows.description)
                parameters = ",".join("?" for _ in rows.description)
                inserted = target.executemany(
                    f"INSERT INTO {table} ({columns}) VALUES ({parameters})", rows
                )
                counts[name] = inserted.rowcount
                column_counts[name] = len(rows.description)
            for kind, _, _, sql in catalog:
                if kind != "table":
                    target.execute(sql)
            for name in sorted(tables):
                table = identifier(name)
                order = ",".join(map(str, range(1, column_counts[name] + 1)))
                sql = f"SELECT * FROM {table} ORDER BY {order}"
                pairs = zip(source.execute(sql), target.execute(sql), strict=True)
                if any(left != right for left, right in pairs):
                    raise ValueError(f"row mismatch in {name}")
            if target.execute("PRAGMA integrity_check").fetchall() != [("ok",)]:
                raise ValueError("Calendar database failed integrity_check")
            if target.execute("PRAGMA foreign_key_check").fetchone() is not None:
                raise ValueError("Calendar database failed foreign_key_check")
            target.commit()
            target.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        candidate.chmod(0o600)
        os.link(candidate, destination)
    return counts


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("restored_database", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    for name, count in stage(args.restored_database, args.destination).items():
        print(f"{name} {count}")
