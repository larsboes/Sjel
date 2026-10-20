# Sjel GPUI prototype

An isolated evaluation app for a possible native desktop surface. It is intentionally outside
the production Cargo workspace: it does not change Sjel's shipping UI, capability processes,
or production dependency lockfiles.

The app has three native pages. **Home** shows what needs you now: agent writes that wait for
approval, kept-running capabilities that do not answer, calendar entries still ahead today and
tomorrow, and the recent agent writes. **Capabilities** lists every capability with start and
stop. **Notes** opens a project's notes from the `projects` capability and edits them as blocks
(`src/notes.rs`, `src/blocks.rs`). Each block keeps its markdown verbatim, so a save writes back
exactly what the editor did not change; a note that does not round-trip opens read-only. A save
follows each pause in typing and sends the content it started from, so the capability refuses an
edit made over a newer version. Home and Capabilities read the shell (`sjel-status`) every 10 seconds with the inbound token, through the
same endpoints as the menu-bar app (`core/src/shell.rs`). The token comes from the login Keychain
item `sjel-inbound-token` that `tools/setup-inbound-auth.sh` writes. The app reads it through
`/usr/bin/security`, which that script puts on the item's access list. The token stays in the
app process and never reaches the browser or the log.

The app and the web dashboard share one set of destinations. `core/src/route.rs` copies the paths and
labels of `dashboard/src/lib/nav.ts`, and a test fails when the two differ. A page with a native
view opens here. Every other destination opens the dashboard in the browser, logged in through a
single-use ticket. `sjel://<path>` links follow the same rule: `sjel://capabilities` opens the
native page, `sjel://calendar` opens the dashboard at `/calendar`.

## Why this shape: GPUI for every client

GPUI is the goal for every Sjel client, on the Mac and on iOS (decided 2026-10-11, revised
2026-10-15). AppKit or UIKit draws the operating-system chrome, and GPUI draws the content. The
workspace pins one GPUI snapshot, `gpui-pre =0.3.9` with gpui-kit 0.7.3, and uses the Longbridge
`gpui-pre-mobile` fork on iOS. Sources: `upstreams.toml`, the mobile page of gpui-kit's website docs,
and `src/macos_glass.rs`.

The move has four steps:

0. One Cargo workspace builds GPUI, gpui-kit and gpui-mobile, and shows one list fed over
   loopback HTTP on the Mac and in the iOS Simulator. If this fails, the iPhone decision opens
   again.
1. A command palette, measured for start time and resident memory against Tauri.
2. The dashboard's routes, ported into this app: `context`, `calendar` and `inbox` first, with
   the native `capabilities` page as the reference. GPUI draws the charts. The map comes last,
   with `gpui-webview` as the fallback.
3. Test and review, then decide: switch, keep both, or stop. Svelte and Tauri stay live until
   that review.

The editor spike of 2026-10-18 built block editing, inline formatting,
a `/` menu, drag reorder and a task table without a fork, at 1.6 ms CPU per keystroke on a
1,740-line note. Three GPUI gaps needed outside workarounds: no inline styling outside
code-editor mode, fixed textarea padding, and key-context precedence.

Step 1 measured on 2026-10-20 (`Sjel Desktop.app` release build, 31.3 MB binary, via
`SJEL_GPUI_FRAMES` and `ps -o rss=`): first workspace frame builds in 2.99 ms and finishes
painting at 8.54 ms (`8,537 µs` from `FrameProbe` init), with 101.1 MB resident memory
(`103,536 KB` RSS) versus 109.1 MB (`111,760 KB` RSS) for the always-resident Swift menu-bar app
(`Sjel.app`, 0.94 MB binary) and ~220–300 MB for a Tauri + WKWebView process pair. Decision:
⌘K searches pages and dashboard destinations inside `Sjel Desktop.app` when the GPUI window is
open, while system-wide ⌃⌥Space stays on `Sjel.app`'s already-resident `QuickAsk` `NSPanel` so
global invocation pays neither a second 101 MB resident process nor a cold-launch hop.

This is a stack decision, not a speed fix: on 2026-10-09, Sjel's waits came from services, not
from rendering. It replaces the 2026-10-09 split that gave GPUI only the keyboard surfaces.

## Workspace and the iOS app

This directory is one Cargo workspace with one lockfile, so every member builds against the
same GPUI snapshot (step 0 above).

| Member | What it holds |
|---|---|
| `.` | The Mac app |
| `core/` | What both apps share: the destinations and the shell client. No GPUI types. |
| `ios/` | The iOS app: GPUI content in `src/lib.rs`, the UIKit host in `host/` |

On iOS, UIKit owns the window, the navigation bar and the safe areas. GPUI draws the content in
a child view controller. `gpui-pre-mobile` comes from git rev `c24a1308e1`, because its only
release (0.1.0) pins an older GPUI. That rev also pins `libc =0.2.189`, so the whole workspace
uses that version. The iOS 27 SDK starts only apps with the UIScene life cycle, so the host
uses a scene delegate.

