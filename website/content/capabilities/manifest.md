# Capability manifest specification (`service.toml`)

Copy the relevant fields from [`schemas/service.toml.example`](../../schemas/service.toml.example).
That file is the complete field reference. Manifests use flat, single-line TOML fields, including
argv arrays for commands. Do not wrap them in a `[service]` or `[backup]` table.

## Schema reference

<a id="service-section"></a>

### Root fields

There is no `[service]` section in the supported format. The runner reads these fields at the
root of the manifest:

| Field | Shape | Use |
| --- | --- | --- |
| `name` | string | Unique service identity |
| `kind` | string | `process`, `container`, or `data`; defaults to `container` |
| `command`, `build` | string arrays | Process argv and optional build argv |
| `port` | string | Process port, exported by the runner as `SJEL_PORT` |
| `requires` | string array | Runtime capability prerequisites |
| `autostart` | string `"true"` | Persistent service, mutually exclusive with `schedule` |
| `schedule` | string | Periodic process interval, such as `30m`, `6h`, or `1d` |
| `full_disk_access` | string `"true"` | macOS scheduled process uses the FDA launcher |

Container fields include `image`, `tag`, `ports`, and `volumes`. Data capabilities declare identity
and backup fields. The schema describes permitted combinations, readiness, panels, proxy routes,
and backup and freshness contracts.

### Example process manifest

This is a minimal illustrative shape; choose a unique name and port for a real capability:

```toml
kind = "process"
name = "example"
command = ["target/release/example-server"]
build = ["cargo", "build", "--locked", "--release", "--bin", "example-server"]
port = "8080"
health_path = "/health"
autostart = "true"
```

For working examples, read [punctuality](../../capabilities/punctuality/service.toml), the scheduled
[entities sync](../../capabilities/entities-sync/service.toml), and the data-only
[store](../../capabilities/store/service.toml).

### Manifest rules

1. Check the public and overlay declarations for port collisions and prerequisite names.
2. Keep deployment values and secret references in the overlay. Follow the schema's path rules
   for mounts and environment files.
3. Validate the declaration with `tools/check-service-tomls.sh` and
   `tools/check-manifest-integrity.sh`. Doctor also reports manifest and port checks.

See [service persistence](../operations/persistence.md) for how declarations become host units.
