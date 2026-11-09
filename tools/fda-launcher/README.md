# Signed macOS helper

`sjel fda-launcher` manages the helper used by capabilities whose service manifest declares
`full_disk_access = "true"`. The helper remains a separate signed executable so granting it
Full Disk Access does not grant that permission to every invocation of a general-purpose shell.

The installed executable lives at `$HOME/Library/Application Support/Sjel/helpers/sjel-fda-launcher`,
with a sibling receipt containing source and signed-binary hashes. Installation is independent
of the private overlay and the checkout's build output. `tools/fda-launcher/install path` owns
that path; persistence generation queries it rather than maintaining a second path rule.

## Commands

| Command | Effect |
|---|---|
| `sjel fda-launcher` or `status` | Read-only signature, source freshness, and receipt check; nonzero if missing, stale, modified, or unmanaged |
| `sjel fda-launcher install` | Build and sign a missing or stale helper; a current installation is untouched |
| `sjel fda-launcher verify` | Run a disposable launchd job that attempts to enumerate the protected Mail directory; return only success/failure, never directory contents |
| `sjel fda-launcher migrate` | Install, prove access, switch installed Sjel jobs, check loaded jobs, and remove the former overlay executable |
| `sjel fda-launcher path` | Print the managed executable path |

Installation requires a real code-signing identity and uses `tools/codesign-binary.sh` in strict
mode. It refuses unowned files, symlinks, and modified installations. Source changes during a
build abort publication. A lock excludes concurrent installers, and candidates are signed and
verified before replacing the executable. Local compilation follows the runtime power policy.
Re-running `install` reconciles source updates; no private build or signing script is needed.
Doctor reports helper freshness when an enabled capability declares Full Disk Access.

## Migration and permission

macOS permission is verified, not inferred from a valid signature or a terminal invocation.
`verify` runs the actual installed helper under launchd, outside a terminal's responsibility
chain. It does not create or modify Mail data. An absent protected directory leaves access
unverified rather than manufacturing a passing probe.

If macOS requires a new grant, add the managed executable under System Settings → Privacy &
Security → Full Disk Access, then rerun `sjel fda-launcher migrate`. No existing job is changed
before the probe passes. The former executable remains until installed and loaded jobs no
longer use it. A failed job switch restores the changed job files and reloads them; if rollback
fails, the command preserves its private temporary backup and reports the location.

The receipt proves installation integrity and source freshness, not a continuing permission
grant. `verify` is the live permission check. Migration does not move personal configuration or
capability data, and it does not publish installed executables.

## Checks

`tools/fda-launcher.test.sh` uses isolated homes and planted build, signing, and launchd commands.
It covers idempotence, overwrite refusal, mandatory signing, the access gate, failed-job rollback,
and removal only after a successful switch. No test requests a real privacy grant or starts a
real capability. Real permission and service health must be checked separately during rollout.
