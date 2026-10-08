#!/usr/bin/env python3
"""Report GPUI Git pins and revision-specific upstream source links."""

from __future__ import annotations

import argparse
import sys
import tomllib
from pathlib import Path

ZED_GIT = "https://github.com/zed-industries/zed"
GPUI_PACKAGES = ("gpui", "gpui_platform")


def dependency_table(value: object) -> tuple[str, str, str]:
    if isinstance(value, str):
        return ("registry", value, "")
    if not isinstance(value, dict):
        return ("unknown", repr(value), "")
    source = str(value.get("git", "registry"))
    revision = str(value.get("rev", ""))
    version = str(value.get("version", ""))
    return (source, version, revision)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Show GPUI dependency sources and revision-specific Zed links."
    )
    parser.add_argument("manifest", type=Path, help="Cargo.toml to inspect")
    args = parser.parse_args()
    manifest = args.manifest.expanduser()
    try:
        data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        print(f"Cannot read Cargo manifest {manifest}: {error}", file=sys.stderr)
        return 2

    dependencies: dict[str, tuple[str, str, str]] = {}
    for section_name in ("dependencies", "dev-dependencies", "build-dependencies", "workspace"):
        section = data.get(section_name, {})
        if section_name == "workspace" and isinstance(section, dict):
            section = section.get("dependencies", {})
        if not isinstance(section, dict):
            continue
        for name in GPUI_PACKAGES:
            if name in section:
                dependencies[name] = dependency_table(section[name])

    if not dependencies:
        print(f"No gpui or gpui_platform dependency found in {manifest}", file=sys.stderr)
        return 1

    revisions: set[str] = set()
    for name, (source, version, revision) in sorted(dependencies.items()):
        if source == "registry":
            print(f"{name}: crates.io version {version or '(unspecified)'}")
            continue
        print(f"{name}: {source}")
        if source.rstrip("/") == ZED_GIT and revision:
            revisions.add(revision)
            print(f"  revision: {revision}")
            print(f"  crate source: {ZED_GIT}/tree/{revision}/crates/{name}")
            print(f"  README: {ZED_GIT}/blob/{revision}/crates/gpui/README.md")
        elif source.rstrip("/") == ZED_GIT:
            print("  no immutable rev is declared; inspect Cargo.lock before using source examples")
        else:
            print(f"  version: {version or '(not declared)'}; source is not the Zed repository")

    if len(revisions) > 1:
        print("GPUI crates use different Zed revisions; align them before relying on API compatibility.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
