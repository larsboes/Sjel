#!/bin/bash
# Minimal single-line TOML value extraction, shared by everything that reads
# a manifest (axon.toml, machine.toml, capabilities/*/service.toml). Not a
# real TOML parser — handles `key = "value"` and `key = ["a", "b"]` on one
# line each, which is all Axon's manifests use on purpose (see upstreams.toml
# style). bash 3.2-safe (macOS stock bash), no mapfile/readarray.
#
# The boundary, stated once: anything beyond single-line (array-of-tables like
# [[state_mount]], nested tables, dotted queries) is read in TypeScript through
# Bun.TOML directly -- self.ts, packs-codex.ts, obsidian-deploy.ts and
# libs/overlay each do exactly that. A shared CLI wrapper for it existed until
# 2026-08-02 and never had a caller: the callers are all TS already, and the ones
# that are not are the shell gates, which stay on this file so a manifest check
# needs nothing but bash -- which is exactly why the gates' manifests stay
# single-line.

toml_get() {  # toml_get <key> <file> — empty output, exit 0, if key absent
  grep -E "^$1[[:space:]]*=" "$2" | sed -E "s/^$1[[:space:]]*=[[:space:]]*\"([^\"]*)\".*/\1/" || true
}

_toml_split_array() {  # stdin: `key = ["a", "b"]` lines — one element per output line
  sed -E 's/^[^\[]*\[(.*)\][[:space:]]*$/\1/' \
    | tr ',' '\n' \
    | sed -E 's/^[[:space:]"]*//; s/[[:space:]"]*$//' \
    | grep -v '^$'
}

toml_array() {  # toml_array <key> <file> — one element per output line
  grep -E "^$1[[:space:]]*=[[:space:]]*\[" "$2" | _toml_split_array || true
}

# Section-aware access, for multi-section manifests like upstreams.toml (the
# per-capability service.toml files are single-section, so toml_get suffices
# there). Still not a real parser — same single-line contract, just scoped.

toml_sections() {  # toml_sections <file> — list [section] names, one per line
  grep -E '^\[[^]]+\]' "$1" | sed -E 's/^\[([^]]+)\].*/\1/' || true
}

toml_section() {  # toml_section <name> <file> — print the body lines of [name]
  awk -v s="$1" '
    /^\[/ { line=$0; sub(/[ \t]+$/,"",line); in_s=(line=="[" s "]"); next }
    in_s { print }
  ' "$2"
}

toml_get_in() {  # toml_get_in <section> <key> <file> — key scoped to one section
  toml_section "$1" "$3" \
    | grep -E "^$2[[:space:]]*=" \
    | sed -E "s/^$2[[:space:]]*=[[:space:]]*\"([^\"]*)\".*/\1/" || true
}

toml_array_in() {  # toml_array_in <section> <key> <file> — array scoped to one section
  toml_section "$1" "$3" \
    | grep -E "^$2[[:space:]]*=[[:space:]]*\[" \
    | _toml_split_array || true
}

toml_set() {  # toml_set <key> <value> <file> — in-place write of a top-level scalar
  # Replaces an existing `key = "..."` line, or appends one if absent. `-i.bak`
  # + rm is the portable form that works identically under BSD sed (macOS) and
  # GNU sed (Linux) — plain `-i` differs between the two. `|` as the sed
  # delimiter since values here are filesystem paths (contain `/`).
  local key="$1" val="$2" file="$3"
  if grep -qE "^$key[[:space:]]*=" "$file"; then
    sed -i.bak -E "s|^$key[[:space:]]*=.*|$key = \"$val\"|" "$file"
    rm -f "$file.bak"
  else
    printf '%s = "%s"\n' "$key" "$val" >> "$file"
  fi
}
