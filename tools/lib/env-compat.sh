#!/bin/bash
# Makes every setting visible under both names while the rename from Axon to Sjel (2026-09-26)
# settles: a variable exported as AXON_X is also exported as SJEL_X, and the reverse, unless the
# other name is already set. New code reads SJEL_X; an older unit, overlay script or shell profile
# that sets or reads AXON_X keeps working. libs/sjel-config/src/env.rs is the Rust counterpart.
#
# Sourced first by tools/lib/paths.sh. Works under bash 3.2 and zsh (~/.zshrc sources paths.sh),
# so it uses `env` and `eval` rather than bash's ${!name} or zsh's ${(P)name}.
_sjel_env_sync() {
  local from="$1" to="$2" name rest
  for name in $(env | sed -n "s/^\(${from}[A-Za-z0-9_]*\)=.*/\1/p"); do
    rest="${name#"$from"}"
    eval "[ -n \"\${${to}${rest}+x}\" ] || export ${to}${rest}=\"\${${name}}\""
  done
}
_sjel_env_sync AXON_ SJEL_
_sjel_env_sync SJEL_ AXON_
unset -f _sjel_env_sync
