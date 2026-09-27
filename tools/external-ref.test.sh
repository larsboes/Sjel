#!/bin/bash
# Test for tools/lib/external-ref.sh, the one resolver for a capability this machine
# consumes but does not run (retired-tracker#169).
#
# Everything here runs against a scratch overlay in mktemp with synthetic `.test` hostnames.
# The real overlay is never read: this gate must give the same answer on a machine that runs
# every capability locally as on the one that provoked the feature.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

TEST_ROOT="$(mktemp -d)"
trap 'rm -rf "$TEST_ROOT"' EXIT
OVERLAY="$TEST_ROOT/overlay"
mkdir -p "$OVERLAY/config"

# The resolver reads the overlay through the two variables paths.sh exports. Sourcing paths.sh
# and then pointing them at the fixture is the whole setup — no fake SJEL_ROOT, because the one
# thing this file reads out of the repo is a capability's tracked manifest.
SJEL_ROOT="$ROOT"
export SJEL_ROOT
source "$ROOT/tools/lib/toml.sh"
source "$ROOT/tools/lib/external-ref.sh"
SJEL_OVERLAY_ROOT="$OVERLAY"
SJEL_MACHINE_TOML="$OVERLAY/config/machine.toml"

fail() { echo "FAIL: $1" >&2; exit 1; }

# --- 1. a declared provider resolves through the systems id ---------------------------------
cat > "$SJEL_MACHINE_TOML" <<'EOF'
os = "macos"
container_runtime = "docker"
capabilities = ["comms"]

[capability.vaultwarden]
provided_by = "family-vault"

[capability.comms]
port = "5432"
EOF
cat > "$OVERLAY/config/systems.local.toml" <<'EOF'
[family-vault]
url = "https://vault.provider.test/"
EOF

got="$(capability_endpoint vaultwarden DOMAIN)" || fail "a declared provider must resolve"
# Trailing slash trimmed at the resolver, so no caller has to decide whether to add one before
# appending a health path — two callers deciding differently is a URL with a double slash in it.
[ "$got" = "https://vault.provider.test" ] || fail "expected the provider url, got '$got'"

# --- 2. the provider wins over the capability's own server config ---------------------------
# The ordering that shipped in materialize-inference-key and must not invert on the way into a
# shared resolver: an explicit operator declaration beats a fact inferred from a server's config.
printf 'DOMAIN=https://this-host.test\n' > "$OVERLAY/config/vaultwarden.env"
got="$(capability_endpoint vaultwarden DOMAIN)" || fail "resolution must still succeed"
[ "$got" = "https://vault.provider.test" ] || fail "env_file DOMAIN overrode an explicit declaration"

# --- 3. a machine that self-hosts is unchanged ----------------------------------------------
cat > "$SJEL_MACHINE_TOML" <<'EOF'
os = "linux"
container_runtime = "docker"
capabilities = ["vaultwarden"]
EOF
got="$(capability_endpoint vaultwarden DOMAIN)" || fail "the env_file fallback must still work"
[ "$got" = "https://this-host.test" ] || fail "expected the env_file DOMAIN, got '$got'"

# --- 4. nothing declared is a distinct answer from a broken declaration ----------------------
# Exit 1, silent: the caller knows what the address is FOR and writes the better message.
rm -f "$OVERLAY/config/vaultwarden.env"
set +e
out="$(capability_endpoint vaultwarden DOMAIN 2>"$TEST_ROOT/err")"; rc=$?
set -e
[ "$rc" -eq 1 ] || fail "an undeclared capability must exit 1, got $rc"
[ -z "$out" ] || fail "an undeclared capability must print nothing, got '$out'"
[ ! -s "$TEST_ROOT/err" ] || fail "an undeclared capability must not report; the caller does"

# --- 5. a dangling reference fails loudly and never falls back ------------------------------
# The one that matters most. A provided_by naming an id with no url means the operator said
# something specific and got it wrong; resolving to a local address anyway would point a vault
# client at whatever answers on this host.
cat > "$SJEL_MACHINE_TOML" <<'EOF'
os = "macos"
container_runtime = "docker"
capabilities = []

