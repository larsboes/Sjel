#!/bin/bash
# Current-tree publication floor. This deliberately scans the Git index rather than
# the working tree: ignored files may exist locally, but they cannot ride a commit into
# the public repository. It also rejects retired named-overlay and real-device markers.
set -euo pipefail

ROOT="${SJEL_PUBLICATION_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

failed=0
mac_home="/""Users/"
linux_home="/""home/"

# The accounts whose home directories belong to THIS installation. A path under one of them is
# a publication defect wherever it appears; a home path under any other account belongs to the
# machine that produced it, which is evidence rather than a leak.
#
# Inverted on 2026-10-02, from "flag every home path, allow foreign ones by name". That
# direction grows without limit: /home/agent and /home/runner were the first entries, the
# kitinerary verdict in upstreams.toml -- which quotes KDE's own macOS builder -- was the next,
# and /Users/builder or /Users/jenkins arrive the first time another project publishes a macOS
# build path. This direction does not grow at all, and it makes the home half of this gate the
# same kind of rule as the marker half below: name what is private to this installation, never
# the shape of what is private.
#
# It also deletes two exemptions rather than adding one. The repository's own lib/home/ and the
# container homes /home/agent/ and /home/runner/ matched the old marker and needed carve-outs;
# under an inclusion list they cannot match at all. A rule that needs fewer exceptions as it
# gets more precise is the sign it was pointed the right way.
#
# The cost, stated rather than discovered: an account name now appears in a tracked file. It is
# the repository owner's, already public in the remote URL, and CI cannot check the paths that
# matter without being told it -- CI has no overlay to read. An installation under another
# account sets SJEL_PRIVATE_USERS, and the regression test injects one rather than depending on
# this default.
private_users="${SJEL_PRIVATE_USERS:-larsboes}"
private_path="(${mac_home}|${linux_home})($(printf '%s' "${private_users}" | tr ' ' '|'))/"

while IFS= read -r -d '' path; do
  case "$path" in
    */__pycache__/*|*.pyc|*.pyo)
      echo "publication hygiene: tracked interpreter artifact: $path" >&2
      failed=1
      ;;
  esac
done < <(git ls-files -z)

# Ask Git for the small candidate set first; opening every indexed blob separately made
# Doctor pay one process launch per file. Inspect the indexed blob for each candidate so
# the verdict still describes exactly what Git would publish. `strings` includes binary
# metadata.
#
# The blob goes to a file rather than into a pipe, and that is not tidiness. Apple's
# `strings` answers differently on a pipe than on a path: `git show :file | strings` found
# NONE of the one `/Users/...` occurrence in upstreams.toml, while `strings <file>` found it
# and GNU `strings` in CI found it either way. So this gate was green on this workstation
# and red on every push — the worst way for a gate to be wrong, because it teaches the
# operator to trust the local run.
blob="$(mktemp)"
trap 'rm -f "$blob"' EXIT
while IFS= read -r path; do
  [ -n "$path" ] || continue
  git show ":$path" >"$blob" 2>/dev/null || continue
  if [ -n "$(LC_ALL=C strings "$blob" | grep -E "${private_path}" || true)" ]; then
    echo "publication hygiene: tracked blob contains this installation's home path: $path" >&2
    failed=1
  fi
done < <(git grep --cached -a -l -E "${private_path}" || true)

legacy_tooling_path='~/Developer/'"Tooling"
# Each marker must be followed by a non-identifier character or end of line, so a name that
# merely BEGINS with one does not match. capabilities/finance/src/allocation.rs names a journal
# tag `axon-personal-cents`, which is a field name in a ledger format and exposes nothing — and
# it turned this gate red on every push to main from the commit that introduced it. A marker
# exists to catch a deployment being named, not a string starting with the same letters.
instance_markers="(sjel-personal|sjel-family|axon-personal|axon-family|axon-work|lifeos-mono|obsidian-mono|DS220|Open Telekom Cloud|${legacy_tooling_path})([^-A-Za-z0-9]|$)"
while IFS= read -r path; do
  [ -n "$path" ] || continue
  case "$path" in
    # A file whose job is to detect markers has to contain them. That is this script, its
    # test, the sibling gate that scans built site bytes for the same list (#168), and the
    # advisory ISA sweep with its test. Nothing else earns an entry here: every other file
    # assembles the string at run time.
    tools/check-publication-hygiene.sh|tools/check-publication-hygiene.test.sh) continue ;;
    tools/check-site-payload.sh) continue ;;
    tools/isa-hygiene.sh|tools/isa-hygiene.test.sh) continue ;;
  esac
  echo "publication hygiene: tracked blob contains a deployment-instance marker: $path" >&2
  failed=1
done < <(git grep --cached -a -l -E "$instance_markers" || true)

if [ "$failed" -ne 0 ]; then
  exit 1
fi

echo "publication hygiene passed (tracked artifacts, workstation paths, and deployment markers)"
