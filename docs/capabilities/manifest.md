# Capability Manifest Specification (`service.toml`)

Every capability must provide a `service.toml` at its root directory describing its identity, network ports, lifecycle commands, and dependencies.

## Schema Reference

### `[service]` Section

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `name` | string | Yes | Unique capability identifier matching directory name |
| `kind` | string | Yes | Execution mode: `process`, `container`, or `data` |
| `description` | string | Yes | 1-2 sentence specification of unit purpose |
| `command` | string | For `process` | Shell execution command to start the service |
| `port` | integer | For network svcs | Unique TCP port on `127.0.0.1` |
| `schedule` | string | Optional | Periodic trigger cadence (e.g. `24.0h`, `12.0h`, `30m`) |
| `requires` | array[string]| Optional | List of prerequisite capabilities that must start first |
| `full_disk_access` | boolean | Optional | Whether binary requires macOS Full Disk Access launcher |

### Example Process Manifest

```toml
[service]
name = "punctuality"
kind = "process"
port = 8089
description = "DB journey punctuality and delay tracking service"
command = "target/release/punctuality-server"
requires = ["transit"]

[backup]
contract = "punctuality"
retention_days = 30
```

### Manifest Rules

1. **Port Uniqueness**: Every declared port must be globally unique across both the core repository and active overlays.
2. **Dependency Closure**: All names listed in `requires` must exist as declared capabilities.
3. **No Dynamic Ports**: Ports are static, declared contracts; dynamic ephemeral ports are prohibited.
