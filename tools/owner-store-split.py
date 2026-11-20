#!/usr/bin/env python3
"""Stage an owner database from a restored shared-store backup."""

import argparse
import os
import sqlite3
import tempfile
from contextlib import closing
from pathlib import Path

OWNERS = {
    "calendar": {
        "calendar_contexts",
        "calendar_entries",
        "calendar_entry_tombstones",
        "calendar_google_exports",
        "calendar_rhythms",
        "calendar_trip_materializations",
    },
    "punctuality": {
        "punctuality_ingest_months",
        "punctuality_ingest_runs",
        "punctuality_stations",
        "punctuality_stop_stats",
    },
    "software": {"software_apps", "software_decisions"},
}


def identifier(name):
    return '"' + name.replace('"', '""') + '"'


def stage(owner, source_path, destination):
    if not source_path.is_file():
        raise ValueError("a restored shared database file is required")
    if destination.exists():
        raise FileExistsError(destination)
    with tempfile.TemporaryDirectory(dir=destination.parent) as temporary:
        candidate = Path(temporary) / destination.name
        with (
            closing(sqlite3.connect(source_path.resolve().as_uri() + "?mode=ro", uri=True)) as source,
            closing(sqlite3.connect(candidate)) as target,
        ):
            source.execute("BEGIN")
            catalog = source.execute(
                "SELECT type, name, tbl_name, sql FROM sqlite_schema "
                "WHERE tbl_name GLOB ? AND sql IS NOT NULL ORDER BY type, name",
                (f"{owner}_*",),
            ).fetchall()
            tables = {name for kind, name, _, _ in catalog if kind == "table"}
            if tables != OWNERS[owner]:
                raise ValueError(f"the shared database has an unexpected {owner} schema")
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
            if target.execute(
                "SELECT 1 FROM sqlite_schema WHERE name = 'sqlite_sequence'"
            ).fetchone():
                counters = source.execute(
                    "SELECT name, seq FROM sqlite_sequence WHERE name GLOB ?", (f"{owner}_*",)
                ).fetchall()
                if any(name not in tables for name, _ in counters):
                    raise ValueError("owner sequence references an unknown table")
                target.execute("DELETE FROM sqlite_sequence")
                target.executemany("INSERT INTO sqlite_sequence VALUES (?, ?)", counters)
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
                raise ValueError("owner database failed integrity_check")
            if target.execute("PRAGMA foreign_key_check").fetchone() is not None:
                raise ValueError("owner database failed foreign_key_check")
            target.commit()
            target.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        candidate.chmod(0o600)
        os.link(candidate, destination)
    return counts


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("owner", choices=sorted(OWNERS))
    parser.add_argument("restored_database", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    for name, count in stage(args.owner, args.restored_database, args.destination).items():
        print(f"{name} {count}")
