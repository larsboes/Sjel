#!/bin/bash
# check-manifest-integrity.sh — the manifest referential-integrity gate (CI: repo gates).
# So a dangling name can never ship:
#   every entry in any capabilities/*/service.toml `requires = [...]` maps to a real
#   capabilities/<name>/ dir.
# Pure file-based (dir-existence) check: it reads the tracked manifests and the tree they
# name, and nothing else — no git, no network, no live service.
#
# What this gate deliberately no longer checks, and where those checks went:
# until 2026-07-26 it also validated the ENABLED capability set — that each enabled name
# had a directory, and that the set was dependency-closed. Both read `capabilities = [...]`
# from the tracked axon.toml. That field now lives in <overlay>/config/machine.toml, which
# is per-machine and absent from a fresh clone by construction, so those two checks moved
# to tools/doctor, which runs on a real machine and can see its overlay. The split is the
# honest one: this gate checks what is true of the repo, doctor checks what is true of
# the machine. See schemas/machine.toml.example.
set -e

# paths.sh sources toml.sh and exports SJEL_ROOT.
_lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/lib" && pwd)"
source "$_lib/paths.sh"

fail=0

# Every service.toml `requires = [...]` entry → each must be a real capabilities/<name>/ dir.
# Word-splitting on toml_array's newline-per-element output is safe: capability names carry
# no whitespace (bash 3.2-safe, no mapfile).
for svc in "$SJEL_ROOT"/capabilities/*/service.toml "$SJEL_ROOT"/*/service.toml; do
  [ -f "$svc" ] || continue          # empty glob → literal path, skip it
  owner="$(basename "$(dirname "$svc")")"
  for dep in $(toml_array requires "$svc"); do
    if [ ! -d "$SJEL_ROOT/capabilities/$dep" ]; then
      echo "FAIL [$owner]: requires '$dep' but capabilities/$dep/ does not exist" >&2
      fail=1
    fi
  done
done

if [ "$fail" -ne 0 ]; then
  echo "manifest integrity check FAILED." >&2
  exit 1
fi

echo "manifest integrity check passed (every service.toml requires= resolves to a real capability)."
echo "Enabled-set checks (dirs exist, set is dependency-closed) run in tools/doctor — machine-level, not hermetic."
