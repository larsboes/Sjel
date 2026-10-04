#!/bin/bash
# Keep the Pack's stable migration command; the implementation lives in sjel-cli.
set -u
_here="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
_root="${SJEL_ROOT:-$(cd "$_here/../../../.." && pwd)}"
export SJEL_ROOT="$_root"
. "$_root/tools/lib/sjel-cli.sh"
sjel_cli_exec inference-keys migrate "$@"
