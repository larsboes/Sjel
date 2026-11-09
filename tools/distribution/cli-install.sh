#!/bin/bash
# Bootstrap and lifecycle for the prebuilt CLI; no developer runtime is needed.
set -euo pipefail

usage() {
  printf '%s\n' \
    'Usage: bash cli-install.sh [install|update|check|status|rollback] [options]' \
    '  --version vX.Y.Z       Select an official release (default: latest)' \
    '  --archive FILE --sha256 DIGEST  Install a local archive with a trusted digest' \
    '  --prefix DIR           Installation root (default: XDG data directory/sjel)' \
    '  --bin-dir DIR          Command directory (default: ~/.local/bin)' \
    '  --help                 Show help without changing anything' \
    'No services, credentials or harness settings are created.'
}
fail() { printf 'sjel install: %s\n' "$*" >&2; exit 1; }
valid_version() { [[ "$1" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; }
valid_digest() { [[ "$1" =~ ^[0-9a-f]{64}$ ]]; }

ACTION=install
case "${1:-}" in install|update|check|status|rollback) ACTION="$1"; shift ;; esac
PREFIX="${XDG_DATA_HOME:-$HOME/.local/share}/sjel"
BIN_DIR="$HOME/.local/bin"
VERSION=""; ARCHIVE=""; DIGEST=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --version|--archive|--sha256|--prefix|--bin-dir)
      [ "$#" -ge 2 ] && [ -n "$2" ] || fail "missing value for $1"
      case "$1" in
        --version) VERSION="$2" ;;
        --archive) ARCHIVE="$2" ;;
        --sha256) DIGEST="$2" ;;
        --prefix) PREFIX="$2" ;;
        --bin-dir) BIN_DIR="$2" ;;
      esac
      shift 2 ;;
    *) fail "unknown argument: $1" ;;
  esac
