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
printf '%s\n' '#!/bin/bash' 'printf "%s\n" "$*" >> "$TEST_LOG"' 'exit 1' > "$tmp/old"
printf '%s\n' '#!/bin/bash' 'printf "%s\n" "$*" >> "$TEST_LOG"' 'if [ "${1:-}" = --service-runner-protocol ]; then printf "%s\n" sjel-service-runner-sqld-rotation-v1; fi' 'exit 0' > "$tmp/new"
chmod +x "$tmp/old" "$tmp/new"
export TEST_LOG="$tmp/log" SJEL_OVERLAY_ROOT="$tmp/overlay"
unset SJEL_CLI_BIN CARGO_TARGET_DIR SJEL_MACHINE_TOML
runner="$root/tools/service-runner.sh"

for verb in start resume restart recreate; do
  : > "$TEST_LOG"
  if SJEL_CLI_BIN="$tmp/old" SJEL_SERVICE_RUNNER_PROTOCOL=sjel-service-runner-sqld-rotation-v1 "$runner" "$verb" sqld >/dev/null 2>&1; then
    printf 'old binary accepted %s sqld\n' "$verb" >&2; exit 1
  fi
  [ "$(< "$TEST_LOG")" = --service-runner-protocol ] || exit 1
  : > "$TEST_LOG"
  SJEL_CLI_BIN="$tmp/new" "$runner" "$verb" sqld >/dev/null
  [ "$(< "$TEST_LOG")" = "$(printf '%s\n' --service-runner-protocol "service-runner.sh $verb sqld")" ] || exit 1
done
# Matching output with a failing exit status is not a valid probe.
printf '%s\n' '#!/bin/bash' 'printf "%s\n" sjel-service-runner-sqld-rotation-v1' 'exit 1' > "$tmp/bad"
chmod +x "$tmp/bad"
for bin in "$tmp/bad" "$tmp/missing"; do
  if SJEL_CLI_BIN="$bin" "$runner" start sqld >/dev/null 2>&1; then
    printf 'failed or missing probe accepted\n' >&2; exit 1
  fi
done
for args in 'status sqld' 'stop sqld' 'start other' 'restart other'; do
  : > "$TEST_LOG"
  # Intentional splitting: each fixture is a two-word argument list.
  SJEL_CLI_BIN="$tmp/new" "$runner" $args >/dev/null
  [ "$(< "$TEST_LOG")" = "service-runner.sh $args" ] || exit 1
done

# A legacy whole-machine fan-out reaches the same launcher for each service.
printf '%s\n' '#!/bin/bash' 'if [ "${2:-}" = up ]; then exec "$TEST_RUNNER" start sqld; fi' 'printf "%s\n" "$*" >> "$TEST_LOG"' 'exit 1' > "$tmp/fanout"
chmod +x "$tmp/fanout"
export TEST_RUNNER="$runner"
: > "$TEST_LOG"
if SJEL_CLI_BIN="$tmp/fanout" "$runner" up >/dev/null 2>&1; then
  printf 'old fan-out accepted sqld\n' >&2; exit 1
fi
[ "$(< "$TEST_LOG")" = --service-runner-protocol ] || exit 1

# Failed rebuild must not let the last good but unguarded binary run sqld.
cp "$tmp/old" "$root/target/release/sjel-cli"
sleep 1
touch "$root/tools/sjel-cli/src/newer.rs"
printf '%s\n' '#!/bin/bash' 'exit 1' > "$tmp/bin/cargo"
chmod +x "$tmp/bin/cargo"
: > "$TEST_LOG"
if env -u SJEL_CLI_BIN PATH="$tmp/bin:$PATH" "$runner" start sqld >/dev/null 2>&1; then
  printf 'failed build accepted stale binary\n' >&2; exit 1
fi
[ "$(< "$TEST_LOG")" = --service-runner-protocol ] || exit 1
: > "$TEST_LOG"
env -u SJEL_CLI_BIN PATH="$tmp/bin:$PATH" "$runner" status sqld >/dev/null 2>&1 || :
[ "$(< "$TEST_LOG")" = 'service-runner.sh status sqld' ] || exit 1

# A missing cargo follows the same fallback, without inheriting a host toolchain.
mkdir -p "$tmp/no-cargo"
for tool in dirname basename hostname find; do
  ln -s "$(command -v "$tool")" "$tmp/no-cargo/$tool"
done
: > "$TEST_LOG"
if PATH="$tmp/no-cargo" "$runner" start sqld >/dev/null 2>&1; then
  printf 'missing cargo accepted stale binary\n' >&2; exit 1
fi
[ "$(< "$TEST_LOG")" = --service-runner-protocol ] || exit 1

if [ -n "${SJEL_TEST_REAL_BIN:-}" ]; then
  mkdir -p "$root/capabilities/sqld"
  printf 'name = "sqld"\n' > "$root/capabilities/sqld/service.toml"
  printf 'os = "linux"\ncontainer_runtime = "docker"\ncapabilities = ["sqld"]\n' > "$tmp/overlay/config/machine.toml"
  : > "$tmp/overlay/config/sqld-rotation-inhibit"
  for verb in start resume restart recreate; do
    output="$(SJEL_CLI_BIN="$SJEL_TEST_REAL_BIN" "$runner" "$verb" sqld 2>&1)" && {
      printf 'real runner accepted inhibited %s\n' "$verb" >&2; exit 1
    }
    case "$output" in *'inhibited for rotation'*) ;; *) printf 'unexpected refusal: %s\n' "$output" >&2; exit 1 ;; esac
  done
fi
printf 'service-runner protocol: all checks passed\n'
