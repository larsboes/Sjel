# sjel-containment

Agent containment by provider: macOS seatbelt profile generator and loopback egress proxy.

## Surface

```rust
use sjel_containment::{classify_ring, provider_hosts, ContainedCommand, EgressPolicy, EgressProxy, Ring, SeatbeltProfile};
```

CLI launcher:
```bash
sjel-contain <harness-bin> --provider <name> --project <dir> [-- <harness-args...>]
```

The ring and the admitted hosts come from the provider's entry in `providers.toml` (`ring`,
`hosts`), read through `sjel_inference::ReviewedProvidersList`.

A `trusted` provider runs the harness directly. Every other provider runs under
`sandbox-exec` in dev mode: `SJEL_STATUS_URL` names the demo node (`sjel demo up`),
`SJEL_ENV=dev`, and `SJEL_DEMO_TOKEN` carries the demo node's throwaway token, sent as
`X-Sjel-Token`. The overlay and the Keychain are closed, so no live token is reachable.

## Why this shape: OS sandbox enforcement over cooperative agent policies

A cooperative in-process command classifier cannot prevent unauthorized filesystem access, secret exfiltration, or unsanctioned network communication.

Operating system kernel enforcement via macOS `/usr/bin/sandbox-exec` guarantees that untrusted providers and harnesses execute strictly within the designated project directory.

## Why this shape: Loopback egress proxy with strict domain allowlists

Contained agents require network access to their designated model provider and standard language package registries, but must be barred from reaching other external endpoints or local network services.

Directing traffic through a loopback HTTP/HTTPS CONNECT proxy (`127.0.0.1:<port>`) enforces host allowlists and records all connection attempts and refusals.
