# Service Persistence (launchd & systemd)

Sjel capabilities run continuously or periodically via host service managers.

## Platform Implementations

- **macOS**: `launchd` via user agents in `~/Library/LaunchAgents/com.sjel.<name>.plist`.
- **Linux**: `systemd` user units via `~/.config/systemd/user/sjel-<name>.service`.

## Plist Structure & Invariants

A standard Sjel launchd agent uses `service-runner.sh`:

```xml
<dict>
  <key>Label</key>
  <string>com.sjel.punctuality</string>
  <key>ProgramArguments</key>
  <array>
    <string>/path/to/Sjel/tools/service-runner.sh</string>
    <string>start</string>
    <string>punctuality</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>StandardOutPath</key>
  <string>/tmp/axon-punctuality-schedule.log</string>
  <key>StandardErrorPath</key>
  <string>/tmp/axon-punctuality-schedule.err</string>
</dict>
```

## Lifecycle Commands

```bash
# Start an enabled service
tools/service-runner.sh start <capability>

# Stop an enabled service
tools/service-runner.sh stop <capability>

# Inspect service state
tools/service-runner.sh status

# Reinstall/sync launchd persistence plists
tools/service-runner.sh install-persistence <capability>
```

## Full Disk Access Invariant

On macOS, binaries needing access to protected databases (e.g. Mail, Messages, Calendar) cannot execute directly via launchd. They are invoked through `sjel-fda-launcher`, a signed helper with granted FDA privileges.
