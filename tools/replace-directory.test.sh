#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
. "$root/tools/lib/replace-directory.sh"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/sjel-replace-test.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

mv() {
  case "${failure:-}" in
    preserve) [ "$1" != "$scratch/current" ] || return 71 ;;
    install|partial-install)
      if [ "$1" = "$scratch/incoming" ] && [ "${partial_install:-}" = yes ]; then
        /bin/mkdir -p "$2/partial"
        printf partial > "$2/partial/version"
        return 72
      fi
      [ "$1" != "$scratch/incoming" ] || return 72
      ;;
  esac
  /bin/mv "$@"
}

for failure in preserve install partial-install none; do
  mkdir -p "$scratch/current" "$scratch/incoming"
  printf old > "$scratch/current/version"
  printf new > "$scratch/incoming/version"
  partial_install=no
  [ "$failure" != partial-install ] || partial_install=yes
  if sjel_replace_directory "$scratch/incoming" "$scratch/current"; then
    [ "$failure" = none ]
    [ "$(cat "$scratch/current/version")" = new ]
    [ ! -e "$scratch/incoming" ]
  else
    [ "$failure" != none ]
    [ "$(cat "$scratch/current/version")" = old ]
    [ "$(cat "$scratch/incoming/version")" = new ]
  fi
  [ ! -e "$scratch/current.previous.$$" ]
  rm -rf "$scratch/current" "$scratch/incoming"
done
echo "replace-directory: 4 checks passed"