[capability.vaultwarden]
provided_by = "no-such-system"
EOF
printf 'DOMAIN=https://this-host.test\n' > "$OVERLAY/config/vaultwarden.env"
set +e
out="$(capability_endpoint vaultwarden DOMAIN 2>"$TEST_ROOT/err")"; rc=$?
set -e
[ "$rc" -eq 2 ] || fail "a dangling reference must exit 2, got $rc"
[ -z "$out" ] || fail "a dangling reference must resolve to nothing, got '$out'"
grep -q "no-such-system" "$TEST_ROOT/err" || fail "the error must name the unresolved id"
grep -q "systems.local.toml" "$TEST_ROOT/err" || fail "the error must name the file to fix"

# --- 6. the external set is exactly the sections that declare a provider --------------------
cat > "$SJEL_MACHINE_TOML" <<'EOF'
os = "macos"
container_runtime = "docker"
capabilities = ["comms"]

[capability.comms]
port = "5432"

[capability.vaultwarden]
provided_by = "family-vault"

[capability.pihole]
provided_by = "family-dns"
EOF
got="$(external_capabilities | sort | tr '\n' ' ')"
[ "$got" = "pihole vaultwarden " ] || fail "expected the two declared providers, got '$got'"

# A per-capability override that is not a reference must not be mistaken for one — [capability.X]
# has meant "where this machine binds it" since long before this feature existed.
external_capabilities | grep -q '^comms$' && fail "a plain override was read as an external reference"

# --- 7. no second implementation ------------------------------------------------------------
# The class this feature exists to close: two tools each resolving a vault address by hand, which
# is how they came to disagree about whether DOMAIN describes the server you run or the one you
# use. Any new reader outside the resolver is the same bug again.
strays="$(cd "$ROOT" && grep -rlE "grep -m1 '\^DOMAIN=|\[vaultwarden\]" tools/ 2>/dev/null \
  | grep -v '^tools/lib/external-ref.sh$' \
  | grep -v '\.test\.sh$' || true)"
[ -z "$strays" ] || fail "these resolve a vault address by hand instead of calling capability_endpoint: $strays"

# --- 8. trusted peers: the declaration is an intersection, not a location -------------------
# PRD Q39 (2026-08-25). Presence in systems.local.toml cannot be the test: nvidia-nim lives in
# the same file and is a cloud provider. `owner = "self"` is the declaration.
cat > "$OVERLAY/config/systems.local.toml" <<'EOF'
[family-vault]
url = "https://vault.example.test"

[lars-mac]
url = "https://lars-mac.example.test:11434"
owner = "self"

[old-intel]
url = "https://intel.example.test:11434"
owner = "self"

[nvidia-nim]
url = "https://integrate.example.test/v1"
EOF

got="$(trusted_peer_ids | sort | tr '\n' ' ')"
[ "$got" = "lars-mac old-intel " ] || fail "expected the two owner=self hosts, got '$got'"

trusted_peer_ids | grep -q '^nvidia-nim$' && fail "a cloud provider in the same file was read as owned hardware"
trusted_peer_ids | grep -q '^family-vault$' && fail "an entry declaring no owner was read as owned hardware"

[ "$(systems_owner lars-mac)" = "self" ] || fail "systems_owner did not read the declaration"
[ -z "$(systems_owner nvidia-nim)" ] || fail "systems_owner invented an owner"

got="$(trusted_peers_env)"
[ "$got" = "lars-mac,old-intel" ] || fail "expected a comma-separated pair, got '$got'"

# The single-host state, which is this machine's today and must produce exactly the behaviour
# that shipped before Q39: no declaration, no peers, empty output, exit 0.
cat > "$OVERLAY/config/systems.local.toml" <<'EOF'
[nvidia-nim]
url = "https://integrate.example.test/v1"
EOF
got="$(trusted_peers_env)"
[ -z "$got" ] || fail "an overlay declaring no owner must trust nothing, got '$got'"

# And a missing file is not an error: an overlay may have no systems map at all.
rm -f "$OVERLAY/config/systems.local.toml"
got="$(trusted_peers_env)" || fail "a missing systems.local.toml must not fail the resolver"
[ -z "$got" ] || fail "a missing systems map produced peers: '$got'"

echo "external-ref tests passed"