done
[ -z "$VERSION" ] || valid_version "$VERSION" || fail 'version must be vMAJOR.MINOR.PATCH'
[ -z "$DIGEST" ] || valid_digest "$DIGEST" || fail 'digest must be 64 lowercase hexadecimal characters'
case "$PREFIX" in /*) ;; *) fail '--prefix must be absolute' ;; esac
case "$BIN_DIR" in /*) ;; *) fail '--bin-dir must be absolute' ;; esac
PREFIX="${PREFIX%/}"; BIN_DIR="${BIN_DIR%/}"
[ -n "$PREFIX" ] && [ -n "$BIN_DIR" ] || fail 'installation directories cannot be /'
case "$PREFIX$BIN_DIR" in *$'\n'*|*$'\r'*) fail 'installation paths cannot contain newlines' ;; esac
[ ! -L "$PREFIX" ] || fail 'installation root must not be a symlink'
[ ! -L "$PREFIX/versions" ] || fail 'versions directory must not be a symlink'
if [ -n "$ARCHIVE" ] || [ -n "$DIGEST" ]; then
  [ -n "$ARCHIVE" ] && [ -n "$DIGEST" ] || fail '--archive and --sha256 must be supplied together'
  case "$ACTION" in install|update) ;; *) fail 'local archives are only accepted for install or update' ;; esac
fi

OS="$(uname -s)"
case "$OS" in Linux) SUFFIX=unknown-linux-gnu ;; Darwin) SUFFIX=apple-darwin ;; *) fail "unsupported OS: $OS" ;; esac
case "$(uname -m)" in x86_64|amd64) ARCH=x86_64 ;; arm64|aarch64) ARCH=aarch64 ;; *) fail 'unsupported architecture' ;; esac
PLATFORM="$ARCH-$SUFFIX"
RELEASES=https://github.com/larsboes/Sjel/releases
OWNERSHIP='sjel-cli-direct-install-v1'

owned() {
  [ -f "$PREFIX/.sjel-cli-owned" ] && [ ! -L "$PREFIX/.sjel-cli-owned" ] &&
    [ "$(< "$PREFIX/.sjel-cli-owned")" = "$OWNERSHIP" ]
}
version_at() {
  local name="$1" target
  [ -L "$PREFIX/$name" ] || return 1
  target="$(readlink "$PREFIX/$name")"
  case "$target" in versions/v*) ;; *) fail "invalid $name pointer" ;; esac
  valid_version "${target#versions/}" || fail "invalid $name version"
  [ -d "$PREFIX/$target" ] && [ ! -L "$PREFIX/$target" ] || fail "missing $name release"
  printf '%s\n' "${target#versions/}"
}
if [ "$ACTION" = status ]; then
  owned || fail 'not a direct CLI installation'
  printf 'installed: %s\nplatform: %s\nroot: %s\n' "$(version_at current)" "$PLATFORM" "$PREFIX"
  if [ -L "$PREFIX/previous" ]; then printf 'previous: %s\n' "$(version_at previous)"; fi
  exit 0
fi

wget_fetch() (
  local url="$1" output="$2" limit="$3" headers code hop
  headers="$(mktemp)" || return 1
  trap 'rm -f "$headers"' EXIT
  # Wget's --https-only applies to recursive links, not redirects.
  for ((hop=0; hop<10; hop++)); do
    case "$url" in https://*) ;; *) fail 'download redirect must use HTTPS' ;; esac
    if wget --no-config --quiet --server-response --max-redirect=0 --timeout=30 --tries=1 \
      --output-document=- "$url" 2>"$headers" | head -c "$((limit + 1))" > "$output"; then
      [ "$(wc -c < "$output" | tr -d ' ')" -le "$limit" ] || return 1
      return 0
    fi
    code="$(awk '$1 ~ /^HTTP\// {code=$2} END {print code}' "$headers")"
    case "$code" in 301|302|303|307|308) ;; *) return 1 ;; esac
    url="$(awk 'tolower($1) == "location:" {value=$2} END {sub(/\r$/, "", value); print value}' "$headers")"
  done
  fail 'too many download redirects'
)
fetch() {
  local limit="${3:-268435456}"
  case "$1" in https://*) ;; *) fail 'downloads must use HTTPS' ;; esac
  if command -v curl >/dev/null 2>&1; then
    curl --disable --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
      --connect-timeout 15 --max-time 300 --max-filesize "$limit" "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then
    wget_fetch "$1" "$2" "$limit"
  else
    fail 'curl or wget is required for downloads'
  fi || fail 'download failed; check connectivity and that the release is published for this platform'
}
latest_version() (
  local value temporary
  temporary="$(mktemp)" || fail 'cannot allocate update-check temporary file'
  trap 'rm -f "$temporary"' EXIT
  fetch "$RELEASES/latest/download/CLI_VERSION" "$temporary" 64 || fail 'cannot discover the latest CLI release'
  value="$(< "$temporary")"
  valid_version "$value" || fail 'official release has no valid CLI_VERSION asset'
  printf '%s\n' "$value"
)
if [ "$ACTION" = check ]; then
  owned || fail 'not a direct CLI installation'
  [ -n "$VERSION" ] || VERSION="$(latest_version)"
  printf 'installed: %s\navailable: %s\n' "$(version_at current)" "$VERSION"
  if [ "$(version_at current)" != "$VERSION" ]; then printf '%s\n' 'Run sjel self update to install. Nothing was changed.'; fi
  exit 0
fi

for tool in awk chmod cp dirname find grep gzip head ln ls mkdir mktemp mv readlink rm rmdir sort tar tr uname uniq wc; do
  command -v "$tool" >/dev/null 2>&1 || fail "required command is missing: $tool"
done
command -v sha256sum >/dev/null 2>&1 || command -v shasum >/dev/null 2>&1 || fail 'sha256sum or shasum is required'
if [ "$ACTION" != rollback ] && [ -z "$ARCHIVE" ]; then
  command -v curl >/dev/null 2>&1 || command -v wget >/dev/null 2>&1 || fail 'curl or wget is required for downloads'
fi

# mkdir is the portable exclusive lock; stale locks require explicit inspection.
if [ -e "$PREFIX" ] && ! owned; then
  [ -d "$PREFIX" ] && [ -z "$(ls -A "$PREFIX")" ] || fail 'refusing an existing, unowned installation root'
fi
if [ "$ACTION" != install ]; then owned || fail 'not a direct CLI installation'; fi
mkdir -p "$PREFIX"
PREFIX="$(cd -P "$PREFIX" && pwd)"
mkdir "$PREFIX/.install-lock" 2>/dev/null || fail "another installation holds $PREFIX/.install-lock; inspect it before removing a stale lock"
STAGE=""; LINK=""
cleanup() {
  [ -z "$LINK" ] || rm -f "$LINK"
  [ -z "$STAGE" ] || rm -rf "$STAGE"
  rmdir "$PREFIX/.install-lock"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
STAGE="$(mktemp -d "$PREFIX/.staging.XXXXXX")"
printf '%s\n' "$$" > "$STAGE/pid"

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | awk '{print $1}'
  else fail 'sha256sum or shasum is required'; fi
}
safe_relative() {
  [[ "$1" =~ ^[a-zA-Z0-9_.@+/-]+$ ]] &&
    case "/$1/" in *'/../'*|*'/./'*|*'//'*) return 1 ;; *) return 0 ;; esac
}
verify_payload() {
  local dir="$1" sum path count=0 actual
  [ -f "$dir/PAYLOAD_SHA256SUMS" ] && [ ! -L "$dir/PAYLOAD_SHA256SUMS" ] || fail 'payload integrity manifest is missing'
  [ -z "$(find "$dir" ! -type d ! -type f -print -quit)" ] || fail 'payload contains a symlink or special file'
  : > "$STAGE/verified-paths"
  while IFS=' ' read -r sum path; do
    if ! valid_digest "$sum" || ! safe_relative "$path"; then fail 'invalid payload checksum entry'; fi
    [ "$path" != PAYLOAD_SHA256SUMS ] || fail 'recursive checksum entry'
    [ -f "$dir/$path" ] || fail "payload file missing: $path"
    printf '%s\n' "$path" >> "$STAGE/verified-paths"
    count=$((count + 1))
  done < "$dir/PAYLOAD_SHA256SUMS"
  [ "$(sort "$STAGE/verified-paths" | uniq | wc -l | tr -d ' ')" = "$count" ] || fail 'duplicate payload checksum entry'
  actual="$(find "$dir" -type f | wc -l | tr -d ' ')"
  [ "$actual" = "$((count + 1))" ] || fail 'payload contains unrecorded files'
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$dir" && sha256sum --check --status PAYLOAD_SHA256SUMS) || fail 'payload changed; preserve edits before updating'
  elif command -v shasum >/dev/null 2>&1; then
    (cd "$dir" && shasum -a 256 --check --status PAYLOAD_SHA256SUMS) || fail 'payload changed; preserve edits before updating'
  else fail 'sha256sum or shasum is required'; fi
}
validate_release() {
  local dir="$1" expected="$2"
  verify_payload "$dir"
  [ "$(< "$dir/CLI_VERSION")" = "$expected" ] || fail 'payload version does not match the requested release'
  [ "$(< "$dir/CLI_PLATFORM")" = "$PLATFORM" ] || fail "payload is not built for $PLATFORM"
  [ -x "$dir/target/release/sjel-cli" ] && [ -f "$dir/distribution.json" ] || fail 'incomplete CLI payload'
  [ -f "$dir/tools/distribution/cli-management.sh" ] && [ -f "$dir/tools/distribution/cli-install.sh" ] || fail 'CLI lifecycle helpers are missing'
}
atomic_link() {
  local target="$1" name="$2"
  ln -s "$target" "$STAGE/link"
  if [ "$OS" = Linux ]; then mv -fT "$STAGE/link" "$name"; else mv -fh "$STAGE/link" "$name"; fi
}
CURRENT=""
if owned; then
  CURRENT="$(version_at current)"
  if [ "$ACTION" != rollback ]; then validate_release "$PREFIX/versions/$CURRENT" "$CURRENT"; fi
fi
if [ "$ACTION" = rollback ]; then
  PREVIOUS="$(version_at previous)" || fail 'no previous release is retained'
  validate_release "$PREFIX/versions/$PREVIOUS" "$PREVIOUS"
  SJEL_ROOT="$PREFIX/versions/$PREVIOUS" "$PREFIX/versions/$PREVIOUS/target/release/sjel-cli" --help >/dev/null || fail 'previous CLI cannot run on this host'
  atomic_link "versions/$PREVIOUS" "$PREFIX/current"
  atomic_link "versions/$CURRENT" "$PREFIX/previous"
  printf 'Recovered %s. Harness copies and user data were not changed.\n' "$PREVIOUS"
  exit 0
fi

if [ -z "$ARCHIVE" ]; then
  [ -n "$VERSION" ] || VERSION="$(latest_version)"
  if [ "$CURRENT" = "$VERSION" ]; then printf 'Already installed: %s\n' "$VERSION"; exit 0; fi
  ASSET="sjel-cli-$VERSION-$PLATFORM.tar.gz"
  printf 'Downloading Sjel %s (%s)...\n' "$VERSION" "$PLATFORM"
  fetch "$RELEASES/download/$VERSION/SHA256SUMS" "$STAGE/SHA256SUMS"
  DIGEST="$(awk -v asset="$ASSET" '$2 == asset {print $1}' "$STAGE/SHA256SUMS")"
  valid_digest "$DIGEST" || fail 'official release has no unique checksum for this platform'
  ARCHIVE="$STAGE/$ASSET"
  fetch "$RELEASES/download/$VERSION/$ASSET" "$ARCHIVE"
fi
[ -f "$ARCHIVE" ] || fail 'archive does not exist'
[ "$(hash_file "$ARCHIVE")" = "$DIGEST" ] || fail 'archive SHA-256 does not match the trusted digest'
[ "$(wc -c < "$ARCHIVE" | tr -d ' ')" -le 268435456 ] || fail 'archive exceeds 256 MiB'
# Bound expanded bytes before extraction. A broken gzip pipeline is a refusal.
gzip -dc "$ARCHIVE" | head -c 536870913 > "$STAGE/expanded.tar" || fail 'archive is corrupt or exceeds the expanded-size limit'
[ "$(wc -c < "$STAGE/expanded.tar" | tr -d ' ')" -le 536870912 ] || fail 'expanded archive exceeds 512 MiB'
tar -tf "$STAGE/expanded.tar" > "$STAGE/members" || fail 'cannot list archive'
while IFS= read -r member; do
  member="${member%/}"
  case "$member" in sjel|sjel/*) ;; *) fail 'archive entry is outside sjel/' ;; esac
  safe_relative "$member" || fail "unsafe archive path: $member"
done < "$STAGE/members"
[ -s "$STAGE/members" ] || fail 'archive is empty'
tar -tvf "$STAGE/expanded.tar" > "$STAGE/types" || fail 'cannot inspect archive'
if grep -qv '^[-d]' "$STAGE/types"; then fail 'archive contains links or special files'; fi
mkdir "$STAGE/unpacked"
tar -xf "$STAGE/expanded.tar" -C "$STAGE/unpacked" --no-same-owner --no-same-permissions
PAYLOAD="$STAGE/unpacked/sjel"
[ -f "$PAYLOAD/CLI_VERSION" ] || fail 'CLI_VERSION is missing'
PAYLOAD_VERSION="$(< "$PAYLOAD/CLI_VERSION")"
valid_version "$PAYLOAD_VERSION" || fail 'invalid payload version'
[ -n "$VERSION" ] || VERSION="$PAYLOAD_VERSION"
validate_release "$PAYLOAD" "$VERSION"
[ "$CURRENT" != "$VERSION" ] || fail 'this version is already installed; release versions are immutable'
[ ! -e "$PREFIX/versions/$VERSION" ] && [ ! -L "$PREFIX/versions/$VERSION" ] || fail 'release directory already exists; use rollback for a retained version'
SJEL_ROOT="$PAYLOAD" "$PAYLOAD/target/release/sjel-cli" --help >/dev/null || fail 'CLI cannot run on this host; check the release OS and libc requirements'
mkdir -p "$BIN_DIR"
if [ -e "$BIN_DIR/sjel" ] || [ -L "$BIN_DIR/sjel" ]; then
  [ -L "$BIN_DIR/sjel" ] && [ "$(readlink "$BIN_DIR/sjel")" = "$PREFIX/sjel" ] || fail "refusing to replace $BIN_DIR/sjel owned by another installation"
fi
if [ -e "$PREFIX/sjel" ] && ! owned; then fail 'refusing to overwrite an unowned launcher'; fi
# Prepare command publication before activation; never switch current on a link failure.
if [ ! -L "$BIN_DIR/sjel" ]; then
  CANDIDATE_LINK="$BIN_DIR/.sjel-link-$$"
  [ ! -e "$CANDIDATE_LINK" ] && [ ! -L "$CANDIDATE_LINK" ] || fail 'temporary command link already exists'
  ln -s "$PREFIX/sjel" "$CANDIDATE_LINK"
  LINK="$CANDIDATE_LINK"
fi
mkdir -p "$PREFIX/versions"
# The stable dispatcher is installed once; all mutable lifecycle logic lives in each release.
if [ ! -e "$PREFIX/sjel" ]; then
  cp "$PAYLOAD/tools/distribution/cli-management.sh" "$PREFIX/sjel"
  chmod 755 "$PREFIX/sjel"
fi
mv "$PAYLOAD" "$PREFIX/versions/$VERSION"
printf '%s\n' "$OWNERSHIP" > "$PREFIX/.sjel-cli-owned"
printf '%s\n' "$BIN_DIR" > "$PREFIX/command-dir"
if [ -n "$CURRENT" ]; then atomic_link "versions/$CURRENT" "$PREFIX/previous"; fi
if [ -n "$LINK" ]; then
  if [ "$OS" = Linux ]; then mv -fT "$LINK" "$BIN_DIR/sjel"; else mv -fh "$LINK" "$BIN_DIR/sjel"; fi
  LINK=""
fi
atomic_link "versions/$VERSION" "$PREFIX/current"
printf 'Installed %s (%s).\nCommand: %s/sjel\n' "$VERSION" "$PLATFORM" "$BIN_DIR"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) printf 'For this shell: export PATH=%q:$%s\n' "$BIN_DIR" PATH
     printf '%s\n' 'For future shells, add that directory to the shell configuration.' ;;
esac
printf '%s\n' 'No services, credentials or harness settings were changed.' \
  'Run sjel harnesses list, then explicitly select a harness/profile.' \
  'After an update, inspect sjel harnesses status before synchronizing materialized copies.'
