#!/bin/bash
# tools/lib/delta.sh — the version-to-version delta, shared by tools/update.sh (the consumer's
# "what would I get if I updated" preview) and tools/release (the notes for a new tag). One home
# so the two never drift (README.md#documentation-stays-owned-and-current). Sourced AFTER tools/lib/paths.sh (which exports SJEL_ROOT and
# sources toml.sh); it pulls in version.sh itself. bash 3.2-safe (README.md#portable-shell), no bash-4 syntax.
#
# The delta is computed live from git + the manifests — there is no committed CHANGELOG to rot
# (README.md#documentation-stays-owned-and-current, same reason ARCHITECTURE.md is generated). Everything degrades gracefully before any
# release tag exists: latest_release_ref() returns empty and callers fall back to origin/main.

_delta_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
# shellcheck source=tools/lib/version.sh
. "$_delta_dir/version.sh"                                   # ver_gt, ver_numeric, norm_ver
command -v toml_sections >/dev/null 2>&1 || . "$_delta_dir/toml.sh"   # defensive: normally via paths.sh

# latest_release_ref — the highest semver tag (vX.Y.Z), or empty if there are no release tags yet.
# `--sort=-v:refname` gives descending version order; the first tag that is actually a dotted
# number (norm_ver strips the leading v) wins, so a stray `v1.0.0-rc1` or a non-version tag is
# skipped rather than mistaken for the latest release.
latest_release_ref() {
  local t glob
  glob="$(release_tag_glob)" || return 1
  for t in $(git -C "$SJEL_ROOT" tag -l "$glob" --sort=-v:refname 2>/dev/null); do
    if ver_numeric "$(norm_ver "$t")"; then printf '%s\n' "$t"; return 0; fi
  done
  return 0
}

# caps_at_ref <ref> — capability directory names present at <ref> (one per line, empty if none).
caps_at_ref() {
  git -C "$SJEL_ROOT" ls-tree -d --name-only "$1:capabilities" 2>/dev/null || true
}

# _sections_to_file <ref> <file> <sections_out> <toml_out> — dump <file> as it was at <ref> into
# <toml_out> and its sorted [section] names into <sections_out>. Both empty if the file didn't
# exist at that ref (e.g. toolchain.toml at an older tag) — that reads as "all added" across the
# boundary, which is truthful.
_sections_to_file() {
  local ref="$1" file="$2" sec_out="$3" toml_out="$4"
  : > "$sec_out"; : > "$toml_out"
  if git -C "$SJEL_ROOT" show "$ref:$file" > "$toml_out" 2>/dev/null; then
    toml_sections "$toml_out" | sort > "$sec_out"
  fi
}

# print_manifest_delta <from_ref> <to_ref> — categorized "what changed moving from -> to":
# capabilities, upstreams (incl. verdict changes), toolchain, and a capped commit summary.
print_manifest_delta() {
  local from="$1" to="$2" r
  for r in "$from" "$to"; do
    if ! git -C "$SJEL_ROOT" rev-parse --verify --quiet "$r^{commit}" >/dev/null 2>&1; then
      echo "  (delta unavailable — ref '$r' not found; fetch first?)"
      return 0
    fi
  done

  local d; d="$(mktemp -d)"

  # --- capabilities: dir add/remove ---
  caps_at_ref "$from" | sort > "$d/caps_from"
  caps_at_ref "$to"   | sort > "$d/caps_to"
  echo "Capabilities:"
  local ca cr
  ca="$(comm -13 "$d/caps_from" "$d/caps_to")"
  cr="$(comm -23 "$d/caps_from" "$d/caps_to")"
  if [ -n "$ca$cr" ]; then
    [ -n "$ca" ] && printf '%s\n' "$ca" | sed 's/^/  + /'
    [ -n "$cr" ] && printf '%s\n' "$cr" | sed 's/^/  - /'
  else
    echo "  (no change)"
  fi
  echo

  # --- upstreams: section add/remove + verdict change over the common set ---
  _sections_to_file "$from" upstreams.toml "$d/up_from" "$d/up_from.toml"
  _sections_to_file "$to"   upstreams.toml "$d/up_to"   "$d/up_to.toml"
  echo "Upstreams:"
  local ua ur any=0 s vf vt
  ua="$(comm -13 "$d/up_from" "$d/up_to")"
  ur="$(comm -23 "$d/up_from" "$d/up_to")"
  [ -n "$ua" ] && { printf '%s\n' "$ua" | sed 's/^/  + /'; any=1; }
  [ -n "$ur" ] && { printf '%s\n' "$ur" | sed 's/^/  - /'; any=1; }
  while IFS= read -r s; do
    [ -n "$s" ] || continue
    vf="$(toml_get_in "$s" verdict "$d/up_from.toml")"
    vt="$(toml_get_in "$s" verdict "$d/up_to.toml")"
    if [ "$vf" != "$vt" ]; then printf '  ~ %s: %s → %s\n' "$s" "${vf:-—}" "${vt:-—}"; any=1; fi
  done <<EOF
$(comm -12 "$d/up_from" "$d/up_to")
EOF
  [ "$any" -eq 0 ] && echo "  (no change)"
  echo

  # --- toolchain: tool add/remove + required-class change ---
  _sections_to_file "$from" toolchain.toml "$d/tc_from" "$d/tc_from.toml"
  _sections_to_file "$to"   toolchain.toml "$d/tc_to"   "$d/tc_to.toml"
  echo "Toolchain:"
  local ta tr rf rt
  any=0
  ta="$(comm -13 "$d/tc_from" "$d/tc_to")"
  tr="$(comm -23 "$d/tc_from" "$d/tc_to")"
  [ -n "$ta" ] && { printf '%s\n' "$ta" | sed 's/^/  + /'; any=1; }
  [ -n "$tr" ] && { printf '%s\n' "$tr" | sed 's/^/  - /'; any=1; }
  while IFS= read -r s; do
    [ -n "$s" ] || continue
    rf="$(toml_get_in "$s" required "$d/tc_from.toml")"
    rt="$(toml_get_in "$s" required "$d/tc_to.toml")"
    if [ "$rf" != "$rt" ]; then printf '  ~ %s: %s → %s\n' "$s" "${rf:-—}" "${rt:-—}"; any=1; fi
  done <<EOF
$(comm -12 "$d/tc_from" "$d/tc_to")
EOF
  [ "$any" -eq 0 ] && echo "  (no change)"
  echo

  # --- commits: capped, with an explicit remainder count (no silent truncation, README.md#documentation-stays-owned-and-current) ---
  echo "Commits ($from..$to):"
  local log n cap=15
  log="$(git -C "$SJEL_ROOT" log --oneline "$from..$to" 2>/dev/null || true)"
  if [ -n "$log" ]; then
    n="$(printf '%s\n' "$log" | wc -l | tr -d ' ')"
    printf '%s\n' "$log" | head -"$cap" | sed 's/^/  /'
    [ "$n" -gt "$cap" ] && echo "  … +$((n - cap)) more"
  else
    echo "  (none)"
  fi

  rm -rf "$d"
}
