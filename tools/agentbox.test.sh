#!/bin/bash
# Tests for capabilities/agentbox/agentbox — the runtime gate, and the two network modes it
# ships since Q75 (2026-09-02).
#
# The capability had no test file at all until then. It was defended by a table in its README,
# which is a record of one afternoon and not a check: the isolation claim moved from
# apple-container's host-only network to docker's `--network none`, and nothing would have
# reported it if the new mode quietly reached the internet.
#
# Two halves. The first runs anywhere: a scratch overlay declaring a runtime agentbox does not
# accept must be refused BY NAME, before any container call. The second needs a live docker
# daemon and is skipped loudly without one, matching service-runner.test.sh's bun preamble.
set -uo pipefail

fails=0
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }

_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
SRC_ROOT=""
for _c in "$_dir/.." "$_dir"; do
  if [ -f "$_c/capabilities/agentbox/agentbox" ]; then SRC_ROOT="$(cd "$_c" && pwd)"; break; fi
done
[ -n "$SRC_ROOT" ] || { echo "agentbox: cannot find capabilities/agentbox/agentbox from $_dir" >&2; exit 1; }

SCRATCH="$(mktemp -d "/tmp/agentbox.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT
ROOT="$SCRATCH/axon"
OVERLAY="$SCRATCH/overlay"

# An operator's exported SJEL_OVERLAY_ROOT / SJEL_MACHINE_TOML wins over the scratch axon.toml,
# so without this the launcher reads the REAL machine and the fixture is inert
# (tools/lib/test-support.sh#isolate_axon_env).
source "$SRC_ROOT/tools/lib/test-support.sh"
isolate_axon_env

