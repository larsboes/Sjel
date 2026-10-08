# Sjel GPUI prototype

An isolated evaluation app for a possible native desktop surface. It is intentionally outside
the production Cargo workspace: it does not change Sjel's shipping UI, capability processes,
or dependency lockfiles.

The first slice is a native shell and a manual `GET /health` check against `sjel-status`. It
reads no personal data and uses no credentials. The default endpoint comes from the
`capabilities/sjel-status/service.toml` manifest. `SJEL_STATUS_URL` can override it for another
loopback port; this prototype rejects non-loopback endpoints.

```sh
cd apps/gpui-prototype
cargo run
```

The check is expected to show **Unavailable** when `sjel-status` is stopped. Start it through
the normal Sjel service runner before evaluating the connected state. GPUI and its platform
crate are pinned to one Zed repository revision because the matching `gpui_platform` crate is
not available from crates.io; the framework remains pre-1.0, so this is not a production
dependency decision.

## Evaluation scope

- Does a GPU-native Rust shell feel like a better desktop home for Sjel?
- Is the build and macOS packaging burden acceptable beside Tauri, iOS, and the Swift menu-bar
  companion?
- Can richer views use the existing HTTP contracts without adding direct database access or
  exposing credentials to the client?

The current screen is a shell and connectivity spike, not a dashboard port. The next meaningful
slice would be a read-only view over a specifically reviewed status API response.
