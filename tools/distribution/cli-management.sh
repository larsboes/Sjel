#!/bin/bash
# Stable direct-install dispatcher; versioned helpers own installation policy.
set -euo pipefail
SOURCE="${BASH_SOURCE[0]}"
while [ -L "$SOURCE" ]; do
  DIR="$(cd -P "$(dirname "$SOURCE")" && pwd)"
  TARGET="$(readlink "$SOURCE")"
  case "$TARGET" in /*) SOURCE="$TARGET" ;; *) SOURCE="$DIR/$TARGET" ;; esac
done
INSTALL_ROOT="$(cd -P "$(dirname "$SOURCE")" && pwd)"
SJEL_ROOT="$INSTALL_ROOT/current"
[ -f "$INSTALL_ROOT/.sjel-cli-owned" ] && [ -f "$SJEL_ROOT/tools/distribution/cli-install.sh" ] || {
  printf '%s\n' 'sjel: incomplete direct installation; reinstall the verified CLI archive' >&2
  exit 2
}
export SJEL_ROOT
# Keep registry paths stable across version switches and isolate deployment state from source installs.
export SJEL_OVERLAY_ROOT="${SJEL_OVERLAY_ROOT:-$INSTALL_ROOT/overlay}"
if [ "${1:-}" = help ] && [ "${2:-}" = self ]; then
  printf '%s\n' 'Usage: sjel self check | update [--version vX.Y.Z] | status | rollback'
  exit 0
fi
case "${1:-}" in
  help|--help|-h|'')
    if [ "$#" -le 1 ]; then
      printf '%s\n' 'Sjel CLI-only installation. Desktop and node services are not installed.' \
        'Lifecycle: sjel --version; sjel self check; sjel self update; sjel self rollback' ''
    fi
    ;;
  --version|-V) printf '%s\n' "$(< "$SJEL_ROOT/CLI_VERSION")"; exit 0 ;;
  self)
    case "${2:-}" in
      update|check|status|rollback)
        ACTION="$2"; shift 2
        BIN_DIR="$(< "$INSTALL_ROOT/command-dir")"
        exec bash "$SJEL_ROOT/tools/distribution/cli-install.sh" "$ACTION" "$@" --prefix "$INSTALL_ROOT" --bin-dir "$BIN_DIR"
        ;;
      help|--help|-h|'')
        printf '%s\n' 'Usage: sjel self check | update [--version vX.Y.Z] | status | rollback' \
          'Updates require an explicit command. Harness copies and credentials are not changed.'
        exit 0 ;;
    esac
    ;;
esac
[ -x "$SJEL_ROOT/target/release/sjel-cli" ] || {
  printf '%s\n' 'sjel: active CLI is missing; try sjel self rollback' >&2
  exit 2
}
exec "$SJEL_ROOT/target/release/sjel-cli" "$@"
