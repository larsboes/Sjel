#!/bin/bash
# Tests for tools/toolchain-check's `needed_by` scoping (#163).
#
# The defect this covers is a false requirement: a global toolchain check reports a tool as
# MISSING on a machine that has no path to invoke it. A runtime node told to install a SAST
# scanner learns to ignore the check, and then misses the one entry that mattered. The mirror
# defect is just as bad — a restore that discovers a missing dependency after it has already
# held a service down.
#
# Built on the same throwaway-root idea as persistence.test.sh: a scratch Axon root with its own
# overlay, machine.toml, toolchain.toml and capability manifests, plus a stub PATH so presence and
# absence are decided here rather than by whatever the host happens to have installed. Nothing in
# this file consults the real machine.
set -uo pipefail

fails=0
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }

_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
SRC_TOOLS=""
for _c in "$_dir" "$_dir/tools"; do
  if [ -f "$_c/toolchain-check" ]; then SRC_TOOLS="$_c"; break; fi
done
[ -n "$SRC_TOOLS" ] || { echo "toolchain-scope: cannot find toolchain-check next to $_dir" >&2; exit 1; }

# Before the scratch root exists, and it is the whole of defect D7. This suite was excluded from
# CI on 2026-08-26 as "only ever green inside the Bazel sandbox", on a measurement taken on a
# machine whose shell exports the SJEL_* set — which the overlay's own config/shell does. The
# sandbox was not hiding a scoping bug; it was scrubbing the environment. Without this line the
# four capability-field checks read the OPERATOR's machine.toml and the OPERATOR's capabilities
# directory instead of the fixtures written twenty lines below, so `withdb` is never enabled
# anywhere and `fixturedb` correctly reads n/a. Cleared, all 30 checks pass; the exclusion in
# .github/workflows/ci.yml is gone with it (tools/lib/test-support.sh#isolate_axon_env).
source "$SRC_TOOLS/lib/test-support.sh"
isolate_axon_env

