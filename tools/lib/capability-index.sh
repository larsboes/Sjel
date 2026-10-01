# tools/lib/capability-index.sh — the capabilities half of `sjel search`.
#
# `sjel search mail` returned no capability on 2026-09-30, although capabilities/comms sweeps
# the inbox: the search matched a capability's name and kind and nothing else (ISA ISC-33).
# The index is now what a capability says about itself, in two places it already keeps:
#
#   the README's opening paragraph, which states what the capability is;
#
#   the route manifest, the `r("GET", "/triage", "...")` array that `GET /routes` serves.
#   It is read from the source and not from the running server, because most capabilities
#   are off on a given machine and `/routes` answers 401 without the deployment token. Each
#   server's `the_manifest_covers_every_served_route` test keeps the array and the router
#   in step, so the source is the same list `/routes` would return.
#
# Not the whole README: measured 2026-10-01, a whole-README match for "backup" found 16
# capabilities, because nearly every README says how it is backed up. The opening paragraph
# and the routes found 5.
#
# Reads $SJEL_ROOT, and $SJEL_OVERLAY_CAPS_DIR when it is set. bash 3.2-safe
# (CONTRIBUTING.md#portable-shell).

capability_dir() {  # <name> -> its directory, empty if no root has it
  if [ -d "$SJEL_ROOT/capabilities/$1" ]; then
    printf '%s' "$SJEL_ROOT/capabilities/$1"
  elif [ -n "${SJEL_OVERLAY_CAPS_DIR:-}" ] && [ -d "$SJEL_OVERLAY_CAPS_DIR/$1" ]; then
    printf '%s' "$SJEL_OVERLAY_CAPS_DIR/$1"
  fi
}

# The first paragraph after the README's title, joined to one line. The title is the first
# heading, not line 1: four READMEs open with blank lines.
capability_summary() {  # <dir>
  [ -f "$1/README.md" ] || return 0
  awk '!titled { if (/^#/) titled = 1; next }
       /^#/ { exit }
       NF == 0 && seen { exit }
       NF { seen = 1; printf "%s%s", (n++ ? " " : ""), $0 }' "$1/README.md"
}

# One line per declared route: `METHOD /path  description`.
capability_routes() {  # <dir>
  [ -d "$1/src" ] || return 0
  find "$1/src" -name '*.rs' -print0 |
    xargs -0 perl -0777 -ne '
      while (/\br\(\s*"(GET|POST|PUT|PATCH|DELETE|HEAD)",\s*"([^"]*)",\s*"((?:[^"\\]|\\.)*)"/g) {
        print "$1 $2  $3\n";
      }'
}

# stdin: one `name<TAB>kind` line per registered capability (the registry, already reduced).
# A capability matches on its name, its kind, its summary, or any route. Route matches are
# listed under it, at most three, so the answer says which call to make and not only where.
#
# Matched with `case`, not `grep -q`, for the reason tools/lib/tool-index.sh gives: under
# pipefail, `grep -q` exits on the first match and the upstream SIGPIPE fails the pipeline.
search_capabilities() {  # <query> -> exit 0 if anything matched
  local needle hits=0 name kind dir summary routes head route matched shown listed
  needle="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')"
  while IFS="$(printf '\t')" read -r name kind; do
    [ -n "$name" ] || continue
    dir="$(capability_dir "$name")"
    summary="" routes=""
    if [ -n "$dir" ]; then
      summary="$(capability_summary "$dir")"
      routes="$(capability_routes "$dir")"
    fi
    head="$(printf '%s %s\n%s' "$name" "$kind" "$summary" | tr '[:upper:]' '[:lower:]')"
    matched=0
    case "$head" in *"$needle"*) matched=1 ;; esac
    shown="" listed=0
    while IFS= read -r route; do
      [ -n "$route" ] || continue
      case "$(printf '%s' "$route" | tr '[:upper:]' '[:lower:]')" in
        *"$needle"*)
          matched=1
          if [ "$listed" -lt 3 ]; then
            shown="$shown$route
"
            listed=$((listed + 1))
          fi
          ;;
      esac
    done <<EOF
$routes
EOF
    [ "$matched" -eq 1 ] || continue
    if [ "${#summary}" -gt 100 ]; then summary="${summary:0:97}..."; fi
    printf '  %-20s %-8s %s\n' "$name" "$kind" "$summary"
    printf '%s' "$shown" | sed 's/^/      /'
    hits=$((hits + 1))
  done
  [ "$hits" -gt 0 ]
}
