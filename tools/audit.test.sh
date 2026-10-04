#!/bin/bash
# Proves tools/audit's exit contract and how it decides what a repository is.
# 0 clean · 1 a finding · 2 a scanner is not installed, and a finding outranks a missing
# scanner. Bash 3.2-safe.
#
# The tool is Rust since 2026-10-04, so this drives the launcher in a scratch root: the fixture
# carries tools/lib/sjel-cli.sh and SJEL_CLI_BIN points at a binary built from THIS checkout
# (tools/lib/test-support.sh). Everything else is unchanged — the fixture's paths.sh stub is still
# what hands the run its SJEL_ROOT and SJEL_PERSONAL_ROOT.
set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=tools/lib/test-support.sh
. "$ROOT/tools/lib/test-support.sh"
sjel_cli_prebuilt
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
MOCK_BIN="$SCRATCH/bin"
GITLEAKS_ONLY_BIN="$SCRATCH/bin-gitleaks-only"
mkdir -p "$MOCK_BIN" "$GITLEAKS_ONLY_BIN"

# Repository detection must use Git plumbing: linked worktrees have a .git file,
# not a directory. Run the real audit against isolated repositories with
# a deterministic gitleaks mock and an osv-scanner stub that always reports clean,
# so a non-zero exit can only have come from the half under test.
PRIMARY="$SCRATCH/primary"
LINKED="$SCRATCH/linked"
NONREPO="$SCRATCH/not-a-repo"
MISSING="$SCRATCH/not-reachable"
AUDIT_FIXTURE="$SCRATCH/audit-fixture"
GITLEAKS_LOG="$SCRATCH/gitleaks.log"
OSV_LOG="$SCRATCH/osv.log"
OSV_BOM="$SCRATCH/scanned-bom.json"
mkdir -p "$NONREPO" "$AUDIT_FIXTURE/tools/lib" "$PRIMARY/tools"

# The globally-installed surface's inventory comes from tools/updates, so the fixture carries
# a stub of it. The shape is tools/updates' `--inventory` payload: name/version/ecosystem pairs.
# The brew row is deliberately NOT one of the two ecosystems that pass scans, so the SBOM
# assertion below has something it must have left out rather than mislabelled.
GLOBALS_INVENTORY='{"inventory":[
  {"ecosystem":"crates.io","name":"macmon","version":"0.8.2"},
  {"ecosystem":"npm","name":"@sinclair/typebox","version":"0.34.52"},
  {"ecosystem":"brew","name":"jq","version":"1.7.1"}]}'
cat > "$PRIMARY/tools/updates" <<'STUB'
#!/bin/sh
printf '%s\n' "${SJEL_AUDIT_UPDATES_JSON:-}"
exit "${SJEL_AUDIT_UPDATES_RC:-0}"
STUB
chmod +x "$PRIMARY/tools/updates"

# A CARGO_HOME with one installed crate's published lockfile in it, so the transitive half of
# the pass has something to find. osv-scanner is mocked in these tests, so the contents are
# never parsed — what is asserted is that the file was FOUND and handed over.
CARGO_HOME_FIXTURE="$SCRATCH/cargo-home"
mkdir -p "$CARGO_HOME_FIXTURE/registry/src/index.crates.io-test/macmon-0.8.2"
printf '%s\n' '# planted so the lockfile-copying half of the pass has a file to find' \
  > "$CARGO_HOME_FIXTURE/registry/src/index.crates.io-test/macmon-0.8.2/Cargo.lock"
# and one for a crate that is not installed, which must NOT be picked up
mkdir -p "$CARGO_HOME_FIXTURE/registry/src/index.crates.io-test/absent-9.9.9"
printf '%s\n' '# an uninstalled crate: reaching this would report a version nobody has' \
  > "$CARGO_HOME_FIXTURE/registry/src/index.crates.io-test/absent-9.9.9/Cargo.lock"

git init -q "$PRIMARY"
git -C "$PRIMARY" -c user.name=Axon -c user.email=axon@example.invalid \
  commit -q --allow-empty -m initial
git -C "$PRIMARY" worktree add -q -b audit-linked "$LINKED"

