# tools/lib/version.sh — ordering version strings, and naming Axon's own release line.
#
# This file was 355 lines on 2026-08-28 and is now 57. It was extracted from
# tools/upstream-checker, and PRD Q41 retired that script: with it went the tag-decoration
# rules, the installed-version probes, the drift classifier and the GHSA range comparator
# — roughly 300 lines whose only callers were the checker and tools/lib/advisories.sh.
#
# What is left had nothing to do with upstream watching, which is why the file survives at
# all rather than being deleted with the rest of the stack:
#
#   norm_ver / ver_numeric / ver_gt   tools/lib/delta.sh, tools/release, tools/toolchain-check
#   release_tag_glob                  tools/lib/delta.sh
#   describe_release                  tools/update.sh
#
# The deleted comparator is worth one sentence, because the bug it was written for can come
# back in any caller that reaches for `!=`: "differs from the pin" is not "newer than the
# pin". GitHub's /releases/latest is the most recently PUBLISHED non-prerelease, not the
# highest version, so a project with maintenance branches publishes 58.4.0 after 59.1.0.
# Read as drift, that is advice to downgrade wearing the formatting of a supply-chain
# warning. ver_gt exists so an ordering question is answered by an ordering, and
# ver_numeric exists so a tag that cannot be ordered says so instead of guessing.
#
# Portable shell, bash 3.2 compatible (README.md#portable-shell). `sort -V` is present on BSD/macOS and
# GNU alike, so nothing here needs coreutils.

# norm_ver <tag> — strip a leading v.
norm_ver() { printf '%s' "$1" | sed -E 's/^v//'; }

# ver_numeric <v> — true when the tag is a dotted number that can actually be ordered.
# cli-v2026.6.0, ProjectName-v1.1.0, trixie-20260713-slim and git shas are not, and
# inventing an order for them would be worse than admitting there isn't one.
ver_numeric() { printf '%s' "$1" | grep -Eq '^[0-9]+(\.[0-9]+)*$'; }

# ver_gt <a> <b> — true when a sorts strictly above b.
ver_gt() {
  [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n1)" = "$1" ]
}

# --- release-line identity ------------------------------------------------
# Defensive source: normally toml.sh arrives via paths.sh, but version.sh is also sourced
# directly by tests that never went through it. Same idiom as delta.sh.
command -v toml_get_in >/dev/null 2>&1 || . "$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/toml.sh"

# release_tag_glob — the pattern that decides which tags are release tags, from axon.toml
# [release] tag_glob. Read rather than hardcoded because six call sites in two languages ask
# the same question; a literal in each is how a non-release tag takes over version identity.
# Fails loudly rather than guessing: a missing key means the manifest is wrong, and a silent
# default would restore exactly the one-home problem this exists to remove.
release_tag_glob() {
  local g
  g="$(toml_get_in release tag_glob "${SJEL_ROOT:?SJEL_ROOT unset}/axon.toml")"
  [ -n "$g" ] || { echo "version.sh: axon.toml has no [release] tag_glob" >&2; return 1; }
  printf '%s' "$g"
}

# describe_release [<rev>] — `git describe` restricted to the release line. Degrades to a short
# sha when no release tag is reachable, which is the honest answer for an untagged checkout and
# the reason --always is kept. Marks a dirty tree only when describing the working copy: git
# rejects --dirty together with an explicit revision.
describe_release() {
  local rev="${1:-}" glob
  glob="$(release_tag_glob)" || return 1
  if [ -n "$rev" ]; then
    git -C "$SJEL_ROOT" describe --tags --always --match "$glob" "$rev" 2>/dev/null
  else
    git -C "$SJEL_ROOT" describe --tags --always --dirty --match "$glob" 2>/dev/null
  fi
}
