#!/usr/bin/env bash
# Run inside an agent harness's shell to see what its sandbox allows. Prints one line per probe:
# `ok` or `refused`, and whether that is what the containment rule expects. Reads one byte per
# file and writes one marker file it removes. bash 3.2-safe.
#
#   tools/harness-sandbox-probe.sh
set -u
_here="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
# shellcheck source=tools/lib/paths.sh
. "$_here/lib/paths.sh" >/dev/null 2>&1
fails=0
probe() { # <label> <expect ok|refused> <command...>
  label="$1"; expect="$2"; shift 2
  if "$@" >/dev/null 2>&1; then got=ok; else got=refused; fi
  if [ "$got" = "$expect" ]; then mark=as-expected; else mark=UNEXPECTED; fails=$((fails + 1)); fi
  printf '%-34s %-8s %s\n' "$label" "$got" "$mark"
}
target="${CARGO_TARGET_DIR:-$SJEL_ROOT/target}"
probe "read overlay data/"      refused head -c 1 "$SJEL_PERSONAL_ROOT/data/sjel/sjel.db"
probe "read overlay config/"    ok      head -c 1 "$SJEL_PERSONAL_ROOT/config/machine.toml"
probe "read repository"         ok      head -c 1 "$SJEL_ROOT/README.md"
probe "git status"              ok      git -C "$SJEL_ROOT" status --short
probe "write repository"        ok      sh -c "echo x > '$SJEL_ROOT/.sandbox-probe' && rm '$SJEL_ROOT/.sandbox-probe'"
probe "write cargo target dir"  ok      sh -c "mkdir -p '$target' && echo x > '$target/.sandbox-probe' && rm '$target/.sandbox-probe'"
probe "cargo metadata"          ok      cargo metadata --no-deps --format-version 1 --manifest-path "$SJEL_ROOT/Cargo.toml"
probe "loopback sjel-status"    ok      curl -s -o /dev/null --max-time 3 http://127.0.0.1:8082/health
probe "sjel backup status"      ok      sjel backup status
probe "service-runner status"   ok      "$SJEL_ROOT/tools/service-runner.sh" status
probe "registry (crates.io)"    ok      curl -s -o /dev/null --max-time 5 https://index.crates.io/config.json
echo "harness-sandbox-probe: $fails unexpected"
[ "$fails" -eq 0 ]
