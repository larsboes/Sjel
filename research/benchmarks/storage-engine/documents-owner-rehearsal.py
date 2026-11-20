#!/usr/bin/env python3
"""Compare SQLite and native Documents against disposable copies of local metadata."""

import argparse
import contextlib
import json
import os
import shutil
import socket
import sqlite3
import statistics
import subprocess
import tempfile
import time
from pathlib import Path
from urllib.error import URLError
from urllib.parse import urlencode
from urllib.request import ProxyHandler, Request, build_opener

TOKEN = "documents-owner-rehearsal-fixture"
CLIENT = build_opener(ProxyHandler({}))


def get(port, path, **query):
    url = f"http://127.0.0.1:{port}{path}?{urlencode(query)}"
    request = Request(url, headers={"Authorization": f"Bearer {TOKEN}"})
    with CLIENT.open(request, timeout=20) as response:
        return json.load(response)


@contextlib.contextmanager
def owner(binary, database, root):
    root.mkdir()
    (root / "config").mkdir()
    secrets = root / "secrets"
    secrets.mkdir(mode=0o700)
    credential = secrets / "fixture"
    credential.write_text(TOKEN)
    (root / "config/deployment.env").write_text(f"SJEL_INBOUND_TOKEN_FILE={credential}\n")
    copied = root / "metadata.db"
    shutil.copyfile(database, copied)
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    environment = dict(
        os.environ, SJEL_PERSONAL_ROOT=str(root), SJEL_OVERLAY_ROOT=str(root),
        SJEL_DB_PATH=str(copied), SJEL_DOCUMENTS_DB_PATH=str(copied), SJEL_PORT=str(port),
    )
    with (root / "owner.log").open("w") as log:
        process = subprocess.Popen([binary, "serve"], env=environment, stdout=log, stderr=log)
        try:
            for _ in range(100):
                try:
                    get(port, "/health")
                    break
                except URLError:
                    if process.poll() is not None:
                        raise RuntimeError(f"owner exited with status {process.returncode}: {(root / 'owner.log').read_text().splitlines()[-1:]}")
                    time.sleep(0.05)
            else:
                raise RuntimeError("owner did not become ready")
            yield port, environment
        finally:
            process.terminate()
            process.wait(timeout=10)


def project(page):
    return (
        page["total"],
        [(row["id"], row.get("matched_pages", [])) for row in page["documents"]],
    )


def checks(database):
    with contextlib.closing(sqlite3.connect(database.as_uri() + "?immutable=1", uri=True)) as db:
        names = db.execute(
            "SELECT filename FROM documents_items ORDER BY imported_at DESC, id LIMIT 48"
        ).fetchall()
    words = [
        word[:max(3, len(word) // 2)] for index, (name,) in enumerate(names)
        for word in name.removesuffix(".pdf").split()
        if index % 4 == 0 and word.isalnum() and len(word) > 4 and word.isalpha()
    ][:12]
    terms = ["Verspätung", "Verspatung", "Gutschein", "Rech", "ticket Bahn", "Köln", *words]
    for index, term in enumerate(terms):
        yield {"q": term, "limit": 7, "offset": index % 4}
    for status in ("filed", "inbox", "archived"):
        for offset in (0, 2, 10):
            yield {"q": "Rech Bah", "status": status, "limit": 3, "offset": offset}


def run(args):
    times = []
    cases = list(checks(args.control_db.resolve()))
    with tempfile.TemporaryDirectory(prefix="sjel-doc-search-", dir="/tmp") as path:
        root = Path(path)
        with owner(args.control_bin, args.control_db, root / "control") as (control, _):
            with owner(args.native_bin, args.native_db, root / "native") as (native, environment):
                for index, query in enumerate(cases):
                    expected = project(get(control, "/api/documents", **query))
                    started = time.monotonic()
                    observed = project(get(native, "/api/documents", **query))
                    times.append(time.monotonic() - started)
                    if expected != observed:
                        raise ValueError(
                            f"search case {index}: result parity failed "
                            f"({expected[0]} SQLite, {observed[0]} Turso)"
                        )
                cli = subprocess.run(
                    [args.native_bin, "search", "Gutschein", "--limit", "3"],
                    env=environment, stdout=subprocess.DEVNULL, check=False,
                )
                if cli.returncode != 0:
                    raise ValueError("native owner CLI search failed")
                subprocess.run([args.native_bin, "snapshot"], env=environment, check=True)
                snapshot = root / "native/data/documents/snapshots/documents.db"
                with contextlib.closing(sqlite3.connect(snapshot)) as restored:
                    integrity = restored.execute("PRAGMA integrity_check").fetchall()
                    violations = restored.execute("PRAGMA foreign_key_check").fetchall()
                    if integrity != [("ok",)] or violations:
                        raise ValueError("native metadata snapshot failed integrity checks")
    print(f"{len(cases)} search pages matched in order and count")
    print(f"Turso search median {statistics.median(times):.3f}s, max {max(times):.3f}s")
    print("Protected CLI and native snapshot passed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("control_bin", "native_bin", "control_db", "native_db"):
        parser.add_argument(name, type=Path)
    run(parser.parse_args())
