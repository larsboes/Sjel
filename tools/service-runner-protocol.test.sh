#!/bin/bash
set -eu

src="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/sjel-protocol.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
root="$tmp/root"
mkdir -p "$root/tools/lib" "$root/tools/sjel-cli/src" "$root/target/release" "$tmp/overlay/config" "$tmp/bin"
cp "$src/service-runner.sh" "$root/tools/"
cp "$src"/lib/*.sh "$root/tools/lib/"
printf 'overlay = "%s"\n' "$tmp/overlay" > "$root/sjel.toml"
printf '[package]\nname = "fixture"\n' > "$root/tools/sjel-cli/Cargo.toml"

# Both fakes record execution; only the new one implements the protocol.
printf '%s\n' '#!/bin/bash' 'if [ "${1:-}" = runtime ]; then exit 0; fi' 'printf "%s\n" "$*" >> "$TEST_LOG"' 'exit 1' > "$tmp/old"
printf '%s\n' '#!/bin/bash' 'printf "%s\n" "$*" >> "$TEST_LOG"' 'case "${1:-}" in --service-runner-restart-protocol) printf "%s\n" sjel-service-runner-held-restart-v1 ;; esac' 'exit 0' > "$tmp/new"
chmod +x "$tmp/old" "$tmp/new"
export TEST_LOG="$tmp/log" SJEL_OVERLAY_ROOT="$tmp/overlay"
unset SJEL_CLI_BIN CARGO_TARGET_DIR SJEL_MACHINE_TOML
runner="$root/tools/service-runner.sh"

# Verbs other than restart pass straight through without a probe.
for args in 'start other' 'status other' 'stop other'; do
  : > "$TEST_LOG"
  # Intentional splitting: each fixture is a two-word argument list.
  SJEL_CLI_BIN="$tmp/new" "$runner" $args >/dev/null
  [ "$(< "$TEST_LOG")" = "service-runner.sh $args" ] || exit 1
done

# Every restart requires the held-restart protocol.
: > "$TEST_LOG"
if SJEL_CLI_BIN="$tmp/old" SJEL_SERVICE_RUNNER_RESTART_PROTOCOL=sjel-service-runner-held-restart-v1 "$runner" restart other >/dev/null 2>&1; then
  printf 'old binary accepted restart\n' >&2; exit 1
fi
[ "$(< "$TEST_LOG")" = --service-runner-restart-protocol ] || exit 1
: > "$TEST_LOG"
SJEL_CLI_BIN="$tmp/new" "$runner" restart other >/dev/null
[ "$(< "$TEST_LOG")" = "$(printf '%s\n' --service-runner-restart-protocol 'service-runner.sh restart other')" ] || exit 1
# Matching output with a failing exit status is not a valid probe.
printf '%s\n' '#!/bin/bash' 'printf "%s\n" sjel-service-runner-held-restart-v1' 'exit 1' > "$tmp/bad-restart"
chmod +x "$tmp/bad-restart"
for bin in "$tmp/bad-restart" "$tmp/missing"; do
  if SJEL_CLI_BIN="$bin" "$runner" restart other >/dev/null 2>&1; then
    printf 'failed or missing restart probe accepted\n' >&2; exit 1
  fi
done

# A failed rebuild must not let the last good binary restart without safe ordering.
cp "$tmp/old" "$root/target/release/sjel-cli"
sleep 1
touch "$root/tools/sjel-cli/src/newer.rs"
printf '%s\n' '#!/bin/bash' 'exit 1' > "$tmp/bin/cargo"
chmod +x "$tmp/bin/cargo"
: > "$TEST_LOG"
if env -u SJEL_CLI_BIN PATH="$tmp/bin:$PATH" "$runner" restart other >/dev/null 2>&1; then
  printf 'failed build accepted stale restart ordering\n' >&2; exit 1
fi
[ "$(< "$TEST_LOG")" = --service-runner-restart-protocol ] || exit 1
: > "$TEST_LOG"
env -u SJEL_CLI_BIN PATH="$tmp/bin:$PATH" "$runner" status other >/dev/null 2>&1 || :
[ "$(< "$TEST_LOG")" = 'service-runner.sh status other' ] || exit 1

# A missing cargo follows the same fallback, without inheriting a host toolchain.
mkdir -p "$tmp/no-cargo"
for tool in dirname basename hostname find; do
  ln -s "$(command -v "$tool")" "$tmp/no-cargo/$tool"
done
: > "$TEST_LOG"
if PATH="$tmp/no-cargo" "$runner" restart other >/dev/null 2>&1; then
  printf 'missing cargo accepted stale restart ordering\n' >&2; exit 1
fi
[ "$(< "$TEST_LOG")" = --service-runner-restart-protocol ] || exit 1
printf 'service-runner protocol: all checks passed\n'