cp "$ROOT/tools/audit" "$AUDIT_FIXTURE/tools/audit"
cp "$ROOT/tools/lib/sjel-cli.sh" "$AUDIT_FIXTURE/tools/lib/sjel-cli.sh"
cat > "$AUDIT_FIXTURE/tools/lib/paths.sh" <<'PATHS'
SJEL_ROOT="$SJEL_AUDIT_TEST_ROOT"
SJEL_PERSONAL_ROOT="${SJEL_AUDIT_TEST_OVERLAY:-}"
export SJEL_ROOT SJEL_PERSONAL_ROOT
PATHS

cat > "$MOCK_BIN/gitleaks" <<'MOCK'
#!/bin/sh
printf '%s\n' "$*" >> "$SJEL_AUDIT_GITLEAKS_LOG"
exit "${SJEL_AUDIT_GITLEAKS_RC:-0}"
MOCK
# The osv-scanner mock does three jobs: it records its argv, it hands the SBOM it was asked to
# scan back to the test (the real script deletes its temp directory), and it can fail the two
# passes independently — so a failure attributed to the wrong pass is visible rather than
# inferred from a shared exit code.
#
# `cp` runs in the for-loop rather than assuming -L's argument is $2, because the second pass
# puts -L first and the first pass puts it last.
cat > "$MOCK_BIN/osv-scanner" <<'MOCK'
#!/bin/sh
if [ -n "${SJEL_AUDIT_OSV_LOG:-}" ]; then
  printf '%s\n' "$*" >> "$SJEL_AUDIT_OSV_LOG"
fi
prev=""
for a in "$@"; do
  if [ "$prev" = "-L" ] && [ -n "${SJEL_AUDIT_OSV_BOM:-}" ]; then
    cp "$a" "$SJEL_AUDIT_OSV_BOM" 2>/dev/null || true
  fi
  prev="$a"
done
rc="${SJEL_AUDIT_OSV_RC:-0}"
case "$*" in
  *globals.cdx.json*) rc="${SJEL_AUDIT_OSV_GLOBALS_RC:-$rc}" ;;
esac
exit "$rc"
MOCK
cp "$MOCK_BIN/gitleaks" "$GITLEAKS_ONLY_BIN/gitleaks"
chmod +x "$MOCK_BIN/gitleaks" "$MOCK_BIN/osv-scanner" \
  "$GITLEAKS_ONLY_BIN/gitleaks" "$AUDIT_FIXTURE/tools/audit"

run_gitleaks_audit() {
  SJEL_AUDIT_GITLEAKS_LOG="$GITLEAKS_LOG" \
  SJEL_AUDIT_OSV_LOG="$OSV_LOG" \
  SJEL_AUDIT_OSV_BOM="$OSV_BOM" \
  SJEL_AUDIT_UPDATES_JSON="${GLOBALS_INVENTORY}" \
  SJEL_AUDIT_TEST_ROOT="$PRIMARY" \
  SJEL_AUDIT_TEST_OVERLAY="$1" \
  SJEL_AUDIT_GITLEAKS_RC="${2:-0}" \
  CARGO_HOME="$CARGO_HOME_FIXTURE" \
  PATH="$MOCK_BIN:$PATH" \
    "$AUDIT_FIXTURE/tools/audit"
}

# A variant that lets a caller override the inventory, for the branch where tools/updates
# cannot be read at all.
run_globals_audit() {
  SJEL_AUDIT_OSV_LOG="$OSV_LOG" \
  SJEL_AUDIT_OSV_BOM="$OSV_BOM" \
  SJEL_AUDIT_UPDATES_JSON="$1" \
  SJEL_AUDIT_OSV_GLOBALS_RC="${2:-0}" \
  SJEL_AUDIT_TEST_ROOT="$PRIMARY" \
  CARGO_HOME="$CARGO_HOME_FIXTURE" \
  PATH="$MOCK_BIN:$PATH" \
    "$AUDIT_FIXTURE/tools/audit"
}

: > "$GITLEAKS_LOG"
run_gitleaks_audit "$LINKED" >"$SCRATCH/worktree.out" 2>&1 || {
  cat "$SCRATCH/worktree.out"
  echo "FAIL: linked worktree audit did not complete cleanly" >&2
  exit 1
}
grep -F -- "-s $PRIMARY " "$GITLEAKS_LOG" >/dev/null || {
  echo "FAIL: primary repository was not scanned" >&2
  exit 1
}
grep -F -- "-s $LINKED " "$GITLEAKS_LOG" >/dev/null || {
  echo "FAIL: linked worktree was not scanned" >&2
  exit 1
}
if grep -F "$LINKED" "$SCRATCH/worktree.out" >/dev/null; then
  echo "FAIL: audit output exposed the private overlay coordinate" >&2
  exit 1
