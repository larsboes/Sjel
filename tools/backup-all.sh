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
          console.log(r.name);
        }
      ' > "$DERIVED"; then
  echo "backup-all.sh: could not derive the backup set from the capability registry — refusing to report success" >&2
  exit 1
fi

CAPS=()
while IFS= read -r line; do
  [ -n "$line" ] && CAPS+=("$line")
done < "$DERIVED"

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
