#!/bin/bash
# tools/schedule.test.sh — tools/lib/schedule.sh against the shared case table.
#
# The runner's copy of the rule (tools/sjel-cli/src/schedule.rs) reads the same
# tools/lib/schedule-cases.tsv in its unit test. Both passing is what "the gate and the runner
# agree on what a schedule means" rests on.
set -uo pipefail
_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
. "$_dir/lib/schedule.sh"
fails=0
while IFS="$(printf '\t')" read -r spec want; do
  case "$spec" in ''|'#'*) continue ;; esac
  got="$(schedule_seconds "$spec")"; rc=$?
  case "$want" in
    error:*)
      if [ "$rc" -eq 0 ] || [ "${got#*"${want#error:}"}" = "$got" ]; then
        echo "FAIL: '$spec' should be refused with '${want#error:}', got rc=$rc '$got'"; fails=$((fails + 1))
      fi
      ;;
    *)
      if [ "$rc" -ne 0 ] || [ "$got" != "$want" ]; then
        echo "FAIL: '$spec' should be $want seconds, got rc=$rc '$got'"; fails=$((fails + 1))
      fi
      ;;
  esac
done < "$_dir/lib/schedule-cases.tsv"
[ "$fails" -eq 0 ] || { echo "schedule: $fails case(s) failed"; exit 1; }
echo "schedule: all cases passed"
