#!/bin/bash
# Signs a built binary with this Mac's code-signing identity and a fixed identifier.
#
# macOS names a binary by its code signature. An ad-hoc signature changes with every build, so
# the application firewall and privacy grants treat each rebuild as a new app: the Same Wi-Fi
# listener (PRD Q119) went dark after the first rebuild on 2026-09-25 until it was allowed again.
# A stable identity and identifier keep the grant across rebuilds. Same identity lookup as
# tools/fda-launcher/install.
#
#   tools/codesign-binary.sh <path> <identifier>
set -euo pipefail
bin="${1:?usage: codesign-binary.sh <path> <identifier>}"
id="${2:?usage: codesign-binary.sh <path> <identifier>}"
[ "$(uname -s)" = Darwin ] || exit 0

identity="${SJEL_CODESIGN_IDENTITY:-}"
if [ -z "$identity" ]; then
  identity="$(security find-identity -v -p codesigning | awk 'NR==1 && $2 ~ /^[0-9A-F]{40}$/ {print $2}')"
fi
if [ -z "$identity" ]; then
  # Not an error: a machine without an identity keeps the ad-hoc signature it had.
  echo "codesign-binary: no code-signing identity; $bin stays ad-hoc signed" >&2
  exit 0
fi
codesign --force --sign "$identity" --identifier "$id" --options runtime "$bin"
codesign --verify --strict "$bin"
