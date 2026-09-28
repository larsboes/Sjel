#!/bin/bash
# check-obsidian-plugins.sh — the Obsidian-plugin gate (CI: repo gates).
#
# An Obsidian plugin in this repository is a hand-written `main.js` with no build step, so
# nothing between the source and Obsidian can catch a mistake in it. These four rules are
# the ones whose violation is invisible until the pane is already wrong.
#
# R1 — NO BROWSER-CONTEXT HTTP. `fetch()` from a plugin runs in Obsidian's renderer and
#      carries `Origin: app://obsidian.md`. The origin guard admits that exact origin
#      (libs/sjel-server/src/origin.rs, `origin_allowed_by`), but admission is not the
#      whole path: sjel-status carries no CORS layer (capabilities/sjel-status/src/main.rs,
#      `build_router`), so the renderer withholds the registry's reply and discovery fails.
#      `requestUrl` is issued by Electron's main process and sends no Origin, so it needs
#      neither the allowance nor a CORS header; it is the allowed path. The two spellings
#      look interchangeable and are not. `window.fetch` and `globalThis.fetch` are the same
#      global reached through a receiver, so they are named here too: a rule that only
#      catches the bare spelling refuses the mistake a person makes once and passes the one
#      an editor's autocomplete makes.
#
# R2 — READ-ONLY. vault computes the people facts and does not write them
#      (capabilities/vault/src/people.rs); trips owns its projections and overwrites them
#      whole. A plugin that also wrote would be a second writer of the same keys with no
#      shared refusal path. A plugin that must write is a design change, not an edit —
#      reopen this rule then.
#
# R3 — A PLUGIN DECLARING `isDesktopOnly: false` MAY REQUIRE ONLY `obsidian`. Node packages
#      do not exist on mobile; Obsidian's own loader answers a node require there with a
#      notice and null. The manifest and the source have to agree, and only one of them is
#      read at install time.
#
# R4 — ONE ADDRESS. A capability's port is a machine fact (`[capability.<name>] port` in
#      the overlay). A port literal in a plugin goes stale silently, so every address must
#      come from the registry sjel-status serves. Exactly one bootstrap address is allowed,
#      and it must sit on the `registryUrl:` setting line where a reader will find it —
#      the setting itself, not any line that happens to mention the word.
#
# Pure file-based check, same contract as the sibling gates: no git, no network, no build.
# It walks from its working directory rather than from the repository root, which is what
# lets check-obsidian-plugins.test.sh plant a tree and prove the red path instead of
# asserting it.
set -e

# A nested checkout is not part of the tree being checked; `.claude/worktrees/` holds full
# copies of this repository while a fleet is working in it. Same prune, same reason, as
# tools/check-store-transactions.sh.
PRUNE='-name .claude -o -name node_modules -o -name target -o -name .git'

# The rules are about code, so the scan is about code. A whole-line comment becomes an
# empty line — blanked rather than deleted, so the line numbers in a failure still point at
# the file. Without this the gate reports its own documentation: main.js has to be able to
# name the spelling it must not use, and a gate that forbids naming the hazard is a gate
# people work around by leaving the hazard undocumented.
code_of() { # code_of <file> -> the file with whole-line comments blanked
  sed -E 's@^[[:space:]]*(//|\*|/\*).*$@@' "$1"
}

# A browser request, spelled bare or through the global object that carries it. The first
# alternative refuses `fetch(`, `new XMLHttpRequest` and `new EventSource` while leaving a
# member call of somebody else's API alone (`this.cache.fetch(...)`); the second names the
# three receivers under which those members ARE the browser globals.
BROWSER='(^|[^.[:alnum:]_$])(fetch[[:space:]]*\(|XMLHttpRequest|EventSource)'
BROWSER="$BROWSER"'|(window|globalThis|self)\.(fetch[[:space:]]*\(|XMLHttpRequest|EventSource)'

# A write, spelled by its receiver. Naming `vault`, `adapter` and `fileManager` rather than
# the method alone is what keeps `el.append(child)` and `containerEl.createEl(...)` — DOM
# calls every plugin makes — out of the verdict. A gate that cries wolf on rendering is a
# gate somebody deletes.
# The quote around the verb is a character class, not a literal `"`: a plugin written with
# single quotes writes just as hard, and the gate that only reads one of the two spellings
# is the gate that passes the write it exists to refuse.
WRITE="method:[[:space:]]*[\"'](POST|PUT|PATCH|DELETE)"
WRITE="$WRITE"'|(vault|adapter|fileManager)\.(create|modify|append|process|delete|trash|rename|copy|write|remove|mkdir)[A-Za-z]*\('
WRITE="$WRITE"'|processFrontMatter'

fail=0
scanned=0

# Reported with the offending lines, indented, so a failure names the edit to make. `rel`
# comes from the caller's scope, which is the loop below.
refuse() { # refuse <message> <hits>
  echo "FAIL [$rel]: $1" >&2
  printf '%s\n' "$2" | head -5 | sed 's/^/    /' >&2
  fail=1
}

# An Obsidian plugin is a directory holding both a manifest with `minAppVersion` and a
# `main.js` beside it. Identified by that pair rather than by a path, so a plugin added
# anywhere is gated by existing.
while IFS= read -r manifest; do
  dir="$(dirname "$manifest")"
  main="$dir/main.js"
  [ -f "$main" ] || continue
  grep -q '"minAppVersion"' "$manifest" || continue
  rel="${dir#./}"
  scanned=$((scanned + 1))

  # R1
  hits="$(code_of "$main" | grep -nE "$BROWSER" || true)"
  [ -z "$hits" ] || refuse \
    "browser-context HTTP — the renderer withholds replies that carry no CORS header; requestUrl is the allowed path" "$hits"

  # R2
  hits="$(code_of "$main" | grep -nE "$WRITE" || true)"
  [ -z "$hits" ] || refuse \
    "writes — an Obsidian plugin here is read-only" "$hits"

  # R3, only for a plugin that claims to run on mobile.
  if grep -q '"isDesktopOnly"[[:space:]]*:[[:space:]]*false' "$manifest"; then
    hits="$(code_of "$main" | grep -nE 'require\(' | grep -vE 'require\("obsidian"\)' || true)"
    [ -z "$hits" ] || refuse \
      "isDesktopOnly is false but main.js requires something other than obsidian" "$hits"
  fi

  # R4
  hits="$(code_of "$main" | grep -nE '[a-z][a-z0-9+.-]*://[A-Za-z0-9._-]+:[0-9]+' | grep -vE 'registryUrl[[:space:]]*:' || true)"
  [ -z "$hits" ] || refuse \
    "an address outside the one bootstrap setting — take it from the registry instead" "$hits"
done < <(find . \( $PRUNE \) -prune -o -name manifest.json -print | sort)

# A sweep that found nothing is a broken gate, not a clean tree — the failure the sibling
# gate's own history proves is worth guarding.
if [ "$scanned" -eq 0 ]; then
  echo "FAIL: no Obsidian plugin found — the find is broken, not the tree" >&2
  exit 1
fi

if [ "$fail" -ne 0 ]; then
  echo "obsidian plugin check FAILED." >&2
  exit 1
fi

echo "obsidian plugin check passed ($scanned plugin(s): no browser-context HTTP, no writes, no stale addresses)."
