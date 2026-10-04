#!/bin/bash
# packs.sh — thin compatibility shim over tools/packs-claude.
#
# This tool used to symlink Pack skills into ~/.claude/skills. It no longer does
# (principal, 2026-08-09: "we should only deploy from sjel overlays never using
# symlinks"). The deployment now copies, and ownership is proven by a ledger at
# ~/.local/state/axon/pack-deployments/claude.json rather than by reading a
# symlink's target.
#
# Kept as a shim rather than deleted because `sjel pack list claude`, two Pack
# READMEs and schemas/pack.toml.example all name it, and a removed tool turns
# every one of those into a dead reference. The verbs map straight through:
#
#   packs.sh list          -> packs-claude status
#   packs.sh link <name>   -> packs-claude deploy <name>
#   packs.sh unlink <name> -> packs-claude remove <name>
#
# One behaviour difference worth knowing before you run `link`: a destination
# that already exists and is not in the ledger is now a reported collision rather
# than something silently skipped. `packs-claude adopt <name>` claims it, but only
# when its content is byte-identical to the Pack source.
#
# bash 3.2-safe.
set -e

_here="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
_claude() { exec "$_here/packs-claude" "$@"; }

case "${1:-list}" in
  list)   _claude status;;
  link)   [ -n "${2:-}" ] || { echo "usage: packs.sh link <name>" >&2; exit 1; }; _claude deploy "$2";;
  unlink) [ -n "${2:-}" ] || { echo "usage: packs.sh unlink <name>" >&2; exit 1; }; _claude remove "$2";;
  adopt)  [ -n "${2:-}" ] || { echo "usage: packs.sh adopt <name>" >&2; exit 1; }; _claude adopt "$2";;
  status|deploy|remove|sync) _claude "$@";;
  *)      echo "usage: packs.sh list | link <name> | unlink <name> | adopt <name>" >&2; exit 1;;
esac
