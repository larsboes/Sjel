# sjel-status

Local status and lifecycle gateway for enabled capabilities. It reads the service registry, aggregates declared health surfaces, exposes runtime state and proxies capability API requests.

## Why this shape: the manifest is the registry

The process derives capability names, ports and health paths from `service.toml` manifests rather than maintaining a second list. Health reports distinguish an unavailable required autostart service from a normally stopped on-demand service.

The gateway binds to loopback. Lifecycle routes can start or stop processes, so reachability is restricted to the local machine; deployment authentication also protects routes when an inbound token is configured.

## Run

```sh
tools/service-runner.sh start sjel-status
```

The live HTTP contract is available at `GET /api/sjel-status/routes`.

## Considered and declined

- **No duplicate capability registry.** The generated registry output is the source for names and service metadata.
- **No remote unauthenticated process control.** A broader network bind requires an authentication boundary appropriate to that reach.
- **No upstream dependency status endpoint.** Dependency verdicts have their own registry and maintenance surface.
