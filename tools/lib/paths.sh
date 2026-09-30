#!/bin/bash
# Source this from any Axon/capability script instead of hardcoding paths.
# Exports SJEL_ROOT and SJEL_OVERLAY_ROOT, resolved dynamically so nothing
# breaks if either repo gets moved/renamed — set the overlay once, everything
# downstream picks it up automatically.
#
# The overlay location is read from axon.local.toml (gitignored, one per machine,
# written by tools/install.sh) and falls back to axon.toml's shipped default. That
# order is what lets a second machine exist without editing a tracked file, and the
# fallback is what keeps a fresh clone working — CI has no axon.local.toml and
# resolves the shipped default instead.

# Self-locate this file's dir. Under bash, BASH_SOURCE[0] is the sourced path;
# under zsh (e.g. sourced from ~/.zshrc) BASH_SOURCE is unset, so fall back to
# $0, which zsh sets to the sourced file's path. POSIX ${:-} keeps this bash 3.2-safe.
_lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
SJEL_ROOT="$(cd "$_lib/../.." && pwd)"
export SJEL_ROOT
source "$_lib/toml.sh"

_overlay_raw=""
# An already-exported SJEL_OVERLAY_ROOT wins over both files. This is the only way to point a
# single shell at a different deployment without editing something — axon.local.toml is the
# machine's answer to "which overlay am I", and a tool that rewrote it to borrow another one
# would leave the machine pointing somewhere else if it died halfway. tools/demo-up is the
# first caller: it runs the published demonstration against the tracked demo/overlay while the
# operator's real overlay stays exactly where it was. Deliberately not documented as a general
# configuration mechanism — it is a per-invocation override, and anything permanent belongs in
# axon.local.toml where tools/doctor can see it.
_overlay_overridden=""
if [ -n "${SJEL_OVERLAY_ROOT:-}" ]; then
  _overlay_raw="$SJEL_OVERLAY_ROOT"
  _overlay_overridden="yes"
fi
if [ -z "$_overlay_raw" ] && [ -f "$SJEL_ROOT/axon.local.toml" ]; then
  _overlay_raw="$(toml_get overlay "$SJEL_ROOT/axon.local.toml")"
fi
if [ -z "$_overlay_raw" ]; then
  _overlay_raw="$(toml_get overlay "$SJEL_ROOT/axon.toml")"
fi
if [ -z "$_overlay_raw" ]; then
  echo "paths.sh: no 'overlay' in $SJEL_ROOT/axon.local.toml or $SJEL_ROOT/axon.toml — run tools/install.sh" >&2
  return 1 2>/dev/null || exit 1
fi
# SJEL_OVERLAY_ROOT is the canonical active deployment overlay. Keep the historical
# SJEL_PERSONAL_ROOT export as a compatibility alias for existing private scripts.
SJEL_OVERLAY_ROOT="${_overlay_raw/#\~/$HOME}"
SJEL_PERSONAL_ROOT="$SJEL_OVERLAY_ROOT"
export SJEL_OVERLAY_ROOT SJEL_PERSONAL_ROOT
# This machine's own facts (os, container_runtime, capabilities, state mounts) live in
# one file inside the overlay. An overlay describes a deployment, and a deployment may
# own several machines, so the file is selected rather than assumed. Three ways in,
# most specific first:
#
#   1. `machine = "<name>"` in axon.local.toml — that file is gitignored and per-machine
#      already, which makes it the honest place to say which machine this is. It also
#      lets a host carry a role name ("service-node") instead of whatever DNS calls it.
#   2. <overlay>/config/machines/<short-hostname>.toml — zero config for the common case.
#   3. <overlay>/config/machine.toml — the original single-machine layout, still valid.
#      An overlay that never grows a second machine never has to change.
#
# Machine names are private facts: they live in the overlay and in a gitignored file, and
# no generator that writes a tracked artifact reads them.
SJEL_MACHINES_DIR="$SJEL_OVERLAY_ROOT/config/machines"
_machine_name=""
# Skipped when the overlay was overridden above: `machine` names a machine INSIDE the
# overlay axon.local.toml points at, so carrying it across to a borrowed overlay asks for a
# machines/<name>.toml that has no reason to exist there and fails the lookup outright.
if [ -z "$_overlay_overridden" ] && [ -f "$SJEL_ROOT/axon.local.toml" ]; then
  _machine_name="$(toml_get machine "$SJEL_ROOT/axon.local.toml")"
fi
if [ -n "$_machine_name" ]; then
  SJEL_MACHINE_TOML="$SJEL_MACHINES_DIR/$_machine_name.toml"
  if [ ! -f "$SJEL_MACHINE_TOML" ]; then
    echo "paths.sh: axon.local.toml names machine '$_machine_name', but $SJEL_MACHINE_TOML does not exist" >&2
    return 1 2>/dev/null || exit 1
  fi
