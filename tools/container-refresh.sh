#!/bin/bash
# tools/container-refresh.sh — pull every declared image, every day, and recreate what moved.
#
# The job capabilities/container-refresh runs (kind = "process", schedule = "24h"). It is the
# container half of capabilities/host-patch: Q77 (2026-09-02) made every service.toml `tag`
# a rolling channel, and a channel only moves a running house when something pulls it.
#
# Contract, in this order and no other:
#   1. every step is `command -v` guarded: a machine without docker is not a failed refresh
#   2. every failure is recorded and non-fatal to the next: one unreachable registry must not
#      leave the images after it on last week's build
#   3. the DIGEST decides, never the tag. `pull` on `:stable` is a no-op the registry answers
#      cheaply; the container is recreated only when the local digest for that reference
#      actually changed (ISA.md C4 — the declared tag is a channel, the digest is the fact)
#   4. a receipt is written whatever happened, because tools/doctor reads it — a scheduled job
#      that silently stopped firing is the failure mode a scheduled job actually has
#
# WHAT IT WILL NOT DO. It recreates a RUNNING container and nothing else. `recreate` clears the
# maintenance hold on its way through (tools/service-runner.sh:760) and `start_service` brings the
# capability up, so refreshing a held or deliberately stopped capability would undo an operator's
# decision to have it down. Those are recorded as skipped, by name, with the reason — the new
# image is already pulled, and the next `service-runner.sh recreate` applies it.
#
# NOT run here: any scan of what was pulled. grype reads the registry in
# .github/workflows/security.yml, which runs whether or not this host is awake.
#
# Exit 0 = every declared image is current, or there was nothing to do · 2 = a step failed.
# bash 3.2-safe.
set -u

_here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/lib/paths.sh
. "$_here/lib/paths.sh"      # SJEL_ROOT, SJEL_PERSONAL_ROOT, SJEL_MACHINE_TOML, axon_manifest_for
# shellcheck source=tools/lib/platform.sh
# `|| exit 2`, because platform.sh signals a missing overlay or machine.toml with `return 1` when
# sourced. Without that check the script would run on with SJEL_CONTAINER_RUNTIME unset and, under
# `set -u`, die on the first read of it with a message that names nothing useful. There is no
# receipt on this path and there cannot be: the receipt lives in the overlay this machine has not
# configured, and doctor's "has never written a receipt" is the report that covers it.
. "$_here/lib/platform.sh" || exit 2   # SJEL_CONTAINER_RUNTIME — this machine's declared runtime
# shellcheck source=tools/lib/pipe.sh
. "$_here/lib/pipe.sh"       # stream_matches — an exact-line test that does not SIGPIPE the producer

# Same defect, same fix as tools/host-patch.sh: launchd hands a job
# PATH=/usr/bin:/bin:/usr/sbin:/sbin, and no container CLI lives there. On this deployment the
# docker CLI is OrbStack's at ~/.orbstack/bin/docker, which a login shell has and a launchd
# environment does not — the same miss that kept service-runner's persistence defect invisible
# for two weeks (tools/service-runner.sh:50). SJEL_CONTAINER_REFRESH_KEEP_PATH=1 leaves the
# caller's PATH alone, for a host with its own layout and for tools/container-refresh.test.sh,
# which plants a PATH to prove what the script does with and without a runtime.
if [ -z "${SJEL_CONTAINER_REFRESH_KEEP_PATH:-}" ]; then
  PATH="$HOME/.orbstack/bin:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:$HOME/.local/bin:$PATH"
  export PATH
fi

RAN=""; SKIPPED=""; FAILED=""

# The receipt is written on every exit path, including the early ones. tools/doctor reads it and
# reports its age; a run that ended before doing anything still has to leave that trace, or a job
# that stopped firing looks the same as a job with nothing to do.
write_receipt() {  # write_receipt <exit-code>
  # An UNSET overlay cannot reach here — platform.sh above refused to load without one. An
  # UNWRITABLE one can, and it must not turn into a silent success: doctor would then read a stale
  # receipt and report a run that never happened.
  if mkdir -p "$SJEL_PERSONAL_ROOT/data/container-refresh" 2>/dev/null; then
    printf '{"at":"%s","ran":"%s","skipped":"%s","failed":"%s"}\n' \
      "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$(echo $RAN)" "$(echo $SKIPPED)" "$(echo $FAILED)" \
      > "$SJEL_PERSONAL_ROOT/data/container-refresh/last.json"
  else
    echo "container-refresh: cannot write $SJEL_PERSONAL_ROOT/data/container-refresh/last.json — tools/doctor cannot report this run" >&2
  fi
  echo
  echo "── container-refresh: refreshed$RAN ──"
  [ -n "$SKIPPED" ] && echo "── skipped:$SKIPPED ──"
  [ -n "$FAILED" ]  && echo "── FAILED:$FAILED ──"
  exit "$1"
}

