#!/bin/bash
# Planted-payload regression tests for check-site-payload.sh (#168).
#
# The check scans a directory of built bytes, so each case is a throwaway directory with one
# file in it and never needs private data. The negative cases matter more than the positive
# one: this gate is the last thing standing between a mistake in a seeder and a public URL,
# and a gate that silently stops matching is worse than no gate, because the build stays green.
set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/check-site-payload.sh"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

# No overlay: the derived-term half of the check is inert here, which is also how it behaves
# on a CI runner. The derived half is exercised by its own case at the bottom.
unset SJEL_OVERLAY_ROOT AXON_OVERLAY_ROOT

fails=0

plant() {  # plant <filename> <content>
  rm -rf "$SCRATCH/site"
  mkdir -p "$SCRATCH/site"
  printf '%s\n' "$2" > "$SCRATCH/site/$1"
}

expect_reject() {  # expect_reject <description>
  if "$CHECK" "$SCRATCH/site" >/dev/null 2>&1; then
    echo "FAIL: $1 should be rejected"; fails=$((fails + 1))
  fi
}

expect_pass() {  # expect_pass <description>
  if ! "$CHECK" "$SCRATCH/site" >/dev/null 2>&1; then
    echo "FAIL: $1 should pass"; fails=$((fails + 1))
    "$CHECK" "$SCRATCH/site" 2>&1 | sed 's/^/      /' | head -5
  fi
}

# ─── What must be rejected ────────────────────────────────────────────────────

plant fixture.json '{"from":"someone.real@gmail.com"}'
expect_reject "a real email address"

plant fixture.json '{"iban":"DE89370400440532013000"}'
expect_reject "an IBAN written as one run"

plant fixture.json '{"iban":"GB29 NWBK 6016 1331 9268 19"}'
expect_reject "an IBAN written in spaced groups"

# Assembled for the same reason as MARKER below: a tracked file containing a literal
# workstation path is itself what tools/check-publication-hygiene.sh rejects.
MAC_HOME="/""Users/someone/Developer/axon"
LINUX_HOME="/""home/someone/axon"

plant repos.json "{\"path\":\"$MAC_HOME\"}"
expect_reject "a macOS workstation home path"

plant repos.json "{\"path\":\"$LINUX_HOME\"}"
expect_reject "a Linux workstation home path"

plant systems.json '{"url":"https://somehost.tail1a2b3c.ts.net"}'
expect_reject "a tailnet hostname"

plant systems.json '{"host":"192.168.1.42"}'
expect_reject "an RFC1918 address"

# Assembled rather than written out: a tracked file containing the literal would itself trip
# tools/check-publication-hygiene.sh, and growing that script's exclusion list to cover test
# fixtures is how an exclusion list stops meaning anything.
MARKER="axon-$(printf 'personal')"

plant page.html "<p>see the $MARKER overlay</p>"
expect_reject "a deployment-instance marker"

plant fixture.json "{\"tag\":\"$MARKER-cents\",\"value\":1200}"
expect_pass "a journal tag that merely begins with a marker"

# ─── What must pass ───────────────────────────────────────────────────────────

plant fixture.json '{"from":"mara.velten@example.org","health":"http://127.0.0.1:8090/health"}'
expect_pass "a reserved documentation address and a loopback URL"

# The runner's own home is a portable public example, not somebody's machine — the same
# exemption tools/check-publication-hygiene.sh makes for it.
plant build.log 'built in /home/runner/work/Axon/Axon'
expect_pass "a CI runner home path"

# Regression: the derived half used to fire on github.com, because an overlay's systems file
# legitimately names it and every generated page links to it. Terms already present in this
# public repository cannot be leaked by publishing them again, so they are filtered out.
plant page.html '<a href="https://github.com/larsboes/Sjel">Source</a>'
expect_pass "a link to the repository the site is generated from"

# A commit sha is uppercase-free, but a base32-ish token can look like an IBAN prefix. This is
# the shape most likely to produce a false positive on a real bundle.
plant asset.js 'const HASH="a3f9c2e18b7d4600aa12cc34dd56ee78";'
expect_pass "a lowercase hex digest"

# ─── The derived half ─────────────────────────────────────────────────────────
#
# A fake overlay, so the case needs nothing real. The term must be one this repository does
# not itself contain, or the already-public filter correctly skips it.

# The machine name is generated, never written literally. The check skips any term this
# repository already contains, so a literal in THIS file would be tracked, filtered as
# already-public, and the rejection case would pass for the wrong reason — which is exactly
# what happened the first time it was committed.
MACHINE="demohost-$(basename "$SCRATCH")"

# The directory name is derived too. It used to be literally "overlay", which the check emits
# as a derived term; the passing case below happened to contain that word, and only the
# already-public filter finding "overlay" somewhere in the repo kept it green. Run against a
# tree with no repository to search, the filter correctly declined and the case failed.
OVERLAY="$SCRATCH/$MACHINE-root"
mkdir -p "$OVERLAY/config/machines"
touch "$OVERLAY/config/machines/$MACHINE.toml"
export SJEL_OVERLAY_ROOT="$OVERLAY"

plant fixture.json "{\"machine\":\"$MACHINE\"}"
expect_reject "a machine name read from the active overlay"

plant fixture.json '{"machine":"unrelated-value"}'
expect_pass "a payload naming no machine from the active overlay"

# The demo overlay is tracked and public by design, so nothing is derived from it.
export SJEL_OVERLAY_ROOT="$SCRATCH/nested/demo/overlay"
mkdir -p "$SJEL_OVERLAY_ROOT/config/machines"
touch "$SJEL_OVERLAY_ROOT/config/machines/$MACHINE.toml"
plant fixture.json "{\"machine\":\"$MACHINE\"}"
expect_pass "the demo overlay's own machine name"

if [ "$fails" -ne 0 ]; then
  echo "check-site-payload.test.sh: $fails failure(s)" >&2
  exit 1
fi
echo "check-site-payload.test.sh: all cases passed"
