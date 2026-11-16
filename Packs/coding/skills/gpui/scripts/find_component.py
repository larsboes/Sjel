#!/usr/bin/env python3
"""Find a component in locked Cargo sources and the consuming workspace."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


def normalized(text: str) -> str:
    return re.sub(r"[^a-z0-9]", "", text.lower())


def matches(path: Path, query: str, public_only: bool = False) -> list[str]:
    """Return locations, not an inferred API or compatibility verdict."""
    needle = normalized(query)
    found = []
    for source in sorted((path / "src").rglob("*.rs")):
        relative = source.relative_to(path)
        path_match = needle in normalized(str(relative))
        for number, line in enumerate(source.read_text(encoding="utf-8").splitlines(), 1):
            if public_only and not re.match(r"\s*pub\s+", line):
                continue
            if needle in normalized(line) or (public_only and path_match):
                found.append(f"{source}:{number}: {line.strip()}")
    return found


def package_source(manifest: Path, package: str) -> tuple[Path, str]:
    compiler = subprocess.run(["rustc", "-vV"], capture_output=True, text=True, check=False)
    host = next((line.removeprefix("host: ") for line in compiler.stdout.splitlines()
                 if line.startswith("host: ")), None)
    if compiler.returncode or not host:
        raise ValueError("Cannot resolve the Rust host target from rustc -vV")
    result = subprocess.run(
        ["cargo", "metadata", "--offline", "--locked", "--format-version", "1",
         "--filter-platform", host, "--manifest-path", str(manifest)],
        capture_output=True, text=True, check=False,
    )
    if result.returncode:
        raise ValueError(f"Locked offline Cargo metadata failed:\n{result.stderr.strip()}")
    metadata = json.loads(result.stdout)
    candidates = [entry for entry in metadata["packages"] if entry["name"] == package]
    if len(candidates) != 1:
        raise ValueError(f"Expected one locked {package} package; found {len(candidates)}")
    entry = candidates[0]
    source = entry.get("source")
    if not source:
        raise ValueError(f"{package} is locally patched; inspect that checkout before relying on the lock pin")
    return Path(entry["manifest_path"]).parent, f"{entry['name']} {entry['version']} {source}"


def test_candidates(root: Path, query: str) -> list[str]:
    found = []
    pattern = re.compile(r"#\[(?:gpui::)?test(?:\([^\]]*\))?\]\s*(?:#\[[^\]]+\]\s*)*((?:async\s+)?fn\s+\w+)")
    sources = [(path, path.read_text(encoding="utf-8"))
               for path in sorted((root / "src").rglob("*.rs"))]
    homes = {path.parent for path, body in sources
             if any(normalized(name) == normalized(query) for name in
                    re.findall(r"\bpub\s+(?:struct|enum|trait|type)\s+(\w+)", body))}
    sources.sort(key=lambda entry: (not any(home in entry[0].parents for home in homes), entry[0]))
    for path, body in sources:
        if not any(home in path.parents for home in homes) and (
            normalized(query) not in normalized(str(path.relative_to(root)) + body)
        ):
            continue
        for match in pattern.finditer(body):
            number = body.count("\n", 0, match.start(1)) + 1
            found.append(f"{path}:{number}: {match.group(1)}")
    return found


def emit(title: str, results: list[str], limit: int) -> None:
    print(f"\n{title}: {len(results)} matching locations")
    for line in results[:limit]:
        print(f"  {line}")
    if len(results) > limit:
        print(f"  {len(results) - limit} locations omitted; increase --limit to inspect them")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path, help="Consumer workspace Cargo.toml")
    parser.add_argument("component", help="Component or module name, e.g. DataTable or terminal")
    parser.add_argument("--package", default="ely-gpui-component", help="Locked dependency to inspect")
    parser.add_argument("--limit", type=int, default=12, help="Maximum locations per section")
    args = parser.parse_args(argv)
    if not normalized(args.component) or args.limit < 1:
        parser.error("component must contain letters or digits and --limit must be positive")
    manifest = args.manifest.expanduser().resolve()
    try:
        source, identity = package_source(manifest, args.package)
        print(f"Locked dependency: {identity}\nSource: {source}")
        emit("Public source declarations", matches(source, args.component, True), args.limit)
        roots = [manifest.parent]
        roots.extend(directory for directory in sorted(manifest.parent.iterdir())
                     if directory.is_dir() and (directory / "Cargo.toml").is_file())
        usages = [line for root in roots for line in matches(root, args.component)]
        emit("Consumer usages", usages, args.limit)
        for title, test_roots in [("Dependency test candidates", [source]),
                                  ("Consumer test candidates", roots)]:
            tests = [location for root in test_roots for location in test_candidates(root, args.component)]
            emit(title, tests, args.limit)
        print("\nSource matches are navigation, not proof of feature parity. No network or build was requested.")
    except (OSError, ValueError, KeyError) as error:
        print(f"Component lookup failed: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
