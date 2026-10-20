"""Render .mmd sources with mermaid-cli, and report the aspect each one landed at.

Two things here are deliberate.

**The renderer is pinned.** Sjel's dependency doctrine is a pinned version, never
a floating release; `DIAGRAMS_MERMAID_CLI` exists so a bump is a decision someone
made rather than a side effect of a render. mermaid-cli downloads a Chromium on
first use — see `diagrams doctor`.

**The size is reported.** A figure's aspect ratio is the number that decides
where it can sit on a slide, and it is a property of the node text, not something
anyone predicts. `layout.picture()` fits a box, so knowing the aspect before the
first build saves a render-look-adjust cycle.
"""

from __future__ import annotations

import os
import shutil
import struct
import subprocess
from pathlib import Path

from .errors import DiagramError

#: The version the deck and the thesis both render with today.
CLI = os.environ.get("DIAGRAMS_MERMAID_CLI", "11.17.0")

#: Background and scale are fixed rather than flags: figures are embedded on a
#: white slide, and 4x is the resolution that stays crisp when a 3136px render is
#: placed at 11in wide.
BACKGROUND = "white"
SCALE = "4"


def png_size(path: Path) -> tuple[int, int]:
    """Width and height, from the IHDR chunk. No image library needed."""
    with path.open("rb") as handle:
        header = handle.read(24)
    if len(header) < 24 or header[:8] != b"\x89PNG\r\n\x1a\n":
        raise DiagramError(f"{path} is not a PNG; the renderer wrote something else.")
    return struct.unpack(">II", header[16:24])


def _bunx() -> str:
    found = shutil.which("bunx")
    if not found:
        raise DiagramError(
            "bunx is not on PATH.\n"
            "  Install bun: curl -fsSL https://bun.sh/install | bash"
        )
    return found


def render_one(source: Path, target: Path, theme_path: Path) -> tuple[int, int]:
    target.parent.mkdir(parents=True, exist_ok=True)
    command = [
        _bunx(), f"@mermaid-js/mermaid-cli@{CLI}",
        "-i", str(source),
        "-o", str(target),
        "-c", str(theme_path),
        "-b", BACKGROUND,
        "-s", SCALE,
    ]
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode != 0:
        detail = (result.stderr or result.stdout or "").strip()
        raise DiagramError(
            f"mermaid-cli failed on {source.name} (exit {result.returncode})\n"
            f"{detail}"
        )
    if not target.exists():
        raise DiagramError(
            f"mermaid-cli exited 0 but wrote no file for {source.name}.\n"
            f"  Expected: {target}"
        )
    return png_size(target)


def sources(directory: Path) -> list[Path]:
    found = sorted(directory.glob("*.mmd"))
    if not found:
        raise DiagramError(
            f"no .mmd sources in {directory}\n"
            f"  A diagram source is a Mermaid flowchart; see references/mermaid.md."
        )
    return found