fi
grep -F 'private overlay — clean' "$SCRATCH/worktree.out" >/dev/null || {
  echo "FAIL: linked worktree clean status was not explicit" >&2
  exit 1
}

: > "$GITLEAKS_LOG"
if run_gitleaks_audit "$NONREPO" >"$SCRATCH/nonrepo.out" 2>&1; then
  echo "FAIL: non-repository overlay produced a clean audit" >&2
  exit 1
fi
grep -F 'private overlay — not a Git repository' "$SCRATCH/nonrepo.out" >/dev/null || {
  echo "FAIL: non-repository status was not explicit" >&2
  exit 1
}
if grep -F "$NONREPO" "$GITLEAKS_LOG" >/dev/null; then
  echo "FAIL: gitleaks was invoked for a non-repository" >&2
  exit 1
fi

if run_gitleaks_audit "$MISSING" >"$SCRATCH/missing.out" 2>&1; then
  echo "FAIL: unreachable overlay produced a clean audit" >&2
  exit 1
fi
grep -F 'private overlay — not reachable' "$SCRATCH/missing.out" >/dev/null || {
  echo "FAIL: unreachable status was not explicit" >&2
  exit 1
}

run_gitleaks_audit "" >"$SCRATCH/unconfigured.out" 2>&1 || {
  echo "FAIL: an unconfigured optional overlay failed the Axon audit" >&2
  exit 1
}
grep -F 'private overlay — not configured' "$SCRATCH/unconfigured.out" >/dev/null || {
  echo "FAIL: unconfigured status was not explicit" >&2
  exit 1
}

if run_gitleaks_audit "" 1 >"$SCRATCH/leak.out" 2>&1; then
  echo "FAIL: mocked leak produced a clean audit" >&2
  exit 1
fi
grep -F 'Axon — leak(s) found' "$SCRATCH/leak.out" >/dev/null || {
  echo "FAIL: leak status was not explicit" >&2
  exit 1
}

if run_gitleaks_audit "" 2 >"$SCRATCH/error.out" 2>&1; then
  echo "FAIL: mocked scanner error produced a clean audit" >&2
  exit 1
fi
grep -F 'Axon — gitleaks errored (exit 2' "$SCRATCH/error.out" >/dev/null || {
  echo "FAIL: scanner-error status was not explicit" >&2
  exit 1
}

# --- the globally installed surface ---------------------------------------
#
# The second osv-scanner pass covers software installed OUTSIDE this checkout — the class
# `tools/updates` is solely responsible for moving, and the class no scanner looked at before
# 2026-10-03. These assertions are about the SBOM it builds being the right one, and about a
# failure in this pass not being attributed to the repository pass or to nothing at all.

: > "$OSV_LOG"
rm -f "$OSV_BOM"
run_gitleaks_audit "" >"$SCRATCH/globals.out" 2>&1 || {
  cat "$SCRATCH/globals.out"
  echo "FAIL: audit with a global inventory did not complete cleanly" >&2
  exit 1
}
grep -F '2 installed package(s)' "$SCRATCH/globals.out" >/dev/null || {
  cat "$SCRATCH/globals.out"
  echo "FAIL: the inventory was not counted, or the brew entry was counted with it" >&2
  exit 1
}
# The installed crate's published Cargo.lock was found and handed over; the crate that is in the
# registry but not installed was not. Its absence is the whole reason the version is matched.
grep -F '1 crate lockfile(s)' "$SCRATCH/globals.out" >/dev/null || {
  cat "$SCRATCH/globals.out"
  echo "FAIL: an installed crate's lockfile was not found, or an uninstalled one was" >&2
  exit 1
}
[ -s "$OSV_BOM" ] || {
  echo "FAIL: no SBOM was handed to osv-scanner" >&2
  exit 1
}
grep -F '"pkg:cargo/macmon@0.8.2"' "$OSV_BOM" >/dev/null || {
  cat "$OSV_BOM"
  echo "FAIL: the installed cargo crate did not reach the SBOM" >&2
  exit 1
}
# Scoped npm names are percent-encoded, which is what purl requires and what osv-scanner
# resolves: a raw '@scope/name' is accepted too, but encoding is the spec-conformant form.
grep -F '"pkg:npm/%40sinclair/typebox@0.34.52"' "$OSV_BOM" >/dev/null || {
  cat "$OSV_BOM"
  echo "FAIL: the scoped npm package did not reach the SBOM encoded" >&2
  exit 1
}
if grep -F 'pkg:npm/jq@1.7.1' "$OSV_BOM" >/dev/null; then
  echo "FAIL: an ecosystem this pass does not scan was labelled as npm" >&2
  exit 1
