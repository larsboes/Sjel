#!/bin/bash
# Rehearse a CLI archive in an isolated home; never use the host's harness settings.
set -euo pipefail
if [ "$#" -ne 2 ] && [ "$#" -ne 4 ]; then
  printf '%s\n' 'Usage: bash tools/distribution/cli-smoke.sh ARCHIVE TRUSTED_SHA256 [UPDATE_ARCHIVE UPDATE_SHA256]' >&2
  exit 2
fi
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/sjel-cli-smoke.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT
export HOME="$SCRATCH/home"
export XDG_DATA_HOME="$HOME/.local/share"
export XDG_CONFIG_HOME="$HOME/.config"
export XDG_STATE_HOME="$HOME/.local/state"
export XDG_CACHE_HOME="$HOME/.cache"
unset SJEL_OVERLAY_ROOT SJEL_PERSONAL_ROOT SJEL_ROOT SJEL_CLI_BIN CARGO_TARGET_DIR
mkdir -p "$HOME" "$SCRATCH/no-build-tools"
for name in cargo rustc git; do
  printf '#!/bin/bash\nprintf "unexpected developer tool: %s\\n" >&2\nexit 99\n' "$name" > "$SCRATCH/no-build-tools/$name"
  chmod 755 "$SCRATCH/no-build-tools/$name"
done
export PATH="$SCRATCH/no-build-tools:$PATH"
ARCHIVE="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
UPDATE=""
if [ "$#" -eq 4 ]; then UPDATE="$(cd "$(dirname "$3")" && pwd)/$(basename "$3")"; fi
bash "$HERE/cli-install.sh" --archive "$ARCHIVE" --sha256 "$2"
cd "$HOME"
CLI="$HOME/.local/bin/sjel"
"$CLI" --version
"$CLI" --help
"$CLI" self status
"$CLI" harnesses list
"$CLI" pack list pi
mkdir -p "$HOME/.pi/agent"
printf '{}\n' > "$HOME/.pi/agent/settings.json"
"$CLI" pack use pi coding-harness
"$CLI" pack status pi
INSTALLED_ROOT="$(cd -P "$XDG_DATA_HOME/sjel" && pwd)"
grep -Fq "$INSTALLED_ROOT/current/Packs" "$HOME/.pi/agent/settings.json"
if [ "$#" -eq 4 ]; then
  ORIGINAL_VERSION="$("$CLI" --version)"
  cp "$HOME/.pi/agent/settings.json" "$SCRATCH/settings-before-update.json"
  "$CLI" self update --archive "$UPDATE" --sha256 "$4"
  [ "$("$CLI" --version)" != "$ORIGINAL_VERSION" ]
  cmp "$HOME/.pi/agent/settings.json" "$SCRATCH/settings-before-update.json"
  "$CLI" harnesses list
  "$CLI" self rollback
  [ "$("$CLI" --version)" = "$ORIGINAL_VERSION" ]
  cmp "$HOME/.pi/agent/settings.json" "$SCRATCH/settings-before-update.json"
fi
[ ! -d "$XDG_DATA_HOME/sjel/overlay" ] || {
  printf '%s\n' 'CLI smoke: installation unexpectedly created a deployment overlay' >&2
  exit 1
}
printf '%s\n' 'CLI smoke passed in an isolated home; no developer tools or node services were used.'
