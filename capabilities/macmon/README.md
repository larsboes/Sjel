# macmon

Apple Silicon hardware metrics served by the adopted [`macmon`](https://github.com/vladkens/macmon) binary. Sjel does not build or maintain the upstream monitor; this capability manifest configures its service endpoint.

## Why this shape: upstream owns the sampler

The upstream binary reads Apple Silicon power counters and exposes JSON at `/json`. Sjel declares how the service runs and its health path, while the separate `tools/sysmon power` command launches the upstream interactive monitor on demand.

## Run

The metrics service is started on demand through `tools/service-runner.sh`. The interactive monitor is available with:

```sh
tools/sysmon power
```

## Considered and declined

- **No sampler implementation in Sjel.** The upstream binary already provides the hardware-specific sampling behavior.
- **No non-Apple-Silicon deployment.** The metrics service has no equivalent hardware counters on other machine types.
