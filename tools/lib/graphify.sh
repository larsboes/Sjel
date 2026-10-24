#!/bin/bash
# graphify_spec [extras] -> the uv requirement for the graphify release tools/intake approved,
# for example `graphifyy[openai]==0.9.40`. Without an approved release it prints how to get one
# and returns 1: an unread release does not reach an agent (tools/intake, `check graphify`).
# SJEL_INTAKE replaces the intake launcher in tests.
# bash 3.2-safe (CONTRIBUTING.md#portable-shell).
graphify_spec() {
  local extras="${1:-}" version intake
  intake="${SJEL_INTAKE:-${SJEL_ROOT:?}/tools/intake}"
  if ! version="$("$intake" version graphify 2>/dev/null)" || [ -z "$version" ]; then
    echo "graphify: no reviewed release on this machine. Run: tools/intake check graphify" >&2
    return 1
  fi
  printf 'graphifyy%s==%s' "${extras:+[$extras]}" "$version"
}
