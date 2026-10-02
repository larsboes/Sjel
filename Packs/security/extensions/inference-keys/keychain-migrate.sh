#!/bin/bash
# keychain-migrate.sh — move the pi provider keys out of Bitwarden and into the
# macOS login keychain, one item at a time.
#
# Why: pi runs `bw-key.mjs <slug>` once per registered provider, inside startup,
# serially. `bw get notes` costs ~2.9 s (a Node CLI reading a self-hosted vault
# over Tailscale) where `security find-generic-password -w` costs ~10 ms. Eight
# providers' worth of that difference is the entire 20-36 s wait before pi's
# first prompt. Measured 2026-09-17.
#
# The value travels vault -> keychain over a pipe. It is never printed, never an
# argv element (so never visible in ps), and never written to a file.
#
# Re-runnable. A slug already in the keychain is skipped, and `security -U` makes
# a repeat write an update rather than a failure, so an interrupted run is safe
# to restart.
#
# Requires an unlocked vault: run `bwu` first.
#
# usage: keychain-migrate.sh [slug[=Vault Item Name] ...]

set -euo pipefail
umask 077

# The reader and writer live beside this script, so the default follows the script's own
# location rather than an install path: this Pack REGISTERS its extensions where they sit
# in the checkout, and nothing copies them to ~/.pi. An explicit PI_INFERENCE_KEYS_DIR
# still wins, for a copy that has been split up.
EXT_DIR="${PI_INFERENCE_KEYS_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
READER="$EXT_DIR/bw-key.mjs"
WRITER="$EXT_DIR/keychain-write.exp"
ACCOUNT="${PI_KEYCHAIN_ACCOUNT:-$(id -un)}"

# `slug` or `slug=Exact Vault Item Name`. The last three carry legacy vault names,
# which is why they are spelled out: the keychain service is derived from the
# slug alone (`inference-<slug>-api-key`), so the new store gets the right name
# even where the old one has the wrong one. openrouter and kimi are duplicated in
# the vault, and the reader resolves that the same way it does at runtime.
DEFAULT_SPECS=(
	groq
	nvidia-nim
	gemini
	cohere
	ollama-cloud
	"deepseek=Deepseek API Key"
	"openrouter=Openrouter API Key"
	"kimi=Kimi K2 API Token"
)

for tool in node security expect; do
	command -v "$tool" >/dev/null 2>&1 || {
		echo "keychain-migrate: $tool is not on PATH" >&2
		exit 1
	}
done
[ -f "$READER" ] || {
	echo "keychain-migrate: no reader at $READER" >&2
	exit 1
}
[ -x "$WRITER" ] || {
	echo "keychain-migrate: no executable writer at $WRITER" >&2
	exit 1
}

# Fail before touching 8 items if the vault cannot be read at all, so a locked
# vault reads as one clear message instead of eight identical failures.
if ! node "$READER" --status 2>/dev/null | grep -q '"sessionUsable": true'; then
	echo "keychain-migrate: no usable vault session. Run \`bwu\` first." >&2
	node "$READER" --status >&2 || true
	exit 3
fi

SPECS=("$@")
if [ ${#SPECS[@]} -eq 0 ]; then
	SPECS=("${DEFAULT_SPECS[@]}")
fi

moved=0
skipped=0
failed=0

for spec in "${SPECS[@]}"; do
	slug="${spec%%=*}"
	service="inference-${slug}-api-key"

	if security find-generic-password -s "$service" -w >/dev/null 2>&1; then
		echo "  = $slug (already in the keychain)"
		skipped=$((skipped + 1))
		continue
	fi

	# Read through the same code pi uses, so legacy names and ambiguous
	# duplicates resolve exactly as they do at runtime.
	if ! node "$READER" "$spec" | "$WRITER" "$service" "$ACCOUNT"; then
		echo "  ! $slug FAILED — still served from the vault, nothing changed"
		failed=$((failed + 1))
		continue
	fi

	# cmp -s rather than comparing hashes: it reports a truncated or altered write
	# without either value being rendered anywhere. Keychain off on the left is
	# deliberate — the point is to compare the new store against the old source,
	# not against itself.
	if cmp -s <(PI_KEYCHAIN=0 PI_BW_TTL=0 node "$READER" "$spec") <(security find-generic-password -s "$service" -w); then
		echo "  + $slug -> $service  (verified against the vault)"
		moved=$((moved + 1))
	else
		echo "  ! $slug MISMATCH between the keychain and the vault — left in place, investigate"
		failed=$((failed + 1))
		continue
	fi

	# The vault path writes a plaintext copy into its TTL cache to avoid paying
	# `bw` repeatedly. That copy is what this migration exists to retire, so drop
	# it once the keychain answers for this slug.
	node "$READER" --forget "$slug" >/dev/null 2>&1 || true
done

echo
echo "  moved:   $moved"
echo "  skipped: $skipped (already present)"
echo "  failed:  $failed"
echo
echo "  Per-provider verification, which reports the store and never the key:"
echo "    node $READER --check <slug>"
