#!/bin/bash
# Human-run provisioning for the shared inbound API token.
#
# The token value is kept in Vaultwarden and in one mode-0600 runtime file. The overlay's
# deployment.env contains only the runtime file path. Never run this from an agent session.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
source "$TOOLS_DIR/lib/paths.sh"
source "$TOOLS_DIR/lib/external-ref.sh"

command -v bw >/dev/null 2>&1 || {
  echo "setup-inbound-auth: install the declared Bitwarden CLI first (upstreams.toml [bitwarden-cli])" >&2
  exit 1
}
command -v jq >/dev/null 2>&1 || { echo "setup-inbound-auth: jq is required" >&2; exit 1; }
command -v openssl >/dev/null 2>&1 || { echo "setup-inbound-auth: openssl is required" >&2; exit 1; }

DOMAIN=""
REF_RC=0
DOMAIN="$(capability_endpoint vaultwarden DOMAIN)" || REF_RC=$?
if [ "$REF_RC" -eq 2 ]; then exit 1; fi
if [ "$REF_RC" -ne 0 ]; then
  echo "setup-inbound-auth: this machine has no Vaultwarden declared as its canonical secret store" >&2
  exit 1
fi

STATUS_JSON="$(bw status 2>/dev/null || true)"
CURRENT_SERVER="$(echo "$STATUS_JSON" | jq -r '.serverUrl // empty')"
[ "$CURRENT_SERVER" = "$DOMAIN" ] || bw config server "$DOMAIN" >/dev/null

VAULT_STATUS="$(bw status 2>/dev/null | jq -r '.status // "unauthenticated"')"
LOCKED_BY_US=0
case "$VAULT_STATUS" in
  unauthenticated)
    echo "setup-inbound-auth: not logged in to $DOMAIN; run 'bw login' yourself, then retry" >&2
    exit 1
    ;;
  locked)
    echo "Vault is locked. Enter the master password to unlock it for this run only:"
    BW_SESSION="$(bw unlock --raw)"
    export BW_SESSION
    LOCKED_BY_US=1
    ;;
  unlocked) ;;
  *) echo "setup-inbound-auth: unexpected Vaultwarden status" >&2; exit 1 ;;
esac

cleanup() {
  [ "$LOCKED_BY_US" = 1 ] && bw lock >/dev/null 2>&1 || true
  [ -n "${TOKEN_TMP:-}" ] && rm -f "$TOKEN_TMP"
  unset BW_SESSION
}
trap cleanup EXIT HUP INT TERM

bw sync --session "$BW_SESSION" >/dev/null
FOLDER_ID="$(bw list folders --session "$BW_SESSION" --nointeraction \
  | jq -r '.[] | select(.name=="Axon") | .id' | head -1)"
if [ -z "$FOLDER_ID" ]; then
  FOLDER_ID="$(bw get template folder --session "$BW_SESSION" --nointeraction \
    | jq '.name="Axon"' | bw encode \
    | bw create folder --session "$BW_SESSION" --nointeraction | jq -r .id)"
fi
ITEM_NAME="deployment-inbound-token"
ITEM_ID="$(bw list items --folderid "$FOLDER_ID" --session "$BW_SESSION" --nointeraction \
  | jq -r --arg n "$ITEM_NAME" '.[] | select(.name==$n) | .id' | head -1)"

CHOICE="G"
if [ -n "$ITEM_ID" ]; then
  echo "Vault item '$ITEM_NAME' already exists."
  read -r -p "[K]eep its value and re-sync / [R]otate with a new random value / [C]ancel: " CHOICE
  case "${CHOICE:0:1}" in
    [Kk]|[Rr]|[Cc]) ;;
    *) echo "cancelled, nothing written"; exit 0 ;;
  esac
  CHOICE="${CHOICE:0:1}"
else
  echo "A new random token will be generated and stored in '$ITEM_NAME' (Axon folder)."
fi
case "$CHOICE" in
  [Cc]|[cC]) echo "cancelled, nothing written"; exit 0 ;;
esac

SECRET_FILE="$SJEL_PERSONAL_ROOT/secrets/inbound-token"
DEPLOYMENT_FILE="$SJEL_PERSONAL_ROOT/config/deployment.env"
POINTER="$SJEL_PERSONAL_ROOT/secrets/deployment-inbound-token.md"
echo "About to provision the deployment-wide inbound token:"
if [ "$CHOICE" != "K" ] && [ "$CHOICE" != "k" ]; then
  echo "  - generate a random value, write it to Vaultwarden and $SECRET_FILE (mode 600)"