fi

# A finding in THIS pass is a finding, and it is this pass that produced it: the repository
# pass is left clean by the mock, so exit 1 can only have come from the global one.
if run_globals_audit "$GLOBALS_INVENTORY" 1 >"$SCRATCH/globals-finding.out" 2>&1; then
  cat "$SCRATCH/globals-finding.out"
  echo "FAIL: a vulnerability in globally installed software produced a clean audit" >&2
  exit 1
fi
grep -F 'vulnerabilities found above' "$SCRATCH/globals-finding.out" >/dev/null || {
  cat "$SCRATCH/globals-finding.out"
  echo "FAIL: the global finding was not reported" >&2
  exit 1
}

# An inventory that cannot be read is a setup error — never clean, and never a finding. This is
# the same distinction the missing-scanner case makes: an audit that did not run proves nothing.
run_globals_audit "" >"$SCRATCH/globals-noinv.out" 2>&1
noinv_rc=$?
[ "$noinv_rc" -eq 2 ] || {
  cat "$SCRATCH/globals-noinv.out"
  echo "FAIL: an unreadable inventory must exit 2, got $noinv_rc" >&2
  exit 1
}
grep -F 'returned no inventory' "$SCRATCH/globals-noinv.out" >/dev/null || {
  cat "$SCRATCH/globals-noinv.out"
  echo "FAIL: the unreadable inventory was not named" >&2
  exit 1
}
if grep -F 'vulnerabilities found above' "$SCRATCH/globals-noinv.out" >/dev/null; then
  echo "FAIL: an unreadable inventory was reported as a finding" >&2
  exit 1
fi

# --- the exit contract ----------------------------------------------------
#
# A scanner that is not installed is a setup error, not a finding. Until 2026-09-02 a 127
# fell into the finding branch, so this Mac reported fabricated security findings on every
# run — a gate that cries wolf over an absent binary is worse than no gate.

env PATH="/usr/bin:/bin" SJEL_AUDIT_TEST_ROOT="$PRIMARY" \
  "$AUDIT_FIXTURE/tools/audit" >"$SCRATCH/nobin.out" 2>&1
nobin_rc=$?
[ "$nobin_rc" -eq 2 ] || {
  cat "$SCRATCH/nobin.out"
  echo "FAIL: audit with no scanner on PATH must exit 2, got $nobin_rc" >&2
  exit 1
}
grep -F 'not installed' "$SCRATCH/nobin.out" >/dev/null || {
  echo "FAIL: missing scanner was not named" >&2
  exit 1
}
if grep -F 'finding(s)' "$SCRATCH/nobin.out" >/dev/null; then
  cat "$SCRATCH/nobin.out"
  echo "FAIL: a missing scanner was reported as a finding" >&2
  exit 1
fi

# Precedence: a real finding outranks a missing scanner, so a run with both exits 1.
# Otherwise a leak would be reported under the exit code that means "nothing was scanned".
: > "$GITLEAKS_LOG"
env PATH="$GITLEAKS_ONLY_BIN:/usr/bin:/bin" \
  SJEL_AUDIT_GITLEAKS_LOG="$GITLEAKS_LOG" \
  SJEL_AUDIT_GITLEAKS_RC=1 \
  SJEL_AUDIT_TEST_ROOT="$PRIMARY" \
  "$AUDIT_FIXTURE/tools/audit" >"$SCRATCH/both.out" 2>&1
both_rc=$?
[ "$both_rc" -eq 1 ] || {
  cat "$SCRATCH/both.out"
  echo "FAIL: a finding beside a missing scanner must exit 1, got $both_rc" >&2
  exit 1
}
grep -F 'not installed' "$SCRATCH/both.out" >/dev/null || {
  echo "FAIL: the missing scanner was not reported alongside the finding" >&2
  exit 1
}

echo "audit exit contract and repository detection tests: pass"
