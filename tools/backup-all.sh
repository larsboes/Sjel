#!/usr/bin/env bash
# Back up every capability that declares a backup contract.
#
# Exists because the schedule that ran backups was a hand-written LaunchAgent naming ONE
# capability — `backup.sh store` — while three declare a contract. tools/doctor called that unit an
# orphan (no manifest owned it), and it was right twice over: nothing versioned it, and nothing
# would have noticed when a fourth capability declared a contract and was never backed up.
#
# The set is DERIVED, never typed. A capability is in scope because its manifest declares
# `backup_target`, which is the same field tools/backup.sh already refuses to run without and the
# same one sjel-status reads to decide a row belongs in its registry. One definition, three
# readers.
#
# Runs every contract even when one fails, and exits non-zero if any did. Stopping at the first
# failure would let one broken capability silently cancel the backups of the others, which is the
# shape of outage that ends with two weeks of nothing.
set -uo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$TOOLS_DIR/lib/paths.sh"

# Three modes, one derivation. `--list` and `--targets-json` exist for
# capabilities/backup, which drives one run per capability and needs to know
# which capabilities declare what. The set is derived HERE and read there, so
# there is still exactly one definition of "which capabilities are backed up".
MODE="run"
case "${1:-}" in
  --list|--targets-json) MODE="$1" ;;
  "") ;;
  *)
    echo "usage: backup-all.sh [--list | --targets-json]" >&2
    exit 2
    ;;
esac
[ "$#" -le 1 ] || { echo "usage: backup-all.sh [--list | --targets-json]" >&2; exit 2; }

