# tools/lib/pipe.sh — asking "does this stream contain X" without the answer depending on timing.
#
# Every script here runs `set -euo pipefail`, and this shape is a trap under it:
#
#     if "$RUNTIME_BIN" list --format json | grep -q "\"id\":\"$NAME\""; then
#
# `grep -q` exits at the first match. If the producer is still writing at that moment it dies of
# SIGPIPE, and pipefail turns the whole pipeline non-zero — so a FOUND name reads as NOT FOUND.
# Whether it fires depends on where in the output the match happens to sit, which makes it a bug
# that passes every test written against a short answer:
#
#     $ bash -c 'set -o pipefail; seq 1 300000 | grep -q "^7$"; echo "${PIPESTATUS[*]} -> $?"'
#     141 0 -> 141          # grep matched. The pipeline says it failed.
#     $ bash -c 'set -o pipefail; seq 1 300000 | grep -q "^299999$"; echo "${PIPESTATUS[*]} -> $?"'
#     0 0 -> 0              # same question, match near the end, correct answer
#
# It was found for real (#42): `launchctl list | grep -q "com\.axon\.$CAP\$"` reported a loaded
# launchd agent as not loaded, because that output is hundreds of lines long.
#
# The fix is not to drop pipefail — pipefail is why a failing producer is not silently read as
# "no match". It is to stop asking the question in a way that kills the producer.
#
# bash 3.2-safe (CONTRIBUTING.md#portable-shell).

# stream_matches <grep args...> — true when stdin matches, false when it does not.
#
# `grep -c` instead of `grep -q`: it consumes the whole stream, so the producer always reaches its
# own exit rather than being shot in the middle of a write. That makes the answer a property of the
# data instead of a property of scheduling. It costs reading the rest of the input, which for every
# caller here is a container list or a launchctl dump — bounded, small, and already in flight.
#
# A producer that genuinely fails still fails: its non-zero status reaches pipefail exactly as
# before, because nothing here suppresses it.
stream_matches() {
  local hits
  hits="$(grep -c "$@" || true)"
  [ "${hits:-0}" -gt 0 ]
}
