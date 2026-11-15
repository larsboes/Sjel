#!/usr/bin/env python3
"""Report GPUI pins and publication-specific GPUI Kit references."""

from __future__ import annotations

import argparse
import json
import os
import sys
import tomllib
from pathlib import Path

ZED_GIT = "https://github.com/zed-industries/zed"
KIT_GIT = "https://github.com/longbridge/gpui-kit"
GPUI_PACKAGES = ("gpui", "gpui_platform", "gpui-kit")


def report_kit(manifest: Path) -> None:
    lock_path = manifest.with_name("Cargo.lock")
    lock = tomllib.loads(lock_path.read_text(encoding="utf-8"))
    packages = [
        package for package in lock.get("package", [])
        if package.get("name") == "gpui-kit"
    ]
    if len(packages) != 1:
        raise ValueError(f"Expected one locked gpui-kit package in {lock_path}")
    package = packages[0]
    version = package["version"]
    if package.get("source") != "registry+https://github.com/rust-lang/crates.io-index":
        raise ValueError("gpui-kit is patched or uses another source; inspect Cargo.lock and that source")
    print(f"  locked version: {version}")
    print(f"  package source: https://docs.rs/crate/gpui-kit/{version}/source/")
    cargo_home = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")))
    cached = list((cargo_home / "registry/src").glob(f"*/gpui-kit-{version}/.cargo_vcs_info.json"))
    if len(cached) != 1:
        print("  publication revision unavailable: inspect the versioned package source; do not substitute main")
        return
    vcs = json.loads(cached[0].read_text(encoding="utf-8"))
    revision = vcs["git"]["sha1"]
    print(f"  publication revision: {revision}")
    print(f"  cached source: {cached[0].parent}")
    print(f"  crate source: {KIT_GIT}/tree/{revision}/{vcs['path_in_vcs']}")
    for title, path in (
        ("conventions", "skills/gpui-kit/references/conventions.md"),
        ("recipes", "skills/gpui-kit/references/recipes.md"),
        ("coding", "skills/gpui-kit/references/coding-guides.md"),
        ("testing", "skills/gpui-kit/references/gpui/test.md"),
        ("design", "skills/gpui-kit-design-guides/references/design-guides.md"),
    ):
        print(f"  {title}: {KIT_GIT}/blob/{revision}/{path}")


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
        description="Show GPUI dependency sources and release-specific GPUI Kit references."
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
        print(f"No gpui, gpui_platform or gpui-kit dependency found in {manifest}", file=sys.stderr)
        return 1

    revisions: set[str] = set()
    for name, (source, version, revision) in sorted(dependencies.items()):
        if source == "registry":
            print(f"{name}: crates.io version {version or '(unspecified)'}")
            if name == "gpui-kit":
                try:
                    report_kit(manifest)
                except (OSError, ValueError, KeyError) as error:
                    print(f"Cannot resolve GPUI Kit publication: {error}", file=sys.stderr)
                    return 2
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