echo "Axon container-refresh · $(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u)"

# Which capabilities this machine runs, and which of those are containers. Both reads go through
# tools/lib/toml.sh: machine.toml owns the enabled set (tools/capability.sh is its only writer),
# and each service.toml owns whether it declares an image at all.
CONTAINER_CAPS=""
for _cap in $(toml_array capabilities "$SJEL_MACHINE_TOML" 2>/dev/null); do
  _mf="$(axon_manifest_for "$_cap" 2>/dev/null)" || continue
  [ -n "$_mf" ] || continue
  [ -n "$(toml_get image "$_mf")" ] || continue
  CONTAINER_CAPS="$CONTAINER_CAPS $_cap"
done

# A host with no container capabilities is the normal case for a workstation, not a fault. It
# writes the receipt saying so, so doctor can tell "nothing to refresh" from "never ran".
if [ -z "$CONTAINER_CAPS" ]; then
  echo "· no enabled capability declares an image — nothing to refresh"
  SKIPPED="no-container-capabilities"
  write_receipt 0
fi

case "$SJEL_CONTAINER_RUNTIME" in
  docker|podman) RUNTIME_BIN="$SJEL_CONTAINER_RUNTIME" ;;
  *)
    # Same refusal as tools/service-runner.sh:58, and for the same reason: a runtime nobody
    # implemented is a machine.toml defect, not something to resolve by guessing.
    echo "container-refresh: unsupported container_runtime '$SJEL_CONTAINER_RUNTIME' ($SJEL_MACHINE_TOML)" >&2
    FAILED=" runtime:$SJEL_CONTAINER_RUNTIME"
    write_receipt 2
    ;;
esac

if ! command -v "$RUNTIME_BIN" >/dev/null 2>&1; then
  echo "· $RUNTIME_BIN not installed, skipped — nothing pulled"
  SKIPPED=" $RUNTIME_BIN-not-installed"
  write_receipt 0
fi

# The local digest for a reference, or empty when the runtime has never pulled it (and when the
# image was built locally, which has no RepoDigests at all). Empty on both sides means the pull
# told us nothing, so nothing is recreated on that basis.
image_digest() {  # image_digest <image:tag>
  "$RUNTIME_BIN" image inspect "$1" --format '{{index .RepoDigests 0}}' 2>/dev/null || true
}

# Exact-line match through the shared helper, never `grep -q`: -q exits at the first hit and
# the producer dies of SIGPIPE (tools/lib/pipe.sh). Same call as tools/service-runner.sh:838,
# so "running" means the same thing in the receipt as it does in `status`.
container_running() {  # container_running <container-name>
  "$RUNTIME_BIN" ps --format '{{.Names}}' 2>/dev/null | stream_matches -Fx "$1"
}

for cap in $CONTAINER_CAPS; do
  mf="$(axon_manifest_for "$cap" 2>/dev/null)" || continue
  image="$(toml_get image "$mf")"
  tag="$(toml_get tag "$mf")"
  name="$(toml_get name "$mf")"
  ref="$image:$tag"

  echo "▸ $cap — $ref"
  before="$(image_digest "$ref")"
  if ! "$RUNTIME_BIN" pull "$ref"; then
    echo "  ✗ $cap: pull failed — continuing" >&2
    FAILED="$FAILED $cap:pull"
    continue
  fi
  after="$(image_digest "$ref")"

  if [ "$before" = "$after" ]; then
    echo "  · unchanged ${after:-(no digest reported)}"
    SKIPPED="$SKIPPED $cap:unchanged"
    continue
  fi

  echo "  · moved: ${before:-(none)} -> ${after:-(none)}"
  # The hold is the operator's, and recreate would clear it. Checked before `ps` because a held
  # capability is usually a stopped one and "held" is the more useful word in the receipt.
  if [ -f "/tmp/axon-$cap.maintenance" ]; then
    echo "  · '$cap' is held for maintenance — pulled, not recreated"
    SKIPPED="$SKIPPED $cap:held"
    continue
  fi
  if ! container_running "$name"; then
    echo "  · '$name' is not running — pulled, not recreated"
    SKIPPED="$SKIPPED $cap:not-running"
    continue
  fi
  if "$SJEL_ROOT/tools/service-runner.sh" recreate "$cap"; then
    RAN="$RAN $cap"
  else
    rc=$?
    echo "  ✗ $cap: recreate failed (exit $rc) — continuing" >&2
    FAILED="$FAILED $cap:recreate"
  fi
done

[ -n "$FAILED" ] && write_receipt 2
write_receipt 0
