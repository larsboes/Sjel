#!/bin/bash
# Drives the real tools/container-refresh.sh against a planted PATH, a fixture root and a docker
# stub whose digests this test controls — never a real registry and never a real container.
# Asserts the properties its own contract states: an absent runtime is skipped and not failed, a
# host with no container capability writes a receipt and exits 0, the DIGEST decides whether
# anything is recreated, a held capability is pulled but left alone, and one failing pull does not
# stop the images after it. Bash 3.2-safe.
set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRATCH="$(mktemp -d)"
# The maintenance lock path is service-runner's, and it is absolute (/tmp/axon-<cap>.maintenance).
# The fixture capability names are deliberately ones no deployment uses, so this test can create
# and delete that file without touching a real hold.
trap 'rm -rf "$SCRATCH"; rm -f /tmp/axon-refresh-alpha.maintenance /tmp/axon-refresh-beta.maintenance' EXIT

FIXTURE="$SCRATCH/root"
OVERLAY="$SCRATCH/overlay"
STATE="$SCRATCH/state"
RECEIPT="$OVERLAY/data/container-refresh/last.json"
MOCK_BIN="$SCRATCH/bin"
CALLS="$SCRATCH/calls.log"
MACHINE="$OVERLAY/config/machine.toml"
mkdir -p "$FIXTURE/tools/lib" "$FIXTURE/capabilities" "$OVERLAY/config" "$STATE" "$MOCK_BIN"

cp "$ROOT/tools/container-refresh.sh" "$FIXTURE/tools/container-refresh.sh"
# The real libraries, not stubs: platform.sh's runtime resolution and pipe.sh's exact-line match
# are two of the things under test, and a reimplementation here would prove nothing about them.
cp "$ROOT/tools/lib/toml.sh" "$ROOT/tools/lib/platform.sh" "$ROOT/tools/lib/pipe.sh" "$FIXTURE/tools/lib/"

# paths.sh is the one library this fixture replaces. The real one resolves an overlay from
# axon.toml, axon.local.toml and the hostname; this test needs a known root instead.
cat > "$FIXTURE/tools/lib/paths.sh" <<PATHS
SJEL_ROOT="$FIXTURE"
SJEL_PERSONAL_ROOT="$OVERLAY"
SJEL_OVERLAY_ROOT="$OVERLAY"
SJEL_MACHINE_TOML="$MACHINE"
export SJEL_ROOT SJEL_PERSONAL_ROOT SJEL_OVERLAY_ROOT SJEL_MACHINE_TOML
source "$FIXTURE/tools/lib/toml.sh"
axon_manifest_for() {
  [ -f "$FIXTURE/capabilities/\$1/service.toml" ] || return 1
  echo "$FIXTURE/capabilities/\$1/service.toml"
}
PATHS

cat > "$FIXTURE/tools/service-runner.sh" <<'RUNNER'
#!/bin/sh
printf 'service-runner %s\n' "$*" >> "$SJEL_TEST_CALLS"
exit "${SJEL_TEST_RECREATE_RC:-0}"
RUNNER
chmod +x "$FIXTURE/tools/service-runner.sh" "$FIXTURE/tools/container-refresh.sh"

# The docker stub. `image inspect` answers from a per-reference file this test writes, `pull`
# promotes that reference's "next" digest over its current one, and `ps` prints whatever the test
# declared running. That makes the digest transition — the only thing that may trigger a
# recreate — something the test states rather than something a registry decides.
cat > "$MOCK_BIN/docker" <<'DOCKER'
#!/bin/sh
printf 'docker %s\n' "$*" >> "$SJEL_TEST_CALLS"
_key() { echo "$1" | tr -c 'A-Za-z0-9' '_'; }
case "$1" in
  image)   # image inspect <ref> --format <fmt>
    f="$SJEL_TEST_STATE/digest.$(_key "$3")"
    [ -f "$f" ] || exit 1
    cat "$f"
    ;;
  pull)
    if [ "${SJEL_TEST_FAIL_PULL:-}" = "$2" ]; then
      echo "stub: refusing to pull $2" >&2
      exit 1
    fi
    n="$SJEL_TEST_STATE/next.$(_key "$2")"
    [ -f "$n" ] && cp "$n" "$SJEL_TEST_STATE/digest.$(_key "$2")"
    echo "Status: pulled $2"
    ;;
  ps)
    cat "$SJEL_TEST_STATE/running" 2>/dev/null || true
    ;;
  *) exit 0 ;;
esac
exit 0
DOCKER
chmod +x "$MOCK_BIN/docker"

digest_key() { echo "$1" | tr -c 'A-Za-z0-9' '_'; }
set_digest() {  # set_digest <ref> <now> <after-pull>
  printf '%s\n' "$2" > "$STATE/digest.$(digest_key "$1")"
  printf '%s\n' "$3" > "$STATE/next.$(digest_key "$1")"
}

