# tools/lib/test-support.sh — shared helpers for Axon's shell tests.
#
# Two rules so far, and both earned a place here for the same reason: each is an assumption a
# test makes that nobody can see failing.
#
# The first: a test may skip a platform-dependent assertion on a developer machine, and may
# never skip it in CI. The case that prompted it: tools/backup.test.sh forces its short-write
# path with /dev/full, which Linux has and macOS does not. On macOS the test printed a note and
# passed, and the note claimed "it runs in CI" — an assumption nobody could see failing, because
# nobody reads the log of a green job. A
# platform-dependent assertion is not unusual, so the guard belongs to the convention rather
# than to that one test.
#
# The second: a test that builds a scratch Axon root must not inherit the SJEL_* environment.
# Same shape, opposite direction — green in CI, red only on the machine of whoever exported the
# variables. See isolate_axon_env below.
#
# bash 3.2-safe (CONTRIBUTING.md#portable-shell).

# in_ci — true when this is an automated run. GitHub Actions sets CI=true; so does essentially
# every other runner, which is the point: the guard should not know which one it is under.
in_ci() { [ "${CI:-}" = "true" ] || [ "${CI:-}" = "1" ]; }

# skippable <reason> — decide whether a platform-dependent assertion may be skipped here.
#
#   if skippable "no /dev/full on this host"; then
#     echo "NOTE: short-write assertion skipped — no /dev/full on this host"
#   else
#     ... run the assertion ...
#   fi
#
# Returns 0 (skip is allowed) outside CI, after printing what is being given up so the reduced
# coverage is visible rather than silent. Inside CI it does not return: a skip there means the
# assertion has quietly stopped running everywhere, which is the failure this prevents.
skippable() {
  local reason="${1:?skippable needs a reason}"
  if in_ci; then
    echo "FAIL: assertion skipped in CI ($reason) — a platform-dependent check that never runs" >&2
    echo "      anywhere is worse than one that fails. Provide the capability on the runner, or" >&2
    echo "      move the assertion to a host that has it." >&2
    exit 1
  fi
  echo "NOTE: reduced coverage on this host — $reason"
  return 0
}

# isolate_axon_env — drop the SJEL_* environment before a test builds its scratch root.
#
# tools/lib/paths.sh answers "which overlay am I" from an exported SJEL_OVERLAY_ROOT first, and
# only then from axon.local.toml / axon.toml. That precedence is deliberate: it is the
# per-invocation override tools/demo-up uses to run the published demo against demo/overlay
# while the operator's real overlay stays put.
#
# A test inherits it. An operator whose shell exports the set — which the overlay's own
# config/shell does — hands every scratch-root test the REAL overlay and the REAL
# config/machine.toml, so a sandbox that writes `os = "linux"` into its own machine.toml is
# read back as this machine's `os = "macos"`, and every env-block assertion reads the operator's
# declarations instead of the fixture's. The test does not error; it quietly measures the wrong
# machine.
#
# CI exports none of it, so the suite is green there and red only for whoever set the variables
# locally — the same invisible assumption skippable() exists for, which is why this lives beside
# it rather than in four test files. Measured when it was found: persistence, manifest-resolution,
# machine-resolution and state-mount-resolution all failed on the author's machine and all passed
# with the environment cleared.
#
# Call once, before resolving any scratch path. A test that wants its sandbox named explicitly
# still exports SJEL_OVERLAY_ROOT itself AFTER this (tools/check-site-payload.test.sh does).
#
# Both names are cleared: tools/lib/env-compat.sh refills a SJEL_ name from its pre-rename AXON_
# name, so clearing only the new one hands the test the real value again.
isolate_axon_env() {
  local v
  for v in ROOT OVERLAY_ROOT PERSONAL_ROOT MACHINE_TOML MACHINES_DIR CAPS_DIR OVERLAY_CAPS_DIR; do
    unset "SJEL_$v" "AXON_$v"
  done
}
