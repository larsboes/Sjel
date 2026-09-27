#!/bin/bash
# Drives the real tools/host-patch.sh against a planted PATH and a fixture root — never real
# brew. Asserts the three properties its own contract states: an absent upgrader is skipped and
# not failed, a failing step does not abort the ones after it, and the receipt is written and
# parseable whatever happened. Bash 3.2-safe.
set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

FIXTURE="$SCRATCH/root"
OVERLAY="$SCRATCH/overlay"
RECEIPT="$OVERLAY/data/host-patch/last.json"
MOCK_BIN="$SCRATCH/bin"
CALLS="$SCRATCH/calls.log"
mkdir -p "$FIXTURE/tools/lib" "$OVERLAY" "$MOCK_BIN"

cp "$ROOT/tools/host-patch.sh" "$FIXTURE/tools/host-patch.sh"
cat > "$FIXTURE/tools/lib/paths.sh" <<PATHS
SJEL_ROOT="$FIXTURE"
SJEL_PERSONAL_ROOT="$OVERLAY"
export SJEL_ROOT SJEL_PERSONAL_ROOT
PATHS

# The audit is a real invocation with a controlled exit code: host-patch must report the
# scanner's verdict, never decide it.
cat > "$FIXTURE/tools/audit" <<'AUDIT'
#!/bin/sh
printf 'audit\n' >> "$SJEL_TEST_CALLS"
exit "${SJEL_TEST_AUDIT_RC:-0}"
AUDIT
chmod +x "$FIXTURE/tools/audit" "$FIXTURE/tools/host-patch.sh"

for tool in brew uv rustup; do
  cat > "$MOCK_BIN/$tool" <<MOCK
#!/bin/sh
printf '$tool %s\n' "\$*" >> "\$SJEL_TEST_CALLS"
# uv's inventory: one installed tool, so the per-tool upgrade loop has something to upgrade.
[ "$tool \$1 \$2" = "uv tool list" ] && echo "demo-tool v1.0.0"
case "\$SJEL_TEST_FAIL_STEP" in
  "$tool \$1 \$2"|"$tool \$1") exit 3 ;;
esac
exit 0
MOCK
  chmod +x "$MOCK_BIN/$tool"
done

run_patch() {  # run_patch <PATH> — exit code left in $patch_rc, output in $SCRATCH/out
  rm -f "$RECEIPT" "$CALLS"
  env PATH="$1" SJEL_HOST_PATCH_KEEP_PATH=1 \
    SJEL_TEST_CALLS="$CALLS" \
    SJEL_TEST_FAIL_STEP="${SJEL_TEST_FAIL_STEP:-}" \
    SJEL_TEST_AUDIT_RC="${SJEL_TEST_AUDIT_RC:-0}" \
    "$FIXTURE/tools/host-patch.sh" >"$SCRATCH/out" 2>&1
  patch_rc=$?
}

receipt_field() {  # receipt_field <key> — through a real JSON parser, so a broken printf fails
  bun -e 'const r=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"));process.stdout.write(String(r[process.argv[2]]??""))' \
    "$RECEIPT" "$1"
}

# 1. Nothing installed. Every step is skipped, none is failed, and the run is a success —
#    a machine without rustup is not a failed patch run.
SJEL_TEST_FAIL_STEP="" SJEL_TEST_AUDIT_RC=0 run_patch "/usr/bin:/bin"
[ "$patch_rc" -eq 0 ] || {
  cat "$SCRATCH/out"; echo "FAIL: no upgrader installed must exit 0, got $patch_rc" >&2; exit 1; }
for label in "brew update" "uv tool upgrade" "rustup update"; do
  grep -F "$label" "$SCRATCH/out" | grep -F "skipped" >/dev/null || {
    cat "$SCRATCH/out"; echo "FAIL: '$label' was not reported as skipped" >&2; exit 1; }
done
[ -f "$RECEIPT" ] || { echo "FAIL: no receipt was written" >&2; exit 1; }
case "$(receipt_field skipped)" in
  *"rustup update"*) ;;
  *) cat "$RECEIPT"; echo "FAIL: the receipt did not record the skipped steps" >&2; exit 1 ;;
esac
[ -z "$(receipt_field failed)" ] || {
  cat "$RECEIPT"; echo "FAIL: a skipped step was recorded as failed" >&2; exit 1; }
[ "$(receipt_field audit)" = "clean" ] || {
  cat "$RECEIPT"; echo "FAIL: the receipt did not record a clean audit" >&2; exit 1; }

# 2. A failing brew step. The steps after it still run — a job that stops on the first broken
#    formula patches nothing after it — and the run exits 2.
SJEL_TEST_FAIL_STEP="brew upgrade --formula" SJEL_TEST_AUDIT_RC=0 run_patch "$MOCK_BIN:/usr/bin:/bin"
[ "$patch_rc" -eq 2 ] || {
  cat "$SCRATCH/out"; echo "FAIL: a failed step must exit 2, got $patch_rc" >&2; exit 1; }
for later in "uv tool upgrade demo-tool" "rustup update" "audit"; do
  grep -F "$later" "$CALLS" >/dev/null || {
    cat "$CALLS"; echo "FAIL: '$later' did not run after a failed step" >&2; exit 1; }
done
case "$(receipt_field failed)" in
  *"brew upgrade formula"*) ;;
  *) cat "$RECEIPT"; echo "FAIL: the receipt did not name the failed step" >&2; exit 1 ;;
esac
# --greedy is the whole cask step: without it brew skips every cask marked auto_updates or
# version :latest and still reports success, which is a patch run that patches nothing and
# says it did.
grep -F 'brew upgrade --cask --greedy' "$CALLS" >/dev/null || {
  cat "$CALLS"; echo "FAIL: the cask upgrade did not pass --greedy" >&2; exit 1; }

# 3. The audit's verdict is reported, not decided: a finding is exit 1 and is named in the
#    receipt, which is the field tools/doctor reads.
SJEL_TEST_FAIL_STEP="" SJEL_TEST_AUDIT_RC=1 run_patch "$MOCK_BIN:/usr/bin:/bin"
[ "$patch_rc" -eq 1 ] || {
  cat "$SCRATCH/out"; echo "FAIL: an audit finding must exit 1, got $patch_rc" >&2; exit 1; }
[ "$(receipt_field audit)" = "finding" ] || {
  cat "$RECEIPT"; echo "FAIL: the receipt did not record the audit finding" >&2; exit 1; }

# 4. No overlay means no receipt, and the run says so rather than passing silently — doctor
#    would otherwise report a job that ran as one that never has.
rm -f "$RECEIPT"
cat > "$FIXTURE/tools/lib/paths.sh" <<PATHS
SJEL_ROOT="$FIXTURE"
SJEL_PERSONAL_ROOT=""
export SJEL_ROOT SJEL_PERSONAL_ROOT
PATHS
SJEL_TEST_FAIL_STEP="" SJEL_TEST_AUDIT_RC=0 run_patch "$MOCK_BIN:/usr/bin:/bin"
[ "$patch_rc" -eq 0 ] || {
  cat "$SCRATCH/out"; echo "FAIL: an unconfigured overlay must not fail the patch run" >&2; exit 1; }
grep -F 'no overlay configured' "$SCRATCH/out" >/dev/null || {
  cat "$SCRATCH/out"; echo "FAIL: a run with no receipt did not say so" >&2; exit 1; }

echo "host-patch step guarding, failure isolation and receipt tests: pass"
