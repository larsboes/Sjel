# Managed Pi configuration

The [main plan](../../../PLAN.md#pi-daily-use-and-managed-configuration) owns the rollout
decision. `sjel pi` manages selected Pi settings separately from Pack registrations, sessions
and authentication. It plans by default; nothing changes until an explicit `apply`.

[`settings.base.json`](settings.base.json) disables installation telemetry, analytics and cache
warming, and shows cache-miss notices. Cache warming is disabled because Pi can otherwise issue
background model requests. No provider, model, reasoning level or keymap is imposed by the
public baseline. These keys were checked against the installed Pi settings schema.

An optional private `<overlay>/config/pi/settings.fragment.json` selects additional supported
settings or overrides public defaults. Keybindings use [`keybindings.base.json`](keybindings.base.json)
and `<overlay>/config/pi/keybindings.fragment.json`. The public keymap is empty; existing keys
remain user-owned until explicitly selected in a fragment. Do not put credentials, session
content or resource-registration arrays in either fragment.

The target is `$PI_CODING_AGENT_DIR`, otherwise `~/.pi/agent`. Pi itself does not support the
Sjel Pack adapter's legacy `PI_SETTINGS_FILE` override; the settings command rejects it to
avoid configuring a file Pi will not read. The Pack adapter still accepts that explicit override
for its existing fixture/deployment contract and now follows `PI_CODING_AGENT_DIR` by default.

## Lifecycle and ownership

```sh
sjel pi plan
sjel pi apply
sjel pi check
sjel pi rollback
```

Apply records each owned key's first prior value and latest Sjel-applied value. It preserves
unowned settings, including `skills`, `extensions`, `packages`, prompts and themes. Removing a
key from the desired configuration releases it and restores its prior value. Rollback restores
all pre-management owned values and releases ownership; it does not undo just the latest apply.
Absent keys remain distinct from explicit JSON `null`.

Later applies and rollback refuse detectable changes to managed values. Resolve those changes
in the owning baseline or private fragment before trying again; there is no force flag that
silently discards interactive changes. For object-valued settings, the complete top-level
object is tracked conservatively: existing extra fields are retained on adoption, and changes
inside that object are treated as drift. Select only the objects you intend Sjel to manage.

Writes use a private ownership ledger and pending transaction beside the target. A pending
transaction records managed values only; it never copies the entire settings file. A later
operation classifies the live managed values as before, after or conflicting and recovers only
when that evidence is unambiguous. Plan and check are read-only. Reports name keys and actions,
not configuration values.

The lock coordinates Sjel's own writes. Pi and editors do not share that protocol, so apply or
rollback while those settings writers are idle. Drift checks and a final reread detect changes,
but filesystem rename does not provide a compare-and-swap guarantee against arbitrary writers.
Neither rollback nor removal touches sessions, provider authentication or work an agent performed.

Settings deployment is not a sandbox. The evaluator's isolated SDK configuration and OrbStack
candidate-code boundary remain separate from daily Pi configuration. The controlled comparison
is documented in the [agent benchmark guide](../../../research/benchmarks/agent-harness/README.md).
