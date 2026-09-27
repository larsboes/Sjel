#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_ROOT="$(mktemp -d)"
trap 'rm -rf "$TEST_ROOT"' EXIT
OVERLAY="$TEST_ROOT/overlay"
FAKE_BIN="$TEST_ROOT/bin"
mkdir -p "$OVERLAY/config" "$FAKE_BIN"
printf 'DOMAIN=https://vault.test\n' > "$OVERLAY/config/vaultwarden.env"

cat > "$FAKE_BIN/bw" <<'EOF'
#!/bin/bash
case "$1 $2" in
  "status ") printf '{"status":"%s","serverUrl":"%s"}\n' "${BW_FAKE_STATUS:-unlocked}" "${BW_FAKE_SERVER:-https://vault.test}" ;;
  "sync --session") ;;
  "list folders") printf '[{"id":"folder-1","name":"Axon"}]\n' ;;
  "list items") printf '[{"id":"item-1","name":"inference-gemini-api-key"}]\n' ;;
  "get item") printf '{"id":"item-1","type":2}\n' ;;
  "get notes") printf 'synthetic-test-key\n' ;;
  *) echo "unexpected bw call: $*" >&2; exit 1 ;;
esac
EOF
chmod +x "$FAKE_BIN/bw"

OUTPUT="$(
  PATH="$FAKE_BIN:$PATH" \
  SJEL_ROOT="$ROOT" \
  SJEL_INFERENCE_KEY_OVERLAY="$OVERLAY" \
  BW_SESSION="synthetic-session" \
  "$ROOT/tools/materialize-inference-key" gemini
)"

TARGET="$OVERLAY/config/runtime-secrets/inference-gemini-api-key"
[ "$(cat "$TARGET")" = "synthetic-test-key" ]
case "$(uname -s)" in
  Darwin) MODE="$(stat -f '%Lp' "$TARGET")" ;;
  *) MODE="$(stat -c '%a' "$TARGET")" ;;
esac
[ "$MODE" = "600" ]
case "$OUTPUT" in
  *synthetic-test-key*) echo "FAIL: command output exposed the key" >&2; exit 1 ;;
esac
grep -q "credential materialized" <<< "$OUTPUT"

MISMATCH_ERROR="$TEST_ROOT/mismatch.err"
if PATH="$FAKE_BIN:$PATH" SJEL_ROOT="$ROOT" SJEL_INFERENCE_KEY_OVERLAY="$OVERLAY" \
  BW_SESSION="synthetic-session" BW_FAKE_SERVER="https://other.test" \
  "$ROOT/tools/materialize-inference-key" gemini >/dev/null 2>"$MISMATCH_ERROR"; then
  echo "FAIL: a logged-in session on another server must stop before config changes" >&2
  exit 1
fi
grep -q "run 'bw logout' yourself" "$MISMATCH_ERROR"

if PATH="$FAKE_BIN:$PATH" SJEL_ROOT="$ROOT" SJEL_INFERENCE_KEY_OVERLAY="$OVERLAY" \
  BW_SESSION="synthetic-session" "$ROOT/tools/materialize-inference-key" unknown \
  >/dev/null 2>&1; then
  echo "FAIL: unknown providers must be rejected" >&2
  exit 1
fi

# --- which Vaultwarden the CLI is checked against -------------------------------------------
#
# The observed failure: a machine that CONSUMES the vault rather than hosting it runs no
# vaultwarden capability, so reading that capability's env_file DOMAIN compared `bw` against a
# server that does not exist locally. The CLI was correctly logged into the vault's real host and
# was told it was "logged into a different server", with a `bw logout` instruction that would have
# made things worse.
#
# The declaration is machine.toml's, resolved through a systems id (retired-tracker#169) — the
# generalized form of the awk lookup this file used to assert against directly. It must win over
# the capability's server config, which stays the fallback for a machine that genuinely self-hosts.
printf '[capability.vaultwarden]\nprovided_by = "family-vault"\n' > "$OVERLAY/config/machine.toml"
printf '[family-vault]\nurl = "https://client-declared.test"\n' > "$OVERLAY/config/systems.local.toml"
# The first case above already materialized this. Left in place, the next run hits the
# "Replace the existing local key file?" prompt, `read` sees EOF under `set -e`, and the script
# dies for a reason that has nothing to do with what is being tested here.
rm -f "$OVERLAY/config/runtime-secrets/inference-gemini-api-key"
if ! PATH="$FAKE_BIN:$PATH" SJEL_ROOT="$ROOT" SJEL_INFERENCE_KEY_OVERLAY="$OVERLAY" \
  BW_SESSION="synthetic-session" BW_FAKE_SERVER="https://client-declared.test" \
  "$ROOT/tools/materialize-inference-key" gemini >/dev/null 2>&1; then
  echo "FAIL: a CLI matching the client-declared vault must be accepted" >&2
  exit 1
fi

# ...and the capability's own DOMAIN must NOT decide it once a client entry exists, or a
# consuming machine is right back where it started.
if PATH="$FAKE_BIN:$PATH" SJEL_ROOT="$ROOT" SJEL_INFERENCE_KEY_OVERLAY="$OVERLAY" \
  BW_SESSION="synthetic-session" BW_FAKE_SERVER="https://vault.test" \
  "$ROOT/tools/materialize-inference-key" gemini >/dev/null 2>&1; then
  echo "FAIL: the capability env DOMAIN must not override an explicit client declaration" >&2
  exit 1
fi

# A dangling declaration — a provider named but not resolvable — must stop here rather than fall
# back to the local DOMAIN. Falling back would point a vault client at whatever answers on this
# host, which is the one outcome worse than refusing.
rm -f "$OVERLAY/config/systems.local.toml" "$OVERLAY/config/runtime-secrets/inference-gemini-api-key"
DANGLING_ERROR="$TEST_ROOT/dangling.err"
if PATH="$FAKE_BIN:$PATH" SJEL_ROOT="$ROOT" SJEL_INFERENCE_KEY_OVERLAY="$OVERLAY" \
  BW_SESSION="synthetic-session" BW_FAKE_SERVER="https://vault.test" \
  "$ROOT/tools/materialize-inference-key" gemini >/dev/null 2>"$DANGLING_ERROR"; then
  echo "FAIL: an unresolvable provider must not fall back to the capability's own DOMAIN" >&2
  exit 1
fi
grep -q "family-vault" "$DANGLING_ERROR"

# A self-hosting machine declares no provider and keeps the old behaviour exactly.
rm -f "$OVERLAY/config/machine.toml" "$OVERLAY/config/runtime-secrets/inference-gemini-api-key"
if ! PATH="$FAKE_BIN:$PATH" SJEL_ROOT="$ROOT" SJEL_INFERENCE_KEY_OVERLAY="$OVERLAY" \
  BW_SESSION="synthetic-session" BW_FAKE_SERVER="https://vault.test" \
  "$ROOT/tools/materialize-inference-key" gemini >/dev/null 2>&1; then
  echo "FAIL: without a client entry the capability DOMAIN must still be used" >&2
  exit 1
fi

echo "materialize-inference-key tests passed"
