#!/bin/bash
# Tests for tools/capability-auth — the header `sjel capability call` sends.
#
# Before this tool existed the call sent no credential, so every gated route answered 403 to
# an agent session. The cases here hold the three ways that can regress: the header is wrong,
# the value leaks through the check, or a deployment with no token gets a malformed header.
#
# Uses a throwaway overlay and a synthetic token. It never reads the real overlay, which is
# why it runs the binary directly rather than through the launcher (the launcher resolves
# SJEL_PERSONAL_ROOT itself, from the machine's own manifest).
set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
bin="${CARGO_TARGET_DIR:-$root/target}/release/sjel-capability-auth"
[ -x "$bin" ] || cargo build --locked --release -p sjel-capability-auth --manifest-path "$root/Cargo.toml" >&2 || exit 2

fails=0
is() { # is <description> <expected> <actual>
  if [ "$2" != "$3" ]; then
    echo "FAIL: $1"
    echo "    expected: '$2'"
    echo "    got:      '$3'"
    fails=$((fails + 1))
  fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/declared/config" "$tmp/undeclared/config"
printf 'synthetic-test-token\n' >"$tmp/token"
printf 'SJEL_INBOUND_TOKEN_FILE=%s\n' "$tmp/token" >"$tmp/declared/config/deployment.env"
: >"$tmp/undeclared/config/deployment.env"

run() { # run <overlay> [arg...]
  local overlay="$1"
  shift
  SJEL_PERSONAL_ROOT="$tmp/$overlay" "$bin" "$@"
}

# --- a declared token -----------------------------------------------------------------

is "the header is a Bearer line for the declared token" \
  "Authorization: Bearer synthetic-test-token" "$(run declared)"
is "--check reports configured" "configured" "$(run declared --check)"
is "--check does not print the token" "0" \
  "$(run declared --check | grep -c synthetic-test-token)"

# --- no token declared: the loopback-only deployment -----------------------------------

run undeclared >/dev/null 2>&1
is "no token exits 3, not an empty header line" "3" "$?"
is "--check reports absent" "absent" "$(run undeclared --check)"

# --- a capability with its own token: comms' api_secret_file ---------------------------

printf 'synthetic-comms-token\n' >"$tmp/comms-token"
printf '{"api_secret_file":"%s"}\n' "$tmp/comms-token" >"$tmp/declared/config/comms.json"
is "comms gets its own token over the deployment's" \
  "Authorization: Bearer synthetic-comms-token" "$(run declared comms)"
is "another capability still gets the deployment token" \
  "Authorization: Bearer synthetic-test-token" "$(run declared finance)"
printf '{"api_secret_file":"%s"}\n' "$tmp/comms-token" >"$tmp/undeclared/config/comms.json"
is "comms needs no deployment token" "configured" "$(run undeclared --check comms)"
is "--check comms does not print the token" "0" \
  "$(run undeclared --check comms | grep -c synthetic-comms-token)"

# --- misuse -----------------------------------------------------------------------------

run declared --bogus >/dev/null 2>&1
is "an unknown flag exits 2" "2" "$?"
run declared comms finance >/dev/null 2>&1
is "two capability names exit 2" "2" "$?"

if [ "$fails" -ne 0 ]; then
  echo "$fails failure(s)"
  exit 1
fi
echo "capability-auth: all cases passed"