else
  echo "  - retrieve the existing Vaultwarden value to $SECRET_FILE (mode 600)"
fi
echo "  - set SJEL_INBOUND_TOKEN_FILE in $DEPLOYMENT_FILE to the file path"
echo "  - write/update $POINTER with a reference only"
read -r -p "Continue? [y/N]: " CONFIRM
case "$CONFIRM" in [Yy]*) ;; *) echo "cancelled, nothing written"; exit 0 ;; esac

mkdir -p "$SJEL_PERSONAL_ROOT/secrets" "$(dirname "$DEPLOYMENT_FILE")"
chmod 700 "$SJEL_PERSONAL_ROOT/secrets"
TOKEN_TMP="$SJEL_PERSONAL_ROOT/secrets/.inbound-token.tmp.$$"
if [ "$CHOICE" = "K" ] || [ "$CHOICE" = "k" ]; then
  bw get notes "$ITEM_ID" --session "$BW_SESSION" --nointeraction > "$TOKEN_TMP"
  chmod 600 "$TOKEN_TMP"
  [ -s "$TOKEN_TMP" ] || { echo "setup-inbound-auth: Vaultwarden item is empty; nothing written" >&2; exit 1; }
else
  (umask 077; openssl rand -hex 32 > "$TOKEN_TMP")
  chmod 600 "$TOKEN_TMP"
  if [ -n "$ITEM_ID" ]; then
    ENCODED="$(bw get item "$ITEM_ID" --session "$BW_SESSION" --nointeraction \
      | jq --rawfile notes "$TOKEN_TMP" '.notes=$notes' | bw encode)"
    bw edit item "$ITEM_ID" "$ENCODED" --session "$BW_SESSION" --nointeraction >/dev/null
  else
    ENCODED="$(bw get template item --session "$BW_SESSION" --nointeraction \
      | jq --arg name "$ITEM_NAME" --rawfile notes "$TOKEN_TMP" --arg fid "$FOLDER_ID" \
        '.type=2 | .name=$name | .notes=$notes | .folderId=$fid | .secureNote={type:0} | del(.login,.card,.identity)' \
      | bw encode)"
    ITEM_ID="$(bw create item "$ENCODED" --session "$BW_SESSION" --nointeraction | jq -r .id)"
  fi
fi

# Verify the vault write/read without printing the value. Replace any earlier temp file with
# the canonical Vaultwarden value before writing the runtime copy.
if [ "$CHOICE" != "K" ] && [ "$CHOICE" != "k" ]; then
  VERIFY_TMP="$TOKEN_TMP.verify"
  bw get notes "$ITEM_ID" --session "$BW_SESSION" --nointeraction > "$VERIFY_TMP"
  chmod 600 "$VERIFY_TMP"
  if ! cmp -s "$TOKEN_TMP" "$VERIFY_TMP"; then
    rm -f "$VERIFY_TMP"
    echo "setup-inbound-auth: Vaultwarden read-back did not match; runtime config was not changed" >&2
    exit 1
  fi
  mv "$VERIFY_TMP" "$TOKEN_TMP"
fi

chmod 600 "$TOKEN_TMP"
mv "$TOKEN_TMP" "$SECRET_FILE"
TOKEN_TMP=""
[ -f "$DEPLOYMENT_FILE" ] || touch "$DEPLOYMENT_FILE"
DEPLOYMENT_REF="$SECRET_FILE"
ENV_TMP="$DEPLOYMENT_FILE.tmp.$$"
grep -v '^SJEL_INBOUND_TOKEN_FILE=' "$DEPLOYMENT_FILE" > "$ENV_TMP" || true
printf 'SJEL_INBOUND_TOKEN_FILE=%s\n' "$DEPLOYMENT_REF" >> "$ENV_TMP"
mv "$ENV_TMP" "$DEPLOYMENT_FILE"

cat > "$POINTER" <<EOF
# Deployment inbound token

Reference only. The plaintext value is in Vaultwarden, item \\`$ITEM_NAME\\`, and the runtime
copy at \\`$SECRET_FILE\\` (required by the capability servers). Do not copy the value into notes,
source control, or an agent session.

- **Declared as:** \\`config/deployment.env\\` → \\`SJEL_INBOUND_TOKEN_FILE\\`
- **Provisioned by:** \\`tools/setup-inbound-auth.sh\\` (run interactively by the operator)
EOF
chmod 600 "$SECRET_FILE"
echo "done — inbound auth is provisioned; token value was not printed. Restart the capability servers and dashboard shell to load it."
