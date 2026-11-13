#!/bin/bash
# Signs a built binary with this Mac's code-signing identity and a fixed identifier.
#
# macOS names a binary by its code signature. An ad-hoc signature changes with every build, so
# the application firewall and privacy grants treat each rebuild as a new app: the Same Wi-Fi
# listener (PRD Q119) went dark after the first rebuild on 2026-09-25 until it was allowed again.
# A stable identity and identifier keep the grant across rebuilds.
#
#   tools/codesign-binary.sh <path> <identifier> [--require-identity]
set -euo pipefail
bin="${1:?usage: codesign-binary.sh <path> <identifier>}"
id="${2:?usage: codesign-binary.sh <path> <identifier> [--require-identity]}"
require_identity=0
case "${3:-}" in
  "") ;;
  --require-identity) require_identity=1 ;;
  *) echo "codesign-binary: unknown option: $3" >&2; exit 2 ;;
esac
[ "$#" -le 3 ] || { echo "codesign-binary: too many arguments" >&2; exit 2; }
[ "$(uname -s)" = Darwin ] || exit 0

identity="${SJEL_CODESIGN_IDENTITY:-}"
if [ -z "$identity" ]; then
  identity="$(security find-identity -v -p codesigning | awk 'NR==1 && $2 ~ /^[0-9A-F]{40}$/ {print $2}')"
fi
if [ -z "$identity" ]; then
  if [ "$require_identity" -eq 1 ]; then
    echo "codesign-binary: a code-signing identity is required for this helper" >&2
    exit 1
  fi
  # Other binaries may retain an existing ad-hoc signature.
  echo "codesign-binary: no code-signing identity; $bin stays ad-hoc signed" >&2
  exit 0
fi
# Re-signing rewrites the file. A build that relinked nothing must leave it untouched, or
# `service-runner.sh refresh` reads the new mtime as a new binary and restarts the service.
already_signed() {
  local details leaf dir
  details="$(codesign -dv "$bin" 2>&1)" || return 1
  printf '%s\n' "$details" | grep -qx "Identifier=$id" || return 1
  printf '%s\n' "$details" | grep -q 'flags=.*(runtime)' || return 1
  codesign --verify --strict "$bin" 2>/dev/null || return 1
  dir="$(mktemp -d)"
  (abs="$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")" && cd "$dir" \
    && codesign -d --extract-certificates "$abs" 2>/dev/null)
  leaf="$(shasum -a 1 "$dir/codesign0" 2>/dev/null | cut -c1-40 | tr 'a-f' 'A-F')"
  rm -rf "$dir"
  [ -n "$leaf" ] && [ "$leaf" = "$(printf '%s' "$identity" | tr 'a-f' 'A-F')" ]
}
already_signed && exit 0
codesign --force --sign "$identity" --identifier "$id" --options runtime "$bin"
codesign --verify --strict "$bin"
