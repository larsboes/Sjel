# sjel-status

Local status and lifecycle gateway for enabled capabilities. It reads the service registry, aggregates declared health surfaces, exposes runtime state and proxies capability API requests.

## Why this shape: the manifest is the registry

The process derives capability names, ports and health paths from `service.toml` manifests rather than maintaining a second list. Health reports distinguish an unavailable required autostart service from a normally stopped on-demand service.

The primary listener binds to loopback. An optional LAN listener admits paired devices, and a protected Unix socket serves the tailnet proxy. Lifecycle routes are authenticated at these boundaries; the LAN listener is disabled unless deployment configuration enables it.

## Run

```sh
tools/service-runner.sh start sjel-status
```

The live HTTP contract is available at `GET /api/sjel-status/routes`.

## On-demand storage explorer

Systems uses `GET /api/sjel-status/storage-explorer?root=/` to read cached directory allocation.
`POST` to the same route explicitly starts a scan and returns its state without waiting for the
walk. Subsequent GET requests report progress, the last snapshot, scan errors and comparable
previous measurements. Neither a GET request nor capability startup starts a discovery scan.
The existing hourly policy-report history is unchanged.

The storage tool owns measurement: `sjel storage browse` performs one allocation walk per
requested folder. The service runs a prebuilt tool without compiling, permits one discovery
process at a time, bounds its runtime, and keeps a bounded in-memory cache. Snapshots survive
closing and reopening Systems, but not restarting this service. A failed refresh retains its
previous snapshot; partial snapshots remain marked and do not establish growth.

The initial root is the writable volume. Child roots must have appeared in a previous snapshot;
callers cannot submit arbitrary filesystem paths. Navigation reads a cached folder first, and
an unmeasured folder requires its own explicit scan. Allocation totals include policy-managed
paths; they are not cleanup candidates. No file-content reads, deletion or scheduled discovery
are exposed by this route.

## Considered and declined

- **No duplicate capability registry.** The generated registry output is the source for names and service metadata.
- **No remote unauthenticated process control.** A broader network bind requires an authentication boundary appropriate to that reach.
- **No upstream dependency status endpoint.** Dependency verdicts have their own registry and maintenance surface.
