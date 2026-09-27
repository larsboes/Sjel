# people-registry

Rung 0's known-person registry, refreshed from the vault every six hours.

`capabilities/comms/src/people_registry.rs` consults a list of the names this operator actually
knows, because rung 1's person detector only fires after a salutation and misses a bare first
name. The list is derived from `Atlas/People` and goes stale the moment a person note is added.

The output is C2 data — real names — so it lands in the overlay, which gitignores `/data/*`, and
never in this repository. `tools/people-registry-refresh.sh` refuses to install an empty or
malformed result, so a failed vault read leaves the previous good file in place rather than
blanking rung 0.

## Why this is a capability and not a LaunchAgent

Same reason as `capabilities/backup`, and with one concrete gain. The hand-written unit carried an
explicit `SJEL_PERSONAL_ROOT` with a comment explaining that launchd gives a job none of the
shell's exports. True — and unnecessary once the job runs through `tools/service-runner.sh`, which
sources `tools/lib/paths.sh`, which exports it. One fewer machine-local value copied by hand into
a generated file.