# `scope` filters out a capability this machine only consumes: its data lives on the deployment
# that provides it, and so does the authority to back it up (tools/backup.sh says so itself and
# exits 1). Asking anyway would turn a correct refusal into a failed run every night.
# `while read`, not `mapfile`: macOS ships bash 3.2 and this repo stays 3.2-safe, which
# tools/backup.sh states at its own head. mapfile is a bash 4 builtin and fails with
# "command not found" — quietly enough that the loop below would simply have run zero times.
# Resolve bun BEFORE deriving anything, and fail by name when it is missing.
#
# Why (2026-09-06): this reported success while backing up nothing for 8.3 days. launchd hands a
# supervised job a minimal environment whose PATH has no /opt/homebrew/bin, `bun` was not found,
# the derivation below printed nothing, and the empty-set branch read that as "no capability
# declares a contract" and exited 0. Three backup contracts went unrun and every signal said
# fine. It is the same launchd-PATH defect service-runner.sh documents at resolve_runtime — and
# the same fix: name the missing binary instead of letting bash's `command not found` vanish
# into a log nobody reads.
#
# SJEL_BUN is how a machine whose supervisor cannot see the login PATH states the real path,
# declared in the overlay's machine.toml under `[capability.backup] env` — the seam
# persistence_env_block exists for, rather than a hand-edit of the generated unit.
BUN="${SJEL_BUN:-bun}"
case "$BUN" in
  /*)
    [ -x "$BUN" ] || {
      echo "backup-all.sh: SJEL_BUN='$BUN' is not an executable file" >&2
      exit 1
    }
    ;;
  *)
    command -v "$BUN" >/dev/null 2>&1 || {
      echo "backup-all.sh: '$BUN' not found on PATH (PATH=$PATH)." >&2
      echo "backup-all.sh: set SJEL_BUN to its absolute path in the overlay's machine.toml, [capability.backup] env." >&2
      exit 1
    }
    ;;
esac

# Derived through a file rather than a process substitution, because `< <(...)` throws the
# pipeline's exit status away: a registry that failed and a machine with no contracts both
# arrive here as zero lines. They mean opposite things, so they must not share a branch.
DERIVED="$(mktemp -t axon-backup-caps)" || exit 1
trap 'rm -f "$DERIVED"' EXIT
if ! "$TOOLS_DIR/capability.sh" registry 2>/dev/null \
    | "$BUN" -e '
        const rows = JSON.parse(require("fs").readFileSync(0, "utf8"));
        for (const r of rows) {
          if (r.scope === "external") continue;
          if (!r.backup_target) continue;
          console.log(r.name + "\t" + r.backup_target);
        }
      ' > "$DERIVED"; then
  echo "backup-all.sh: could not derive the backup set from the capability registry — refusing to report success" >&2
  exit 1
fi

CAPS=()
TARGETS=()
SEEN_TARGETS=""
while IFS="$(printf '\t')" read -r cap target; do
  [ -n "$cap" ] || continue
  CAPS+=("$cap")
  # Dedupe without associative arrays: bash 3.2 is the floor here.
  case " $SEEN_TARGETS " in
    *" $target "*) ;;
    *) SEEN_TARGETS="${SEEN_TARGETS:+$SEEN_TARGETS }$target"; TARGETS+=("$target") ;;
  esac
done < "$DERIVED"

# `--list` answers before the empty-set branch below: a machine with no contracts
# is a legitimate answer to a question, not a failure.
if [ "$MODE" = "--list" ]; then
  while IFS="$(printf '\t')" read -r cap target; do
    [ -n "$cap" ] && printf '%s\t%s\n' "$cap" "$target"
  done < "$DERIVED"
  exit 0
fi

# A target's coordinates are private, so they are resolved from the overlay
# exactly as tools/backup.sh resolves them: `kind` first (ssh is the default),
# then the shape that kind uses. `present` answers only what can be answered
# without the network: a local path that exists, or an ssh target that is merely
# configured. "unknown" is a real answer here and never a claim that a target is
# reachable.
json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

targets_json() {
  printf '['
  sep=""
  for target in ${TARGETS[@]+"${TARGETS[@]}"}; do
    kind="$(toml_get_in "$target" kind "$SYS_LOCAL")"; kind="${kind:-ssh}"
    host=""; path=""; present="unknown"
    case "$kind" in
      local)
        path="$(toml_get_in "$target" path "$SYS_LOCAL")"
        case "$path" in "~/"*) path="$HOME/${path#\~/}" ;; esac
        if [ -n "$path" ]; then
          if [ -d "$path" ]; then present="true"; else present="false"; fi
        fi
        ;;
      ssh)
        host="$(toml_get_in "$target" host "$SYS_LOCAL")"
        [ -n "$host" ] && present="unchecked"
        ;;
    esac
    declared=""
    while IFS="$(printf '\t')" read -r cap t; do
      [ "$t" = "$target" ] || continue
      declared="${declared:+$declared,}\"$(json_escape "$cap")\""
    done < "$DERIVED"
    printf '%s{"id":"%s","kind":"%s","path":"%s","host":"%s","present":"%s","declared_by":[%s]}' \
      "$sep" "$(json_escape "$target")" "$(json_escape "$kind")" \
      "$(json_escape "$path")" "$(json_escape "$host")" "$present" "$declared"
    sep=","
  done
  printf ']\n'
}

SYS_LOCAL="$SJEL_PERSONAL_ROOT/config/systems.local.toml"

if [ "$MODE" = "--targets-json" ]; then
  if [ ! -f "$SYS_LOCAL" ]; then
    echo "backup-all.sh: no $SYS_LOCAL — no target coordinates to report" >&2
    printf '[]\n'
    exit 1
  fi
  targets_json
  exit 0
fi

if [ "${#CAPS[@]}" -eq 0 ]; then
  echo "backup-all.sh: no capability declares a backup contract on this machine — nothing to do."
  exit 0
fi

echo "backup-all.sh: ${#CAPS[@]} contract(s): ${CAPS[*]}"
failed=()
for cap in "${CAPS[@]}"; do
  echo "── $cap"
  if ! "$TOOLS_DIR/backup.sh" "$cap"; then
    failed+=("$cap")
  fi
done

if [ "${#failed[@]}" -gt 0 ]; then
  echo "backup-all.sh: FAILED for ${failed[*]}" >&2
  exit 1
fi
echo "backup-all.sh: every contract backed up."
