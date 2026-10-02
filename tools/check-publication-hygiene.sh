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
home_path="(${mac_home}|${linux_home})[A-Za-z0-9._-]+/"

# The repository's own lib/home/ directory is not a home directory.
#
# Home's decision ladder lives under the dashboard's src/lib/home/, and giving it
# subdirectories made every import specifier below it match the second marker above. The
# exemption is that ONE shape and nothing else. Narrowing the marker instead -- demanding a
# non-identifier character before the slash -- also stopped catching two real leaks, since
# both begin after an identifier character: the a/ and b/ prefixed paths a committed diff
# or patch file carries, and a workstation path written relative, ../../<marker><name>/.
# The exemption applies to the verdict only; the candidate list below stays broad, so a
# file is still opened and read whenever it matches at all.
repo_home_dir="lib/${linux_home#/}"

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
# metadata. Container and CI homes are portable public examples.
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
  home_hits="$(LC_ALL=C strings "$blob" \
    | grep -E "${home_path}" \
    | grep -Ev "${repo_home_dir}" \
    | grep -Ev "${linux_home}(agent|runner)/" || true)"
  if [ -n "$home_hits" ]; then
    echo "publication hygiene: tracked blob contains a workstation home path: $path" >&2
    failed=1
  fi
done < <(git grep --cached -a -l -E "${home_path}" || true)

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
    # test, and since #168 the sibling gate that scans built site bytes for the same list.
    # Nothing else earns an entry here: every other file assembles the string at run time.
    tools/check-publication-hygiene.sh|tools/check-publication-hygiene.test.sh) continue ;;
    tools/check-site-payload.sh) continue ;;
  esac
  echo "publication hygiene: tracked blob contains a deployment-instance marker: $path" >&2
  failed=1
done < <(git grep --cached -a -l -E "$instance_markers" || true)

if [ "$failed" -ne 0 ]; then
  exit 1
fi

echo "publication hygiene passed (tracked artifacts, workstation paths, and deployment markers)"
