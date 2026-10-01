#!/bin/bash
# Human-run provisioning for the shared inbound API token (ISA ISC-45).
#
# The durable copy is in the operator's login Keychain, item `sjel-inbound-token`, the store
# the principal chose on 2026-10-01 for secrets set up per user. The capability servers read a
# mode-0600 runtime file, because a launchd service cannot prompt for a Keychain unlock. The
# overlay's deployment.env holds only that file's path. Never run this from an agent session.
#
# On a host with no `security` command (Linux), the runtime file is the only copy, and the
# script says so before it writes anything.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
source "$TOOLS_DIR/lib/paths.sh"

command -v openssl >/dev/null 2>&1 || { echo "setup-inbound-auth: openssl is required" >&2; exit 1; }

KEYCHAIN_SERVICE="sjel-inbound-token"
ACCOUNT="${USER:-sjel}"
HAS_KEYCHAIN=0
command -v security >/dev/null 2>&1 && HAS_KEYCHAIN=1

SECRET_FILE="$SJEL_PERSONAL_ROOT/secrets/inbound-token"
DEPLOYMENT_FILE="$SJEL_PERSONAL_ROOT/config/deployment.env"
POINTER="$SJEL_PERSONAL_ROOT/secrets/deployment-inbound-token.md"
TOKEN_TMP=""

cleanup() {
  [ -n "$TOKEN_TMP" ] && rm -f "$TOKEN_TMP" "$TOKEN_TMP.verify"
  return 0
}
trap cleanup EXIT HUP INT TERM

keychain_read() {  # prints the stored value, or nothing
  security find-generic-password -a "$ACCOUNT" -s "$KEYCHAIN_SERVICE" -w 2>/dev/null || true
}

EXISTING=0
if [ "$HAS_KEYCHAIN" = 1 ] && [ -n "$(keychain_read)" ]; then EXISTING=1; fi

CHOICE="R"
if [ "$EXISTING" = 1 ]; then
  echo "Keychain item '$KEYCHAIN_SERVICE' already exists."
  read -r -p "[K]eep its value and rewrite the runtime file / [R]otate with a new random value / [C]ancel: " CHOICE
  CHOICE="${CHOICE:0:1}"
  case "$CHOICE" in
    [Kk]) CHOICE="K" ;;
    [Rr]) CHOICE="R" ;;
    *) echo "cancelled, nothing written"; exit 0 ;;
  esac
fi

echo "About to provision the deployment-wide inbound token:"
if [ "$CHOICE" = "K" ]; then
  echo "  - copy the Keychain value to $SECRET_FILE (mode 600)"
elif [ "$HAS_KEYCHAIN" = 1 ]; then
  echo "  - generate a random value, store it in the login Keychain as '$KEYCHAIN_SERVICE'"
  echo "    and in $SECRET_FILE (mode 600)"
else
  echo "  - generate a random value into $SECRET_FILE (mode 600)"
  echo "    This host has no Keychain, so that file is the only copy."
fi
echo "  - set SJEL_INBOUND_TOKEN_FILE in $DEPLOYMENT_FILE to the file path"
echo "  - write $POINTER with a reference only"
read -r -p "Continue? [y/N]: " CONFIRM
case "$CONFIRM" in [Yy]*) ;; *) echo "cancelled, nothing written"; exit 0 ;; esac

mkdir -p "$SJEL_PERSONAL_ROOT/secrets" "$(dirname "$DEPLOYMENT_FILE")"
chmod 700 "$SJEL_PERSONAL_ROOT/secrets"
TOKEN_TMP="$SJEL_PERSONAL_ROOT/secrets/.inbound-token.tmp.$$"

if [ "$CHOICE" = "K" ]; then
  (umask 077; keychain_read > "$TOKEN_TMP")
else
  (umask 077; openssl rand -hex 32 > "$TOKEN_TMP")
  if [ "$HAS_KEYCHAIN" = 1 ]; then
    # Through `security -i` on stdin, so the value is never in an argument list. The same
    # method as `sjel agent enroll` (tools/capability-auth/src/main.rs).
    # `-T` lists who may read the item without a prompt: `security` itself, for the read-back
    # below, and the menu-bar app (apps/mac/install), which trades the token for a browser login.
    TRUSTED="-T /usr/bin/security"
    [ -d "$HOME/Applications/Sjel.app" ] && TRUSTED="$TRUSTED -T $HOME/Applications/Sjel.app"
    # Delete first: `-U` on an existing item replaces the value but keeps the item's old access
    # list, so an item made before Sjel.app was installed never gained the app, and the app
    # prompted for the login password on every read (2026-10-01). If the add fails, the script
    # exits before it touches the runtime file, so the services keep the token they have.
    security delete-generic-password -a "$ACCOUNT" -s "$KEYCHAIN_SERVICE" >/dev/null 2>&1 || true
    printf 'add-generic-password -U -a %s -s %s -l "Sjel inbound token" %s -w %s\n' \
      "$ACCOUNT" "$KEYCHAIN_SERVICE" "$TRUSTED" "$(cat "$TOKEN_TMP")" | security -i >/dev/null
    # Read it back without printing it. A mismatch changes no runtime configuration.
    (umask 077; keychain_read > "$TOKEN_TMP.verify")
    if ! cmp -s "$TOKEN_TMP" "$TOKEN_TMP.verify"; then
      echo "setup-inbound-auth: the Keychain read-back did not match; runtime config was not changed" >&2
      exit 1
    fi
  fi
fi
[ -s "$TOKEN_TMP" ] || { echo "setup-inbound-auth: the token is empty; nothing written" >&2; exit 1; }

chmod 600 "$TOKEN_TMP"
mv "$TOKEN_TMP" "$SECRET_FILE"
TOKEN_TMP=""
[ -f "$DEPLOYMENT_FILE" ] || touch "$DEPLOYMENT_FILE"
ENV_TMP="$DEPLOYMENT_FILE.tmp.$$"
grep -v '^SJEL_INBOUND_TOKEN_FILE=' "$DEPLOYMENT_FILE" > "$ENV_TMP" || true
printf 'SJEL_INBOUND_TOKEN_FILE=%s\n' "$SECRET_FILE" >> "$ENV_TMP"
mv "$ENV_TMP" "$DEPLOYMENT_FILE"

if [ "$HAS_KEYCHAIN" = 1 ]; then
  WHERE="the login Keychain, item \`$KEYCHAIN_SERVICE\`, and the runtime copy at \`$SECRET_FILE\`"
else
  WHERE="the runtime file \`$SECRET_FILE\` only (this host has no Keychain)"
fi
cat > "$POINTER" <<EOF
# Deployment inbound token

Reference only. The value is in $WHERE. The capability servers read the runtime copy. Do not copy
the value into notes, source control, or an agent session.

- **Declared as:** \`config/deployment.env\` → \`SJEL_INBOUND_TOKEN_FILE\`
- **Provisioned by:** \`tools/setup-inbound-auth.sh\` (run interactively by the operator)
EOF
chmod 600 "$SECRET_FILE" "$POINTER"
echo "done — inbound auth is provisioned; the token value was not printed."
echo "Restart the capability servers and the dashboard shell to load it."