Step 0 shows the destinations and one read of the shell's public `/health` over loopback. The
iOS app has no token yet: the inbound token lives in the Mac's login Keychain. Device pairing
is the planned way in.

`ios/run-sim` builds the app, runs it on a headless simulator and prints a screenshot path. It
needs Xcode, XcodeGen and the `aarch64-apple-ios-sim` Rust target. It never opens
Simulator.app.

| Key | Action |
|---|---|
| ⌘K | Search pages and dashboard destinations |
| ⌘1, ⌘2, ⌘3 | Home, Capabilities, Notes |
| ⌘R | Refresh |
| ⌘D | Open the current page in the dashboard |
| J / K, ↓ / ↑ | Select the next or previous row |
| ↩ | Run the selected row's main action: Allow, Start, or open the calendar in the dashboard |
| ⌘⌫ | Run the selected row's costly action: Deny or Stop |
| Esc | Clear the selection |

A selected row shows the actions that a mouse sees on hover, for example Stop.

In a note, ↩ starts a new block and ⇧↩ a new line in the block. ⌫ at the start of a block turns
a list item or heading into a paragraph, then joins the paragraph to the block above. ↑ and ↓
at a block's edge move to the next block. `# `, `- `, `[] `, `1. ` or `> ` at the start of a
paragraph changes its kind. ⌘S saves at once, ⌘[ goes back to the project. Only the block with
the caret shows its markdown; every other block shows bold, italic, code, strikethrough and links
formatted, with the markers hidden (`src/inline.rs`). A click on formatted text puts the caret at
the same place in the markdown.

`/` in an empty paragraph opens a menu of block kinds; type to filter, ↑ and ↓ to pick, ↩ to
apply, Esc to close. On hover, a `⋮⋮` handle left of a block opens the same menu to turn the block
into another kind, and dragging it moves the block. ⌘⇧↑ and ⌘⇧↓ move the block with the caret.

A project page shows its tasks as a table above its notes (`src/tasks.rs`): open tasks first, by
due date, then priority. The checkbox marks a task done through the capability, which also sets
`status:` and `completed:`. A click on the due date edits it (↩ saves, leaving the field keeps
the old date), a click on the priority cycles P1, P2, P3, and the last row adds a task. Due and
priority change only their frontmatter line and save with the content they started from.
`SJEL_GPUI_NO_SAVE` covers the note editor only; the task table always writes.

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
| `cargo test --workspace` | Routes, calendar rules, the shell client, and keyboard navigation in a headless GPUI window |
| `./preview [page] [light\|dark]` | Screenshot of the installed app, through Interceptor |
| `./bench-scroll [page]` | CPU cost of one scroll frame, from the app's own scroll run |
| `SJEL_GPUI_BENCH_NOTE=<record path> ./bench-scroll projects` | CPU cost of one keystroke in that note; saving is off |
| `ios/run-sim` | Screenshot of the iOS app on a headless simulator |

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
| `SJEL_GPUI_NO_GLASS=1` | Draw the normal sidebar instead of the glass one |
| `SJEL_GPUI_FRAMES=<file>` | Log the timings of each frame (`src/frames.rs`) |
| `SJEL_GPUI_BENCH_SCROLL=1` | Scroll the page down and up, then quit |
| `SJEL_GPUI_NO_SAVE=1` | Edit notes without writing them, to try the editor on real notes |
| `SJEL_GPUI_BENCH_NOTE=<record path>` | Open that note with saving off, type 240 characters, then quit |

The app icon comes from `brand/sjel-icon-maskable.svg`. `Resources/make-icon` builds
`Resources/AppIcon.icns` from it. Run it again after a change to the brand mark.

The app uses the components of the pinned `gpui-kit = 0.7.3` facade (command palette, dialog,
notification, button, tag, kbd) and adds no second GPUI dependency. Its lockfile resolves one
`gpui-pre` family. GPUI is still pre-1.0, so this is not a production dependency decision.

GPUI's blurred window is not Apple's Liquid Glass. The resolved public API has no general AppKit
child-view host, so the prototype keeps one narrow bridge in `src/macos_glass.rs`. On macOS 26+
the sidebar floats on a native `NSGlassEffectView`, inset 8 points with rounded corners, the
traffic lights inside it. Glass refracts what is behind it, and
GPUI draws everything into one view, so the bridge puts the glass and a tint strip behind GPUI's
view, and GPUI leaves the sidebar unpainted. Clicks and keys still reach GPUI. Runtime class
lookup gates it; without the class, the app draws its normal sidebar. `SJEL_GPUI_NO_GLASS=1`
draws the normal sidebar on any macOS, also through `./preview`.

Seen on macOS 27: the panel reads as a macOS 26 sidebar in both appearances, one step darker
than the page in dark mode. In light mode a faint seam shows where the AppKit tint strip
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
