#!/bin/bash
# Advisory sweep of the tracked ISA.md files for strings that should not be published.
#
# This is a report, not a gate. It exits 0 whether or not it finds anything, because an ISA is
# an evidence log: it records machine-specific measurements on purpose, and whether a given one
# may be published is the operator's call, not a regex's. The hard floor stays
# tools/check-publication-hygiene.sh, which fails the build on this installation's home paths
# and named deployment markers in any tracked blob. This tool widens the net — volume names,
# hostnames, private addresses, uncommon path roots — and leaves the judgement to a human.
#
# Scope is the Git index: exactly what a commit would publish. A local edit that is not staged
# is not reported, and an untracked ISA.md is not reported either; `git add` it first if you
# want a new ISA in the sweep.
#
# Usage: tools/isa-hygiene.sh                      # this repository's tracked ISA.md files
#        SJEL_ISA_ROOT=<dir> tools/isa-hygiene.sh  # another checkout (used by the test)
#        SJEL_PRIVATE_NAMES="Extreme INTENSO" ...  # machine names from the overlay
set -uo pipefail

ROOT="${SJEL_ISA_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)}"
cd "$ROOT" || { echo "isa-hygiene: cannot enter $ROOT" >&2; exit 2; }

private_users="${SJEL_PRIVATE_USERS:-larsboes}"

# Machine names — volume labels, drive names, hostnames — that this installation uses and the
# public repo must not. The overlay owns them (`config/machine.toml [external_volume] name`, the
# media config's roots), so they arrive here as an env var rather than a committed list. A name is
# matched only inside backticks, the shape these documents write a volume label in, so an English
# "Extreme" in prose is not reported. A `/Volumes/<name>` path is already caught by the rule below.
private_names="${SJEL_PRIVATE_NAMES:-}"

# Each pattern names one shape of machine-private string. They are separate so the report can
# say which shape matched, and so a new shape is one line to add.
home_pattern="/(Users|home)/($(printf '%s' "$private_users" | tr ' ' '|'))/"
volume_pattern='/Volumes/[A-Za-z0-9_.-]+'
hostname_pattern='[A-Za-z0-9_-]+\.local'
ip_pattern='([0-9]{1,3}\.){3}[0-9]{1,3}'
marker_pattern='(sjel-personal|sjel-family|axon-personal|axon-family|axon-work|lifeos-mono|obsidian-mono|DS220|Open Telekom Cloud|~/Developer/Tooling)([^-A-Za-z0-9]|$)'
device_pattern=''
if [ -n "$private_names" ]; then
  names_alt="$(printf '%s' "$private_names" | tr ' ' '|')"
  device_pattern="\`(${names_alt})\`"
fi

# Strings that match a pattern above but are not private. A false-positive list, not a place to
# hide a finding: every entry is justified. `axon.local.toml` is the documented overlay pointer
# (CONTRIBUTING.md), `args.local` is the `.local`-suffixed name capabilities/printing's own P7
# probe already names as a false positive, and the loopback and unspecified addresses name no
# machine.
false_positives='axon\.local\.toml|args\.local|127\.[0-9]+\.[0-9]+\.[0-9]+|0\.0\.0\.0'

isas=()
while IFS= read -r f; do
  [ -n "$f" ] && isas+=("$f")
done < <(git ls-files | grep -E '(^|/)ISA\.md$' || true)

if [ "${#isas[@]}" -eq 0 ]; then
  echo "isa-hygiene: no tracked ISA.md files in $ROOT"
  exit 0
fi

echo "isa-hygiene: sweeping ${#isas[@]} tracked ISA.md file(s) in $ROOT"
echo

findings=0
seen_files=""

sweep() {
  local label="$1" pattern="$2"
  local hit loc rest lineno text filtered tokens
  while IFS= read -r hit; do
    [ -n "$hit" ] || continue
    loc="${hit%%:*}"
    rest="${hit#*:}"
    lineno="${rest%%:*}"
    text="${rest#*:}"
    filtered="$(printf '%s' "$text" | sed -E "s/${false_positives}//g")"
    tokens="$(printf '%s' "$filtered" | grep -oE "$pattern" | sort -u | tr '\n' ' ')"
    [ -n "$tokens" ] || continue
    printf '  %s:%s  %-14s %s\n' "$loc" "$lineno" "$label" "$tokens"
    findings=$((findings + 1))
    case " $seen_files " in
      *" $loc "*) ;;
      *) seen_files="$seen_files $loc" ;;
    esac
  done < <(git grep --cached -n -I -E "$pattern" -- "${isas[@]}" || true)
}

sweep "home path"       "$home_pattern"
sweep "volume path"     "$volume_pattern"
sweep "hostname"        "$hostname_pattern"
sweep "private address" "$ip_pattern"
sweep "instance marker" "$marker_pattern"
[ -n "$device_pattern" ] && sweep "device name" "$device_pattern"

echo
if [ "$findings" -eq 0 ]; then
  echo "isa-hygiene: no machine-private strings found — nothing to review."
else
  nfiles="$(printf '%s\n' $seen_files | grep -c .)"
  echo "isa-hygiene: $findings finding(s) in $nfiles file(s) — review each; this tool never fails the build."
fi
exit 0