else
  _host="$(hostname -s 2>/dev/null || hostname 2>/dev/null || echo "")"
  if [ -n "$_host" ] && [ -f "$SJEL_MACHINES_DIR/$_host.toml" ]; then
    SJEL_MACHINE_TOML="$SJEL_MACHINES_DIR/$_host.toml"
  else
    SJEL_MACHINE_TOML="$SJEL_OVERLAY_ROOT/config/machine.toml"
  fi
fi
export SJEL_MACHINE_TOML SJEL_MACHINES_DIR
unset _machine_name _host

# Capability manifests resolve from two roots. Public Axon holds reusable capabilities;
# the active overlay holds deployment-specific ones, which is what keeps private services
# out of a repository meant for publication. Overlay capabilities are runtime-visible
# only: generators that write tracked artifacts (tools/self.ts, generate-architecture.sh)
# deliberately scan SJEL_CAPS_DIR alone, because a capability name is itself a fact about
# a private deployment.
SJEL_CAPS_DIR="$SJEL_ROOT/capabilities"
SJEL_OVERLAY_CAPS_DIR="$SJEL_OVERLAY_ROOT/capabilities"
export SJEL_CAPS_DIR SJEL_OVERLAY_CAPS_DIR

# <name> -> its service.toml path on stdout. Exit 1 when the name has no manifest,
# exit 2 when both roots declare it. A duplicate is refused rather than resolved by
# path order: silently starting whichever root sorted first is a worse failure than
# stopping, because the two manifests are different services wearing one name.
axon_manifest_for() {
  local name="$1" root_mf="" overlay_mf=""
  if [ -f "$SJEL_CAPS_DIR/$name/service.toml" ]; then
    root_mf="$SJEL_CAPS_DIR/$name/service.toml"
  fi
  if [ -f "$SJEL_OVERLAY_CAPS_DIR/$name/service.toml" ]; then
    overlay_mf="$SJEL_OVERLAY_CAPS_DIR/$name/service.toml"
  fi
  if [ -n "$root_mf" ] && [ -n "$overlay_mf" ]; then
    echo "paths.sh: capability '$name' is declared in both roots:" >&2
    echo "  $root_mf" >&2
    echo "  $overlay_mf" >&2
    echo "Rename one — they are two different services sharing a name." >&2
    return 2
  fi
  if [ -n "$root_mf" ]; then echo "$root_mf"; return 0; fi
  if [ -n "$overlay_mf" ]; then echo "$overlay_mf"; return 0; fi
  # A spine component carries its manifest at the repo root instead (today: dashboard/),
  # because it is not a capability and never appears in machine.toml's enabled set.
  if [ -f "$SJEL_ROOT/$name/service.toml" ]; then
    echo "$SJEL_ROOT/$name/service.toml"
    return 0
  fi
  return 1
}

# <tool> -> the path this machine declares for that state mount, on stdout, with a
# leading ~ expanded. Exit 1 when the machine declares no mount for the tool, exit 2
# when it declares more than one. Ambiguity is refused rather than resolved by order,
# for the same reason axon_manifest_for refuses a duplicate name.
#
# machine.toml's [[state_mount]] is array-of-tables, past toml.sh's single-line
# contract (see its header), so this reads it through Bun.TOML like every other
# array-of-tables caller. That keeps the manifest the one source of truth instead of
# letting each script carry its own fallback path — the failure mode being that the
# two agree on one installation and silently disagree on the next.
axon_state_mount_for() {
  local tool="$1" out=""
  [ -f "$SJEL_MACHINE_TOML" ] || {
    echo "paths.sh: no machine manifest at $SJEL_MACHINE_TOML" >&2
    return 1
  }
  # Inputs go through the environment, not argv: `bun -e` treats trailing arguments
  # as further scripts to run, so a positional path is opened as a file and fails.
  out="$(_SJEL_MOUNT_FILE="$SJEL_MACHINE_TOML" _SJEL_MOUNT_TOOL="$tool" bun -e '
    const file = process.env._SJEL_MOUNT_FILE, tool = process.env._SJEL_MOUNT_TOOL;
    const mounts = Bun.TOML.parse(await Bun.file(file).text()).state_mount ?? [];
    const hits = mounts.filter((m) => m?.tool === tool);
    if (hits.length > 1) process.exit(2);
    if (hits.length === 0 || !hits[0].path) process.exit(1);
    console.log(hits[0].path);
  ' 2>/dev/null)" || {
    local rc=$?
    if [ "$rc" = "2" ]; then
      echo "paths.sh: $SJEL_MACHINE_TOML declares more than one [[state_mount]] for '$tool'" >&2
      return 2
    fi
    echo "paths.sh: $SJEL_MACHINE_TOML declares no [[state_mount]] for '$tool'" >&2
    return 1
  }
  echo "${out/#\~/$HOME}"
}

unset _overlay_raw _lib
