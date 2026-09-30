# tools/lib/tool-index.sh — what tools/ holds, read out of tools/ itself.
#
# `sjel search` indexes commands, capabilities and Packs, and did not index the operator
# machinery at all — which is the one thing AGENTS.md sends a session here for: "run
# `sjel search <task>` before browsing files". A hand-written list of tools was never an
# option; it is the shape that goes stale the day somebody adds a script. So the index is
# each tool's own header comment.
#
# A library rather than four functions inside `axon` because `sjel search` cannot run in CI:
# it calls `tools/capability.sh registry`, which hard-fails without a machine.toml. The
# tools half is testable on any checkout, and tools/tool-index.test.sh drives it against a
# planted tools/ directory.
#
# Reads $SJEL_ROOT. bash 3.2-safe (CONTRIBUTING.md#portable-shell).

# One line about one tool, read from the tool.
#
# Most tools in tools/ open with `# tools/<name> — <what it is>` (or `//` for TypeScript),
# and that convention is the index: no list of tools exists anywhere to keep in step. Two
# shapes need a fallback and neither is a defect —
#
#   a thin launcher carries no description of its own (tools/doctor says "doctor's logic
#   lives in doctor.ts"), so the summary comes from the file it execs;
#
#   an older header names no path at all (tools/restore.sh opens "Restore and verify one
#   tools/backup.sh archive..."), so its first comment line is used verbatim. Printing the
#   file's own words beats printing nothing, and beats a summary invented here.
# The text a query is matched against: a tool's own header, plus the header of the file it
# execs when it is a launcher. Without the second part `sjel search gates` answered with
# tools/ci-local.ts and not with tools/ci-local, which is the file to run.
tool_headers() {  # <path>
  local file="$1" base
  base="${file##*/}"; base="${base%.ts}"; base="${base%.sh}"
  sed -n '1,25p' "$file"
  if [ -f "$SJEL_ROOT/tools/$base.ts" ] && [ "$file" != "$SJEL_ROOT/tools/$base.ts" ]; then
    sed -n '1,25p' "$SJEL_ROOT/tools/$base.ts"
  fi
}

tool_summary() {  # <path> -> the description, or nothing
  local file="$1" base line
  base="${file##*/}"; base="${base%.ts}"; base="${base%.sh}"
  line="$(sed -n '1,25p' "$file" | sed -nE 's@^(#|//) tools/[A-Za-z0-9._-]+ (—|--|-) *@@p' | head -n 1)"
  if [ -z "$line" ] && [ -f "$SJEL_ROOT/tools/$base.ts" ] && [ "$file" != "$SJEL_ROOT/tools/$base.ts" ]; then
    line="$(sed -n '1,25p' "$SJEL_ROOT/tools/$base.ts" | sed -nE 's@^// tools/[A-Za-z0-9._-]+ (—|--|-) *@@p' | head -n 1)"
  fi
  if [ -z "$line" ]; then
    line="$(sed -n '2,6p' "$file" | sed -nE 's@^(# |// )@@p' | head -n 1)"
  fi
  printf '%s' "$line"
}

# tools/ was not indexed at all, which is the section an agent reading AGENTS.md is sent
# here for: "run `sjel search <task>` before browsing files" over an index that held
# capabilities and Packs and none of the operator machinery.
#
# The query is matched against the file name and against the header block, so `sjel search
# backup` finds tools/backup.sh by name and tools/restore.sh by what its header says it
# does. Tests and libraries are excluded: a test is not a thing to run for a task, and
# tools/lib/* is reached through the tool that sources it.
#
# Matched with `case`, not with `grep -q`. This script runs under `pipefail`, and `grep -q`
# exits the moment it matches — which kills the `sed` upstream of it with SIGPIPE, and
# pipefail then reports the whole pipeline as failed. The first version of this silently
# dropped every early match: `sjel search "self-model logic"` found nothing while
# tools/self's second line contains that exact string.
search_tools() {  # <query> -> exit 0 if anything matched
  local query="$1" hits=0 file name base needle haystack summary
  needle="$(printf '%s' "$query" | tr '[:upper:]' '[:lower:]')"
  for file in "$SJEL_ROOT"/tools/*; do
    [ -f "$file" ] || continue
    name="${file##*/}"
    case "$name" in
      *.test.sh|*.test.ts|*.example|*.example.*|*.md|*.json|*.toml) continue ;;
    esac
    base="${name%.ts}"
    # A launcher and the .ts it execs are one tool, and the launcher is the thing to run.
    # Listing both would answer a question about `tools/doctor` with `tools/doctor.ts`.
    if [ "$base" != "$name" ] && [ -f "$SJEL_ROOT/tools/$base" ]; then continue; fi
    haystack="$(printf '%s\n%s' "$name" "$(tool_headers "$file")" | tr '[:upper:]' '[:lower:]')"
    case "$haystack" in
      *"$needle"*)
        summary="$(tool_summary "$file")"
        printf '  tools/%-24s %s\n' "$name" "$summary"
        hits=$((hits + 1))
        ;;
    esac
  done
  [ "$hits" -gt 0 ]
}