SCRATCH="$(mktemp -d "/tmp/toolchain-scope.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT
ROOT="$SCRATCH/axon"; OVERLAY="$SCRATCH/overlay"; STUB_BIN="$SCRATCH/bin"
mkdir -p "$ROOT/tools/lib" "$OVERLAY/config" "$STUB_BIN" "$ROOT/capabilities"
cp "$SRC_TOOLS/toolchain-check" "$ROOT/tools/"
cp "$SRC_TOOLS"/lib/*.sh "$ROOT/tools/lib/"

# The logic lives in the sjel-cli crate since 2026-10-02, and the scratch root has no crate.
sjel_cli_prebuilt
printf 'overlay = "%s"\n' "$OVERLAY" > "$ROOT/axon.toml"

# A toolchain manifest with one entry per scope class. Deliberately not a copy of the real one:
# these assertions are about the scoping mechanism, and pinning them to the live manifest would
# make every future entry a test edit.
cat > "$ROOT/toolchain.toml" <<'TOML'
[fixturecore]
required = "yes"
why = "always needed"
install_linux = "apt-get install -y fixturecore"

[fixturebackup]
required = "optional"
needed_by = ["workflow:backup"]
why = "only a backup run needs this"
install_linux = "apt-get install -y fixturebackup"

[fixturedb]
required = "optional"
needed_by = ["capability-field:backup_sqlite"]
why = "only capabilities declaring backup_sqlite need this"
install_linux = "apt-get install -y fixturedb"

[fixturescan]
required = "optional"
needed_by = ["workflow:audit"]
why = "only an audit run needs this"
install_linux = "apt-get install -y fixturescan"

[fixturetypo]
required = "optional"
needed_by = ["workflo:backup"]
why = "a deliberately misspelled scope token"
install_linux = "apt-get install -y fixturetypo"
TOML

# Every declared tool is ABSENT unless a test installs a stub, so "in scope" and "present" stay
# independent — the thing under test is which entries get checked at all.
PATH="$STUB_BIN:$PATH"; export PATH
install_stub() { printf '#!/bin/bash\nexit 0\n' > "$STUB_BIN/$1"; chmod +x "$STUB_BIN/$1"; }

# Checked BEFORE anything runs, not after. A fixture name that already exists on the host would
# make every absence assertion below pass for the wrong reason — which is exactly what happened
# with the first draft's `dbtool`, a real binary in /opt/homebrew/bin on the authoring machine.
for _t in fixturebackup fixturedb fixturescan fixturetypo; do
  if command -v "$_t" >/dev/null 2>&1; then
    echo "FAIL: fixture name '$_t' resolves to a real binary on this host ($(command -v "$_t"))."
    echo "      Absence assertions would pass for the wrong reason. Rename the fixture."
    exit 1
  fi
done

install_stub fixturecore

machine() {  # machine <capabilities-toml-array>
  printf 'os = "linux"\ncontainer_runtime = "docker"\ncapabilities = %s\n' "$1" > "$OVERLAY/config/machine.toml"
}
mkcap() {  # mkcap <name> [extra manifest lines]
  mkdir -p "$ROOT/capabilities/$1"
  { printf 'kind = "container"\nname = "%s"\nimage = "x"\ntag = "1"\n' "$1"
    shift; [ $# -gt 0 ] && printf '%s\n' "$@"
  } > "$ROOT/capabilities/$1/service.toml"
}

run() {  # run [args...]
  out="$("$ROOT/tools/toolchain-check" --os linux --runtime docker "$@" 2>&1)"
  status=$?
}
line_for() { printf '%s' "$out" | grep -E "^▸ $1 " | head -1; }

in_scope() {  # in_scope <label> <tool>
  local got; got="$(line_for "$2")"
  case "$got" in
    *"n/a here"*) fail "$1: '$2' should be in scope, got: $got" ;;
    "")           fail "$1: '$2' produced no line at all (full: $out)" ;;
  esac
}
out_of_scope() {  # out_of_scope <label> <tool> [expected scope substring]
  local got; got="$(line_for "$2")"
  case "$got" in
    *"n/a here"*) [ $# -lt 3 ] || case "$got" in *"$3"*) ;; *) fail "$1: '$2' n/a should name $3, said: $got" ;; esac ;;
    "")           fail "$1: '$2' produced no line at all — an out-of-scope tool must still be reported" ;;
    *)            fail "$1: '$2' should be out of scope, got: $got" ;;
  esac
}

# --- a minimal runtime node: core only -----------------------------------------------------
# The motivating machine. It runs services and never audits, builds or backs up, so the only
# thing it owes is the core set.
mkcap plain
machine '["plain"]'
run
in_scope "runtime node" fixturecore
out_of_scope "runtime node" fixturebackup "workflow:backup"
out_of_scope "runtime node" fixturescan "workflow:audit"
out_of_scope "runtime node" fixturedb "capability-field:backup_sqlite"
[ "$status" -eq 0 ] || fail "runtime node: absent out-of-scope tools must not fail the check (exit $status)"

# An out-of-scope entry is REPORTED, not dropped. A silent omission teaches an operator nothing
# about why a tool they can see in the manifest was never checked.
case "$out" in
  *"n/a here — needed by workflow:audit"*) ;;
  *) fail "the n/a line must name the scope that would pull the tool in, said: $out" ;;
esac

# --- capability-derived scope --------------------------------------------------------------
# Enabling a capability that declares the field is what pulls the tool in. No machine-role list
# anywhere: the deployment decides, so moving the capability moves the requirement with it.
mkcap withdb 'backup_sqlite = "data/withdb/db.sqlite3"'
machine '["plain", "withdb"]'
run
in_scope "capability declares backup_sqlite" fixturedb
[ "$status" -eq 0 ] || fail "an in-scope OPTIONAL tool that is absent must not fail the check (exit $status)"
case "$(line_for fixturedb)" in
  *absent*) ;;
  *) fail "fixturedb is in scope and not installed, so it should read as absent: $(line_for fixturedb)" ;;
esac

# ...and disabling it takes the requirement away again, which is the half a static list gets wrong.
machine '["plain"]'
run
out_of_scope "capability disabled again" fixturedb

# The exact false positive found while building this: a manifest MENTIONING the field in a
# comment must not pull the tool in. The case was capabilities/postgres/service.toml, whose only
# occurrence of `backup_sqlite` was a comment explaining why it did NOT use one — retired with
# the capability on 2026-08-27, so the fixture below is the whole record of it now.
mkcap commented '# backup_sqlite would not help here -- see the README'
machine '["commented"]'
run
out_of_scope "field named only in a comment" fixturedb

# --- workflow scope ------------------------------------------------------------------------
machine '["plain"]'
run --workflow backup
in_scope "--workflow backup" fixturebackup
in_scope "--workflow backup" fixturecore
out_of_scope "--workflow backup" fixturescan "workflow:audit"

run --workflow audit
in_scope "--workflow audit" fixturescan
out_of_scope "--workflow audit" fixturebackup "workflow:backup"

# A workflow nobody declares pulls nothing extra in, and does not error.
run --workflow nosuchworkflow
in_scope "unknown workflow still checks core" fixturecore
out_of_scope "unknown workflow" fixturebackup
[ "$status" -eq 0 ] || fail "an unrecognised workflow name should not be an error (exit $status)"

# --- a maintainer workstation: scopes compose ------------------------------------------------
# The third machine #163 names, and the one the cases above cannot speak for: they each exercise
# one scope with the others cleared out. A maintainer workstation is where several are live at
# once — it runs capabilities AND audits AND builds — so the defect it guards is scopes REPLACING
# each other instead of adding up. Asking about a workflow must not evict what the enabled
# capabilities already put in scope, or the operator who asks the narrower question gets the
# narrower answer and installs half of what the next command needs.
mkcap withdb2 'backup_sqlite = "data/withdb2/db.sqlite3"'
machine '["plain", "withdb2"]'
run --workflow audit
in_scope "workstation: capability scope survives a workflow question" fixturedb
in_scope "workstation: the asked-for workflow is in scope" fixturescan
in_scope "workstation: core is always in scope" fixturecore
out_of_scope "workstation: an unasked workflow stays out" fixturebackup "workflow:backup"

# ...and the same machine asked about the other workflow moves only that one boundary.
run --workflow backup
in_scope "workstation: capability scope is not workflow-dependent" fixturedb
in_scope "workstation: the newly asked workflow comes in" fixturebackup
out_of_scope "workstation: the previously asked workflow goes back out" fixturescan "workflow:audit"

# --- an in-scope required miss still fails --------------------------------------------------
# The scoping must narrow WHAT is checked without weakening the verdict on what remains.
rm -f "$STUB_BIN/fixturecore"
run
[ "$status" -ne 0 ] || fail "a missing core tool must still fail the check"
case "$(line_for fixturecore)" in
  *MISSING*) ;;
  *) fail "a missing core tool should read as MISSING: $(line_for fixturecore)" ;;
esac
install_stub fixturecore

# --- a typo in a scope token is named, never silently widening -------------------------------
# "workflo:backup" is not a scope this file knows. The dangerous failure would be treating an
# unrecognised token as core and demanding the tool everywhere.
run
out_of_scope "misspelled scope token" fixturetypo "unknown scope"
run --workflow backup
out_of_scope "misspelled token stays out even for the intended workflow" fixturetypo "unknown scope"

# --- JSON carries the same verdict ----------------------------------------------------------
out="$("$ROOT/tools/toolchain-check" --os linux --runtime docker --json 2>&1)"
case "$out" in
  *'"workflow":""'*) ;;
  *) fail "JSON should carry the resolved workflow, said: $out" ;;
esac
case "$out" in
  *'"tool":"fixturescan"'*'"status":"n/a"'*) ;;
  *) fail "JSON should mark an out-of-scope tool n/a, said: $out" ;;
esac
out="$("$ROOT/tools/toolchain-check" --os linux --runtime docker --workflow audit --json 2>&1)"
case "$out" in
  *'"workflow":"audit"'*) ;;
  *) fail "JSON should echo the requested workflow, said: $out" ;;
esac

# --- the report never leaks the host's own state --------------------------------------------
run
case "$out" in
  *"$SCRATCH"*) ;;
  *) fail "the answer should reference the scratch root, said: $out" ;;
esac

if [ "$fails" -gt 0 ]; then
  echo "toolchain-scope: $fails check(s) failed"
  exit 1
fi
echo "toolchain-scope: all checks passed"