write_manifest() {  # write_manifest <cap> <container-name> <image> <tag>
  mkdir -p "$FIXTURE/capabilities/$1"
  cat > "$FIXTURE/capabilities/$1/service.toml" <<MANIFEST
name  = "$2"
image = "$3"
tag   = "$4"
MANIFEST
}

write_machine() {  # write_machine <runtime> <cap...>
  runtime="$1"; shift
  list=""
  for c in "$@"; do list="$list\"$c\", "; done
  cat > "$MACHINE" <<MACHINE
os = "linux"
container_runtime = "$runtime"
capabilities = [${list%, }]
MACHINE
}

run_refresh() {  # run_refresh <PATH> — exit code in $rc, output in $SCRATCH/out
  rm -f "$RECEIPT" "$CALLS" 2>/dev/null
  env PATH="$1" SJEL_CONTAINER_REFRESH_KEEP_PATH=1 \
    SJEL_TEST_CALLS="$CALLS" \
    SJEL_TEST_STATE="$STATE" \
    SJEL_TEST_FAIL_PULL="${SJEL_TEST_FAIL_PULL:-}" \
    SJEL_TEST_RECREATE_RC="${SJEL_TEST_RECREATE_RC:-0}" \
    "$FIXTURE/tools/container-refresh.sh" >"$SCRATCH/out" 2>&1
  rc=$?
}

receipt_field() {  # receipt_field <key> — through a real JSON parser, so a broken printf fails
  bun -e 'const r=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"));process.stdout.write(String(r[process.argv[2]]??""))' \
    "$RECEIPT" "$1"
}

fail() { cat "$SCRATCH/out"; echo "FAIL: $1" >&2; exit 1; }

write_manifest refresh-alpha alpha-container example.org/alpha stable
write_manifest refresh-beta  beta-container  example.org/beta  latest

# 1. No enabled capability declares an image. This is the normal workstation case: a receipt that
#    says so, and exit 0. Silence here would be indistinguishable from a job that stopped firing.
write_machine docker
SJEL_TEST_FAIL_PULL="" run_refresh "$MOCK_BIN:/usr/bin:/bin"
[ "$rc" -eq 0 ] || fail "a host with no container capability must exit 0, got $rc"
[ -f "$RECEIPT" ] || fail "no receipt was written"
case "$(receipt_field skipped)" in
  *no-container-capabilities*) ;;
  *) cat "$RECEIPT"; fail "the receipt did not record that there was nothing to refresh" ;;
esac
[ -s "$CALLS" ] && fail "the runtime was called although nothing declares an image"

