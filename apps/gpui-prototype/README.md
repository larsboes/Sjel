# Sjel GPUI prototype

An isolated evaluation app for a possible native desktop surface. It is intentionally outside
the production Cargo workspace: it does not change Sjel's shipping UI, capability processes,
or production dependency lockfiles.

The app has two native pages. **Home** shows what needs you now: agent writes that wait for
approval, kept-running capabilities that do not answer, calendar entries still ahead today and
tomorrow, and the recent agent writes. **Capabilities** lists every capability with start and
stop. Both read the shell (`sjel-status`) every 10 seconds with the inbound token, through the
same endpoints as the menu-bar app (`src/shell.rs`). The token comes from the login Keychain
item `sjel-inbound-token` that `tools/setup-inbound-auth.sh` writes. The app reads it through
`/usr/bin/security`, which that script puts on the item's access list. The token stays in the
app process and never reaches the browser or the log.

The app and the web dashboard share one set of destinations. `src/route.rs` copies the paths and
labels of `dashboard/src/lib/nav.ts`, and a test fails when the two differ. A page with a native
view opens here. Every other destination opens the dashboard in the browser, logged in through a
single-use ticket. `sjel://<path>` links follow the same rule: `sjel://capabilities` opens the
native page, `sjel://calendar` opens the dashboard at `/calendar`.

| Key | Action |
|---|---|
| ⌘K | Search pages and dashboard destinations |
| ⌘1, ⌘2 | Home, Capabilities |
| ⌘R | Refresh |
| ⌘D | Open the current page in the dashboard |
| J / K, ↓ / ↑ | Select the next or previous row |
| ↩ | Run the selected row's main action: Allow, Start, or open the calendar in the dashboard |
| ⌘⌫ | Run the selected row's costly action: Deny or Stop |
| Esc | Clear the selection |

A selected row shows the actions that a mouse sees on hover, for example Stop.

The default endpoint comes from `capabilities/sjel-status/service.toml`. `SJEL_STATUS_URL` can
point to another loopback port. The app refuses any endpoint that is not `http` on 127.0.0.1 or
localhost.

```sh
cd apps/gpui-prototype
cargo run          # development
./install          # ~/Applications/Sjel Desktop.app, opened by the menu bar's Open App
```

`./install` also registers the `sjel://` scheme. Run it again after a change.

## Testing without taking focus

These checks run while you keep working in other apps. None of them activates the app.

| Command | What it checks |
|---|---|
| `cargo test` | Routes, calendar rules, the shell client, and keyboard navigation in a headless GPUI window |
| `./preview [page] [light\|dark]` | Screenshot of the installed app, through Interceptor |
| `./bench-scroll [page]` | CPU cost of one scroll frame, from the app's own scroll run |

`preview` and `bench-scroll` show the window for a few seconds in the bottom-right corner,
above other apps. GPUI paints nothing while another window covers it, so a hidden window cannot be
measured. Run `./install` first: both scripts use the installed release build.

| Variable | Effect |
|---|---|
| `SJEL_GPUI_NO_ACTIVATE=1` | Start and follow `sjel://` links without taking focus |
| `SJEL_GPUI_FLOAT=1` | Open the window in the bottom-right corner, above other apps |
| `SJEL_GPUI_OPEN=sjel://…` | Open this destination after start |
| `SJEL_GPUI_APPEARANCE=light\|dark` | Preview a palette without changing the system setting |
| `SJEL_GPUI_OPAQUE=1` | Draw an opaque window instead of the blurred one |
| `SJEL_GPUI_FRAMES=<file>` | Log the timings of each frame (`src/frames.rs`) |
| `SJEL_GPUI_BENCH_SCROLL=1` | Scroll the page down and up, then quit |

The app icon comes from `brand/sjel-icon-maskable.svg`. `Resources/make-icon` builds
`Resources/AppIcon.icns` from it. Run it again after a change to the brand mark.

The app uses the components of the pinned `gpui-kit = 0.6.0` facade (command palette, dialog,
notification, button, tag, kbd) and adds no second GPUI dependency. Its lockfile resolves one
`gpui-pre` family. GPUI is still pre-1.0, so this is not a production dependency decision.

GPUI's blurred window is not Apple's Liquid Glass. The resolved public API has no general AppKit
child-view host, so the prototype keeps one narrow, opt-in bridge in `src/macos_glass.rs`. Set
`SJEL_GPUI_GLASS=1` on macOS 26+ to float the sidebar on a native `NSGlassEffectView`, inset 8
points with rounded corners, the traffic lights inside it. Glass refracts what is behind it, and
GPUI draws everything into one view, so the bridge puts the glass and a tint strip behind GPUI's
view, and GPUI leaves the sidebar unpainted. Clicks and keys still reach GPUI. Runtime class
lookup gates it; without the class, the app draws its normal sidebar. `SJEL_GPUI_GLASS=1
./preview home dark` shows it.

Seen on macOS 27: the panel reads as a macOS 26 sidebar in light mode. In dark mode it is close
to black, darker than the page. In light mode a faint seam shows where the AppKit tint strip
meets GPUI's page tint, because GPUI's translucent colours blend lighter over an AppKit layer
than over its own. This is an experiment, not a supported GPUI hosting API.

## Evaluation scope

- Does a GPU-native Rust shell feel like a better desktop home for Sjel?
- Is the build and macOS packaging burden acceptable beside Tauri, iOS, and the Swift menu-bar
  companion?
- Can richer views use the existing HTTP contracts without adding direct database access or
  exposing credentials to the client?

The app is an evaluation surface, not a dashboard port. A destination gets a native page when
the native form does something the browser cannot do as well. Until then, the palette opens it
in the dashboard.
