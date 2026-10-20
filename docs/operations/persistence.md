# Service persistence (launchd & systemd)

The runner derives persistence from `service.toml`: an autostart process gets a watchdog; a
scheduled process gets a periodic job. Declaring both is an error. An on-demand process gets no
persistent unit. Containers use the runtime's restart policy rather than a second watchdog.

## Platform implementations

The [persistence renderer](../../tools/sjel-cli/src/persist.rs) writes macOS user agents under
`~/Library/LaunchAgents/com.sjel.<name>.plist`. Linux uses user units under
`${XDG_CONFIG_HOME:-~/.config}/systemd/user/`, with `sjel-<name>.service` and, for scheduled jobs,
`sjel-<name>.timer`. These filenames are generated contracts, including the retained `sjel` prefix.

## Plist structure & invariants

Edit the manifest rather than a generated unit. `persistence-status` renders the expected unit
from [templates](../../tools/templates/) and compares it with the installed bytes; a manual edit
or an old rendering appears stale. `install-persistence` installs the current declaration and
can trigger a scheduled run.

## Lifecycle commands

```bash
tools/service-runner.sh status <capability>
tools/service-runner.sh start <capability>
tools/service-runner.sh stop <capability>
tools/service-runner.sh resume <capability>
```

`stop` creates a maintenance hold so a watchdog cannot immediately undo it. `resume` lifts that
hold and starts the service. `restart` also rebuilds when a build command is declared. Data-only
capabilities reject process lifecycle commands.

Inspect or update a capability's persistence with:

```bash
tools/service-runner.sh persistence-status <capability>
tools/service-runner.sh install-persistence <capability>
```

`tools/service-runner.sh persistence` reports the whole enabled set. Run the runner without
arguments for its complete command reference.

## Backup interruption and exclusion

The API and scheduler claim a capability atomically in the backup ledger. Direct
`tools/backup.sh` runs also take a deployment-local kernel lock; overlapping runs are refused
before they can share a maintenance hold. See the
[backup contract](../../capabilities/backup/README.md#concurrent-runs-and-interruption).

On an ordinary failure or handled signal, the script attempts to resume a capability it held and
remove its unfinished destination archive. A foreground command can delay signal handling.
SIGKILL and power loss cannot run cleanup: inspect any remaining maintenance hold and partial
archive before resuming or removing anything. Locks do not coordinate separate machines.

## Full disk access invariant

For a scheduled macOS process with `full_disk_access = "true"`, the renderer invokes the signed
`sjel-fda-launcher`. Install and grant that helper the required access using
[`tools/fda-launcher`](../../tools/fda-launcher/). A generated job must not assume an interactive
terminal's privacy permissions.

## Container image identity

A manifest's `image` and `tag` identify the declared channel. Runner status prints that declaration;
it does not establish the running build. Inspect the running container's image ID, then inspect
that image's repository digest. A newly pulled image under the same tag may differ from the image
an existing container is running.

Argument drift covers ports, mounts, capabilities, and network settings; it does not detect a
moved image digest. The [container-refresh capability](../../capabilities/container-refresh/README.md)
pulls declared channels and recreates running containers when the digest changes. Held or stopped
containers remain stopped.

[`tools/backup.sh`](../../tools/backup.sh) records the running container's digest as `image_digest`
in the archive's `axon-backup.toml`, falling back to its local image ID if no repository digest is
available. [`tools/restore.sh`](../../tools/restore.sh) prints that historical identity. It does not
reject a restore solely because today's digest differs: rolling channels are expected to move.
Compare the identities when assessing compatibility. An older archive or a backup with no
answering container can have an unrecorded digest.