mkdir -p "$ROOT/tools/lib" "$ROOT/capabilities/agentbox" "$OVERLAY/config"
cp "$SRC_ROOT"/tools/lib/*.sh "$ROOT/tools/lib/"
cp "$SRC_ROOT/capabilities/agentbox/agentbox" "$ROOT/capabilities/agentbox/"
cp -R "$SRC_ROOT/capabilities/agentbox/profiles" "$ROOT/capabilities/agentbox/"
printf 'overlay = "%s"\n' "$OVERLAY" > "$ROOT/axon.toml"
printf 'agent = "pi"\nmodel_port = "8000"\n' > "$OVERLAY/config/agentbox.toml"

BOX="$ROOT/capabilities/agentbox/agentbox"

# --- 1. the runtime gate ---------------------------------------------------
# podman is the interesting rejection: its verbs match docker's closely enough that the box
# would half-work, and it spells the host `host.containers.internal`, so model_base_url would
# render an endpoint that never resolves. Refused by name beats accepted untested.
for runtime in podman apple-container ""; do
  printf 'os = "macos"\ncontainer_runtime = "%s"\ncapabilities = []\n' "$runtime" \
    > "$OVERLAY/config/machine.toml"
  out="$("$BOX" doctor 2>&1)"; rc=$?
  [ "$rc" -eq 0 ] && fail "container_runtime = '$runtime' was accepted (rc=0)"
  case "$out" in
    *"agentbox needs docker"*) ;;
    *"missing 'os' or 'container_runtime'"*) ;;   # the empty case dies one layer earlier, in platform.sh
    *) fail "container_runtime = '$runtime' was refused without naming what agentbox needs: $out" ;;
  esac
  # A doctor that cannot use the declared runtime must not then answer runtime questions from
  # whatever daemon happens to be installed. Every such line reads "not checked".
  #
  # Asserted on every arch since Q77 (2026-09-02). profile.toml carried a per-os/arch
  # checksum literal until then, and require_config died right after the runtime line on any
  # arch the profile had no line for — so this half went unasserted on an x86_64 runner. The
  # profile holds no checksum now (the release's own SHA256SUMS is read at build time), so
  # nothing here is arch-bound.
  case "$runtime" in
    podman|apple-container)
      case "$out" in
        *"image: not checked"*) ;;
        *) fail "container_runtime = '$runtime' still had doctor query a runtime it does not declare: $out" ;;
      esac ;;
  esac
done

# --- 1b. the profile declares a channel, never a version -------------------
# The launcher resolves the latest release at build time (Q77). A version or checksum
# literal reappearing in the profile would silently restore the freeze the ruling removed,
# and nothing else in this file would notice.
PROFILE="$SRC_ROOT/capabilities/agentbox/profiles/pi/profile.toml"
for forbidden in '^version *=' '^archive_sha256'; do
  if grep -Eq "$forbidden" "$PROFILE"; then
    fail "profile.toml declares '$forbidden' — the release is resolved at build time, not recorded here"
  fi
done
for required in '^release_latest_api *=' '^checksums_url *='; do
  grep -Eq "$required" "$PROFILE" || fail "profile.toml is missing '$required' — nothing would say which release to take, or how to verify it"
done

# --- 1c. host-install, end to end against a planted release ----------------
#
# The release-resolution path (resolve_release -> fetch_archive -> cmd_host_install) was
# rewritten on 2026-09-02 (Q77) to take the LATEST release and its published SHA256SUMS
# instead of two literals in profile.toml, and nothing here drove any of it. What that cost:
# the ordinary second `host-install` — the one where the resolved version is already on disk —
# died on `ARCHIVE_SHA: unbound variable` AFTER relinking `current` and `bin/pi`, so the verb
# half-succeeded and wrote no receipt. A grep for the deleted literals cannot see that.
#
# Driven through a planted PATH: `curl` is a stub serving one fake release, so these checks
# need no network and no real upstream tag. jq, tar and shasum stay real — they are the same
# programs the launcher uses on a live install, and stubbing them would only prove the stubs.
command -v jq >/dev/null 2>&1 || { echo "agentbox: jq not on PATH — cannot run (toolchain.toml [jq])" >&2; exit 1; }

HI="$SCRATCH/host-install"
mkdir -p "$HI/bin" "$HI/release/pi"
printf 'os = "macos"\ncontainer_runtime = "docker"\ncapabilities = []\n' > "$OVERLAY/config/machine.toml"
# host-install names the advisory list from upstreams.toml via profile.toml's `upstream`.
printf '[pi-coding-agent]\nurl = "https://github.com/earendil-works/pi"\n' > "$ROOT/upstreams.toml"

# The fake release: a tarball shaped like the real one (archive_root/bin_path from profile.toml)
# and its own SHA256SUMS, so the asset name the launcher looks up is the one it downloads.
printf '#!/bin/sh\necho fixture agent\n' > "$HI/release/pi/pi"
chmod 755 "$HI/release/pi/pi"
tar czf "$HI/release/asset.tar.gz" -C "$HI/release" pi
GOOD_SHA="$(shasum -a 256 "$HI/release/asset.tar.gz" | cut -d' ' -f1)"
case "$(uname -s)" in Darwin) HOST_OS="darwin" ;; *) HOST_OS="linux" ;; esac
case "$(uname -m)" in arm64|aarch64) HOST_ARCH="arm64" ;; *) HOST_ARCH="x64" ;; esac
ASSET="pi-$HOST_OS-$HOST_ARCH.tar.gz"

# State the stub reads at call time, so a case below changes what "the release" says without
# rewriting the stub: TAG, the SHA256SUMS body, and nothing else.
printf 'v9.9.9\n' > "$HI/tag"
printf '%s  %s\n' "$GOOD_SHA" "$ASSET" > "$HI/SHA256SUMS"

cat > "$HI/bin/curl" <<CURL
#!/bin/bash
# Serves the planted release. Argument parsing matches what agentbox actually passes:
# flags, an optional -o <file>, one -H <header>, and the URL last.
out=""; url=""
while [ \$# -gt 0 ]; do
  case "\$1" in
    -o) out="\$2"; shift 2 ;;
    -H) shift 2 ;;
    -*) shift ;;
    *) url="\$1"; shift ;;
  esac
done
case "\$url" in
  *releases/latest)
    tag="\$(cat "$HI/tag")"
    [ -n "\$tag" ] || { printf '{}\n'; exit 0; }
    printf '{"tag_name":"%s"}\n' "\$tag" ;;
  *SHA256SUMS) cat "$HI/SHA256SUMS" ;;
  *.tar.gz)
    [ -n "\$out" ] || exit 1
    cp "$HI/release/asset.tar.gz" "\$out" ;;
  *) exit 22 ;;
esac
CURL
chmod 755 "$HI/bin/curl"

HOST_ROOT="$OVERLAY/data/agentbox/host"
RECEIPT="$HOST_ROOT/installed.json"
receipt_field() { jq -r ".$1 // empty" "$RECEIPT" 2>/dev/null; }

_saved_path="$PATH"
PATH="$HI/bin:$PATH"

# The first install. The version comes from the stub's tag_name and from nowhere else — a
# profile literal reappearing would make this read 0.83.0 instead.
out="$("$BOX" host-install 2>&1)"; rc=$?
[ "$rc" -eq 0 ] || fail "host-install failed on a clean host (rc=$rc): $out"
case "$out" in
  *"latest pi release is 9.9.9"*) ;;
  *) fail "host-install did not resolve the version from the release API: $out" ;;
esac
[ -x "$HOST_ROOT/versions/pi-9.9.9/pi" ] || fail "host-install left no executable at versions/pi-9.9.9/pi"
[ "$(readlink "$HOST_ROOT/bin/pi")" = "$HOST_ROOT/current/pi" ] \
  || fail "bin/pi is not a plain symlink through current (the self-update shim is gone since Q77)"
[ "$(receipt_field version)" = "9.9.9" ] || fail "receipt records version '$(receipt_field version)', not the resolved 9.9.9"
[ "$(receipt_field sha256)" = "$GOOD_SHA" ] || fail "receipt records sha256 '$(receipt_field sha256)', not the archive's $GOOD_SHA"
[ "$(receipt_field verified)" = "this-run" ] || fail "the install path recorded verified='$(receipt_field verified)', not this-run"

# The regression this section exists for: the SAME command again, with the release already on
# disk. It must exit 0 and it must still write the receipt the README promises.
out="$("$BOX" host-install 2>&1)"; rc=$?
[ "$rc" -eq 0 ] || fail "a second host-install of an already-installed release failed (rc=$rc): $out"
case "$out" in
  *"already installed"*) ;;
  *) fail "the second host-install re-fetched instead of recognising the installed release: $out" ;;
esac
[ "$(receipt_field verified)" = "earlier-run" ] \
  || fail "the already-installed path recorded verified='$(receipt_field verified)', not earlier-run — it hashes nothing, and saying otherwise claims a check that did not run"
[ "$(receipt_field sha256)" = "$GOOD_SHA" ] || fail "the already-installed path lost the digest the earlier run verified"

# No receipt to carry a digest forward from — an install that predates this field, or a deleted
# receipt. Unknown is recorded as unknown rather than as a digest nobody checked.
rm -f "$RECEIPT"
out="$("$BOX" host-install 2>&1)"; rc=$?
[ "$rc" -eq 0 ] || fail "host-install failed with the receipt missing (rc=$rc): $out"
[ "$(receipt_field verified)" = "unknown" ] || fail "with no prior receipt the digest is unknown, but verified='$(receipt_field verified)'"
[ -z "$(receipt_field sha256)" ] || fail "with no prior receipt the receipt still printed a sha256: $(receipt_field sha256)"

# --force re-fetches and re-verifies, which is the whole reason it exists.
out="$("$BOX" host-install --force 2>&1)"; rc=$?
[ "$rc" -eq 0 ] || fail "host-install --force failed (rc=$rc): $out"
[ "$(receipt_field verified)" = "this-run" ] || fail "--force recorded verified='$(receipt_field verified)' without re-hashing"

# A moved release under the same tag. The archive is cached per version, so this is the case
# the fetch-fresh SHA256SUMS is there for: the manifest changed, the bytes did not.
printf '%s  %s\n' "0000000000000000000000000000000000000000000000000000000000000000" "$ASSET" > "$HI/SHA256SUMS"
out="$("$BOX" host-install --force 2>&1)"; rc=$?
[ "$rc" -ne 0 ] || fail "host-install accepted an archive whose sha256 does not match the release's SHA256SUMS"
case "$out" in
  *"checksum mismatch"*) ;;
  *) fail "the checksum refusal did not say what failed: $out" ;;
esac

# A release that publishes no checksum for this os/arch. Refused by name, never installed
# unverified — the property the deleted archive_sha256_* literals used to hold.
printf '%s  pi-someotheros-somearch.tar.gz\n' "$GOOD_SHA" > "$HI/SHA256SUMS"
out="$("$BOX" host-install --force 2>&1)"; rc=$?
[ "$rc" -ne 0 ] || fail "host-install accepted a release that lists no checksum line for $ASSET"
case "$out" in
  *"lists no line for"*) ;;
  *) fail "the missing-checksum refusal did not name the asset: $out" ;;
esac

# No tag to resolve — a rate-limited API, or a repository that publishes no release. With no
# literal left in the profile there is no fallback, so this has to stop rather than guess.
printf '%s  %s\n' "$GOOD_SHA" "$ASSET" > "$HI/SHA256SUMS"
: > "$HI/tag"
out="$("$BOX" host-install 2>&1)"; rc=$?
[ "$rc" -ne 0 ] || fail "host-install continued after the release API returned no tag_name"
case "$out" in
  *"no tag_name"*) ;;
  *) fail "the unresolved-release refusal did not name the cause: $out" ;;
esac

PATH="$_saved_path"

# --- 2. the two network modes ----------------------------------------------
HAVE_DOCKER=1
command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1 || HAVE_DOCKER=0

if [ "$HAVE_DOCKER" -eq 0 ]; then
  echo "  ⊘ no answering docker daemon — the network-mode checks are skipped"
else
  # A stand-in for the agentbox image: this asserts what the FLAGS do, and building the real
  # image needs a 100 MB agent tarball off the network. The base image is READ from the
  # Containerfile rather than named here: one reference to the dependency, so a base change is
  # a one-line edit that this test follows instead of contradicting. It also has to be glibc:
  # resolver behaviour on `--network none` differs (glibc fails in ~2 ms, musl burns a 10 s
  # timeout).
  PROBE_IMAGE="$(sed -n 's/^ARG BASE_IMAGE=//p' "$SRC_ROOT/capabilities/agentbox/Containerfile" | head -n1)"
  [ -n "$PROBE_IMAGE" ] || fail "could not read ARG BASE_IMAGE from the agentbox Containerfile"
  if ! docker image inspect "$PROBE_IMAGE" >/dev/null 2>&1; then
    docker pull -q "$PROBE_IMAGE" >/dev/null 2>&1 \
      || { echo "  ⊘ cannot obtain $PROBE_IMAGE — the network-mode checks are skipped"; PROBE_IMAGE=""; }
  fi

  # The flags come out of the launcher, not out of this file. Both modes are single
  # assignments there for exactly this reason, so an edit to either one is an edit to what
  # these checks prove; a copy here would go on passing after the box changed modes.
  # Extraction failing is a failure, never a silent skip.
  MODEL_HOST="$(sed -n 's/^MODEL_HOST="\(.*\)"$/\1/p' "$BOX" | head -n1)"
  OFFLINE_NET_ARGS="$(sed -n 's/^OFFLINE_NET_ARGS="\(.*\)"$/\1/p' "$BOX" | head -n1)"
  ONLINE_NET_ARGS="$(sed -n 's/^ONLINE_NET_ARGS="\(.*\)"$/\1/p' "$BOX" | head -n1)"
  ONLINE_NET_ARGS="${ONLINE_NET_ARGS//\$MODEL_HOST/$MODEL_HOST}"
  if [ -z "$MODEL_HOST" ] || [ -z "$OFFLINE_NET_ARGS" ] || [ -z "$ONLINE_NET_ARGS" ]; then
    fail "could not read MODEL_HOST / OFFLINE_NET_ARGS / ONLINE_NET_ARGS from $BOX — the network checks below would assert a copy"
    PROBE_IMAGE=""
  fi

  if [ -n "$PROBE_IMAGE" ]; then
    # Every run is named, so the leftover check below can filter on containers THIS file
    # created. An `ancestor=` filter would also catch a container the operator happens to have
    # built from the same base image.
    NAME_PREFIX="agentbox-test-$$"
    run_probe() {  # <suffix> <net args> <bash -c script>
      # shellcheck disable=SC2086  # $2 is a flag list and has to split
      docker run --rm --name "$NAME_PREFIX-$1" $2 --entrypoint /bin/bash "$PROBE_IMAGE" -c "$3"
    }

    # The offline mode is the whole security claim. Each of these three is a way it could be
    # false while the box still looked closed.
    out="$(run_probe route "$OFFLINE_NET_ARGS" 'cat /proc/net/route' 2>&1 | tail -n +2)"
    [ -z "$out" ] || fail "'$OFFLINE_NET_ARGS' left a route in the box: $out"

    run_probe tcp "$OFFLINE_NET_ARGS" 'exec 3<>/dev/tcp/1.1.1.1/443' >/dev/null 2>&1 \
      && fail "'$OFFLINE_NET_ARGS' reached 1.1.1.1:443"

    run_probe dns "$OFFLINE_NET_ARGS" "getent hosts $MODEL_HOST" >/dev/null 2>&1 \
      && fail "'$OFFLINE_NET_ARGS' resolved $MODEL_HOST — the host is supposed to be gone too"

    # The control. Without it, an image too broken to open any socket would pass every check
    # above and the block would read as isolation when it is a bug.
    run_probe control "$ONLINE_NET_ARGS" "getent hosts $MODEL_HOST" >/dev/null 2>&1 \
      || fail "the --online control could not resolve $MODEL_HOST — the probe proves nothing"

    # `--rm` on every run is what keeps the box disposable. A leftover would be a writable
    # layer surviving a session, which is the property `--rm` is there for.
    leftover="$(docker ps -a --filter "name=^$NAME_PREFIX-" --format '{{.Names}}')"
    [ -z "$leftover" ] || fail "a probe container outlived its run: $leftover"
  fi
fi

if [ "$fails" -eq 0 ]; then
  echo "agentbox: all checks passed"
else
  echo "agentbox: $fails check(s) failed"
  exit 1
fi
