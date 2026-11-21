#!/bin/bash
# staged-rustfmt.sh — refuse a commit whose staged Rust files rustfmt would change.
#
# Reads the staged blob, not the working tree, so unstaged edits from another session never
# block this commit. Each file takes the edition of its nearest Cargo.toml and the toolchain of
# its own directory. Pass paths to check them instead of the index (used by the tests).
# bash 3.2-safe (CONTRIBUTING.md#portable-shell).
set -u

root="$(git rev-parse --show-toplevel)"
if ! command -v rustfmt >/dev/null 2>&1; then
  echo "staged-rustfmt: rustfmt is not installed — staged Rust was NOT format-checked" >&2
  exit 0
fi

# edition_for <repo-relative file>: the first `edition = "…"` walking up from the file.
edition_for() {
  local dir manifest edition
  dir="$(dirname "$1")"
  while :; do
    if [ "$dir" = . ]; then manifest="$root/Cargo.toml"; else manifest="$root/$dir/Cargo.toml"; fi
    if [ -f "$manifest" ]; then
      edition="$(sed -n 's/^edition *= *"\([0-9]*\)".*/\1/p' "$manifest" | head -1)"
      if [ -n "$edition" ]; then echo "$edition"; return; fi
    fi
    [ "$dir" = . ] && break
    dir="$(dirname "$dir")"
  done
  echo 2021
}

status=0
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
# `rustfmt --check` exits 0 on stdin even when it prints a diff, so compare the output instead.
check() {
  local file="$1" dir
  # Outside both Cargo workspaces that CI formats.
  case "$file" in research/*) return ;; esac
  dir="$root/$(dirname "$file")"
  [ -d "$dir" ] || dir="$root"
  git -C "$root" show ":$file" > "$scratch/staged.rs"
  if ! (cd "$dir" && rustfmt --edition "$(edition_for "$file")" --emit stdout) \
      < "$scratch/staged.rs" > "$scratch/formatted.rs" 2>/dev/null \
    || ! cmp -s "$scratch/staged.rs" "$scratch/formatted.rs"; then
    echo "staged-rustfmt: $file is not formatted — run: rustfmt --edition $(edition_for "$file") $file" >&2
    status=1
  fi
}

if [ "$#" -gt 0 ]; then
  for file in "$@"; do check "$file"; done
else
  while IFS= read -r -d '' file; do check "$file"; done \
    < <(git -C "$root" diff --cached --name-only --diff-filter=ACMR -z -- '*.rs')
fi
exit "$status"