# 2. Capabilities enabled, runtime not on PATH. Skipped, not failed — the same rule
#    tools/host-patch.sh states for a missing upgrader.
# "Absent" has to be built, not assumed: GitHub's Ubuntu runners ship /usr/bin/docker, so a
# planted PATH of /usr/bin:/bin finds a real daemon there and this case turns into a real pull
# (measured 2026-09-02: the case passed on macOS and failed in CI with exit 2). The bin dir
# below is /usr/bin and /bin with every container CLI left out.
NODOCKER_BIN="$SCRATCH/nodocker-bin"
mkdir -p "$NODOCKER_BIN"
for d in /usr/bin /bin; do
  for f in "$d"/*; do
    case "$(basename "$f")" in docker|podman|nerdctl) continue ;; esac
    [ -x "$f" ] && ln -s "$f" "$NODOCKER_BIN/$(basename "$f")" 2>/dev/null
  done
done
write_machine docker refresh-alpha
SJEL_TEST_FAIL_PULL="" run_refresh "$NODOCKER_BIN"
[ "$rc" -eq 0 ] || fail "an absent runtime must exit 0, got $rc"
case "$(receipt_field skipped)" in
  *docker-not-installed*) ;;
  *) cat "$RECEIPT"; fail "the receipt did not record the absent runtime" ;;
esac
[ -z "$(receipt_field failed)" ] || fail "an absent runtime was recorded as a failure"

# 3. An unchanged digest recreates nothing. This is what makes a daily pull cheap enough to
#    schedule: the tag moves rarely, the container is interrupted only when it does.
set_digest example.org/alpha:stable "sha256:aaa" "sha256:aaa"
printf 'alpha-container\n' > "$STATE/running"
SJEL_TEST_FAIL_PULL="" run_refresh "$MOCK_BIN:/usr/bin:/bin"
[ "$rc" -eq 0 ] || fail "an unchanged digest must exit 0, got $rc"
grep -F 'docker pull example.org/alpha:stable' "$CALLS" >/dev/null || fail "the image was never pulled"
grep -F 'service-runner' "$CALLS" >/dev/null 2>&1 && fail "an unchanged digest must not recreate anything"
case "$(receipt_field skipped)" in
  *refresh-alpha:unchanged*) ;;
  *) cat "$RECEIPT"; fail "the receipt did not record the unchanged image" ;;
esac

# 4. A moved digest recreates the running container, and the receipt names it.
set_digest example.org/alpha:stable "sha256:aaa" "sha256:bbb"
SJEL_TEST_FAIL_PULL="" run_refresh "$MOCK_BIN:/usr/bin:/bin"
[ "$rc" -eq 0 ] || fail "a successful recreate must exit 0, got $rc"
grep -F 'service-runner recreate refresh-alpha' "$CALLS" >/dev/null || fail "a moved digest did not recreate the container"
case "$(receipt_field ran)" in
  *refresh-alpha*) ;;
  *) cat "$RECEIPT"; fail "the receipt did not name the refreshed capability" ;;
esac

# 5. A container that is not running is pulled and left alone. `recreate` would START it, which
#    overturns whoever stopped it.
set_digest example.org/alpha:stable "sha256:aaa" "sha256:bbb"
: > "$STATE/running"
SJEL_TEST_FAIL_PULL="" run_refresh "$MOCK_BIN:/usr/bin:/bin"
[ "$rc" -eq 0 ] || fail "a stopped capability must not fail the run, got $rc"
grep -F 'service-runner' "$CALLS" >/dev/null 2>&1 && fail "a stopped container must not be recreated"
case "$(receipt_field skipped)" in
  *refresh-alpha:not-running*) ;;
  *) cat "$RECEIPT"; fail "the receipt did not record the stopped container" ;;
esac

# 6. A maintenance hold survives the refresh. `recreate` deletes the lock and starts the service,
#    so the hold has to be checked here rather than relied on downstream.
set_digest example.org/alpha:stable "sha256:aaa" "sha256:bbb"
printf 'alpha-container\n' > "$STATE/running"
: > /tmp/axon-refresh-alpha.maintenance
SJEL_TEST_FAIL_PULL="" run_refresh "$MOCK_BIN:/usr/bin:/bin"
rm -f /tmp/axon-refresh-alpha.maintenance
[ "$rc" -eq 0 ] || fail "a held capability must not fail the run, got $rc"
grep -F 'service-runner' "$CALLS" >/dev/null 2>&1 && fail "a held capability must not be recreated"
case "$(receipt_field skipped)" in
  *refresh-alpha:held*) ;;
  *) cat "$RECEIPT"; fail "the receipt did not record the maintenance hold" ;;
esac

# 7. A failing pull does not stop the images after it, and the run exits 2 naming the step.
write_machine docker refresh-alpha refresh-beta
set_digest example.org/alpha:stable "sha256:aaa" "sha256:bbb"
set_digest example.org/beta:latest  "sha256:ccc" "sha256:ddd"
printf 'alpha-container\nbeta-container\n' > "$STATE/running"
SJEL_TEST_FAIL_PULL="example.org/alpha:stable" run_refresh "$MOCK_BIN:/usr/bin:/bin"
[ "$rc" -eq 2 ] || fail "a failed pull must exit 2, got $rc"
grep -F 'docker pull example.org/beta:latest' "$CALLS" >/dev/null || fail "the image after a failed pull was never pulled"
grep -F 'service-runner recreate refresh-beta' "$CALLS" >/dev/null || fail "the capability after a failed pull was not recreated"
case "$(receipt_field failed)" in
  *refresh-alpha:pull*) ;;
  *) cat "$RECEIPT"; fail "the receipt did not name the failed pull" ;;
esac

# 8. A container_runtime nobody implemented is a machine.toml defect, not something to guess at.
write_machine containerd refresh-alpha
SJEL_TEST_FAIL_PULL="" run_refresh "$MOCK_BIN:/usr/bin:/bin"
[ "$rc" -eq 2 ] || fail "an unsupported runtime must exit 2, got $rc"
case "$(receipt_field failed)" in
  *runtime:containerd*) ;;
  *) cat "$RECEIPT"; fail "the receipt did not name the unsupported runtime" ;;
esac

# 9. An overlay it cannot write to says so and does not fail the run — a stale receipt read as a
#    fresh one is the failure this message exists to prevent. (An UNSET overlay never reaches the
#    receipt: platform.sh refuses to load without one, which case 10 covers.)
write_machine docker refresh-alpha
set_digest example.org/alpha:stable "sha256:aaa" "sha256:aaa"
rm -rf "$OVERLAY/data"; : > "$OVERLAY/data"    # a file where the directory goes: mkdir -p fails
SJEL_TEST_FAIL_PULL="" run_refresh "$MOCK_BIN:/usr/bin:/bin"
[ "$rc" -eq 0 ] || fail "an unwritable overlay must not fail the refresh, got $rc"
grep -F 'cannot write' "$SCRATCH/out" >/dev/null || fail "a run with no receipt did not say so"
rm -f "$OVERLAY/data"

# 10. No machine.toml at all. platform.sh refuses, and the script stops rather than running on
#     with an unset runtime — the shape that would otherwise surface as a bare `set -u` error.
mv "$MACHINE" "$MACHINE.away"
SJEL_TEST_FAIL_PULL="" run_refresh "$MOCK_BIN:/usr/bin:/bin"
[ "$rc" -eq 2 ] || fail "an unresolvable machine.toml must exit 2, got $rc"
mv "$MACHINE.away" "$MACHINE"

echo "container-refresh guarding, digest gating, hold safety and receipt tests: pass"
