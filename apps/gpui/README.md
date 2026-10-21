# Sjel GPUI

A native desktop surface for Sjel. It is intentionally outside the production Cargo workspace: it
does not change Sjel's shipping UI, capability processes, or production dependency lockfiles.

The app has four pages. **Home** shows what needs you now: agent writes that wait for approval,
kept-running capabilities that do not answer, calendar entries still ahead today and tomorrow, and
the recent agent writes. **Capabilities** lists every capability with start and stop. **Projects**
lists projects, tasks and notes, draws tasks as a table with due and priority edited in place, and
edits a note as blocks (`src/editor.rs`, `src/blocks.rs`). **Ask** runs a multi-turn capability
session in the window.

Every block keeps its markdown verbatim, so a save writes back exactly what the editor did not
change; a note that does not round-trip opens read-only. A save follows each pause in typing and
sends the content it started from, so the capability refuses an edit made over a newer version.

The app reads the shell (`sjel-status`) over loopback with the inbound token, through the same
endpoints as the menu-bar app (`core/src/shell.rs`). The token comes from the login Keychain item
`sjel-inbound-token` that `tools/setup-inbound-auth.sh` writes, read through `/usr/bin/security`,
which that script puts on the item's access list. The token stays in the app process and never
reaches the browser or the log.

`GET /api/sjel-status/events` pushes the health and agent answers when they change
(`capabilities/sjel-status/src/status/events.rs`), so the open window reads them once and then
listens: one held connection and one calendar read per 10 seconds, instead of three reads. A shell
without that stream keeps the timer.

A project's records arrive without their content, and the note editor reads the one record it
opens. A task's done, due and priority go to the capability, which owns the frontmatter edit and
answers with the record's new frontmatter, so the table patches one row instead of reading the
project again. `src/data/` is the only module that talks to the shell: a read keeps its last good
value while it retries and never runs twice at once, and a write toasts its outcome and then lets
the caller re-read only what it changed.

Where a local `axon.db` is readable, `src/data/` answers projects, tasks, calendar and record
reads from it and falls back to the shell otherwise, over one LibSQL handle opened once per process
and shared by every clone. Holding the connection rather than reopening it is the point: opening
costs 0.04 ms, but a fresh connection starts on an empty SQLite page cache, so its first query
costs 0.58 ms against 0.03 ms on a handle that already ran one, and a poll runs several. How the
numbers were measured: `cargo test --manifest-path apps/gpui/Cargo.toml --bin sjel-gpui
holding_one_connection -- --ignored --nocapture`, which times both paths against this machine's
database.

Going back to a note opened in the last five minutes costs no read at all. `src/data/notes.rs`
keeps the 16 notes opened most recently, most recent first, so tabbing back to one is a memory hit
and the frame the click lands in: 0.42 µs at p50 for a 21 KB note, in a release build. Every write
the app makes to a record drops that record, so a kept note is never older than the last write, and
the editor leaves the body it saved in place of the one it opened.

The app and the web dashboard share one set of destinations. `core/src/route.rs` copies the paths
and labels of `dashboard/src/lib/nav.ts`, and a test fails when the two differ. A page with a
native view opens here. Every other destination opens the dashboard in the browser, logged in
through a single-use ticket. `sjel://<path>` links follow the same rule: `sjel://capabilities`
opens the native page, `sjel://calendar` opens the dashboard at `/calendar`.

## Why this shape: GPUI for every client

GPUI is the goal for every Sjel client, on the Mac and on iOS. AppKit or UIKit draws the
operating-system chrome, and GPUI draws the content. The workspace pins one GPUI snapshot,
`gpui-pre =0.3.9` with gpui-kit 0.7.3, and uses the Longbridge `gpui-pre-mobile` fork on iOS.
Sources: `upstreams.toml`, the mobile page of gpui-kit's website docs, and `src/macos_glass.rs`.

It is a stack decision, not a speed fix: Sjel's waits come from services, not from rendering. A
release build starts in one frame under 10 ms with about 100 MB resident — against about 220–300 MB
for a Tauri and WKWebView process pair — and the Swift menu bar keeps system-wide ⌃⌥Space, so global
invocation pays neither a second resident process nor a cold-launch hop. How the numbers were
measured: `SJEL_GPUI_FRAMES` and `ps -o rss=` in `src/frames.rs`.

The move has four steps:

0. **Done.** One Cargo workspace builds GPUI, gpui-kit and gpui-mobile, and shows one list fed over
   loopback HTTP on the Mac and in the iOS Simulator. Had this failed, the iPhone decision would
   open again.
1. **Done.** A command palette, measured for start time and resident memory against Tauri.
2. **In progress.** The dashboard's routes, ported into this app: `context`, `calendar` and `inbox`
   first, with the native `capabilities` page as the reference. GPUI draws the charts. The map
   comes last, with `gpui-webview` as the fallback. Home, Capabilities, Projects and Ask are here;
   `/context` is next, and it joins calendar, trips and finance rather than repeating one
   capability's page.
3. Test and review, then decide: switch, keep both, or stop. Svelte and Tauri stay live until that
   review.

## Workspace and the iOS app

This directory is one Cargo workspace with one lockfile, so every member builds against the same
GPUI snapshot (step 0 above).

| Member | What it holds |
|---|---|
| `.` | The Mac app |
| `core/` | What both apps share: the destinations, the shell client and a paired device's client. No GPUI types. |
| `ios/` | The iOS app: GPUI content in `src/lib.rs`, the UIKit host in `host/` |

On iOS, UIKit owns the window, the navigation bar and the safe areas. GPUI draws the content in a
child view controller. `gpui-pre-mobile` comes from git rev `c24a1308e1`, because its only release
(0.1.0) pins an older GPUI. That rev also pins `libc =0.2.189`, so the whole workspace uses that
version. The iOS 27 SDK starts only apps with the UIScene life cycle, so the host uses a scene
delegate.

The iOS app has no inbound token: it is a paired device (`core/src/device.rs`), signed with its
Keychain key over TLS pinned to the Mac's LAN listener. `ios/src/lib.rs` says how a simulator joins
until Bonjour discovery and a camera scanner exist. `ios/run-sim` builds the app, runs it on a
headless simulator and prints a screenshot path; it needs Xcode, XcodeGen and the
`aarch64-apple-ios-sim` Rust target, and never opens Simulator.app.

## Keys

| Key | Action |
|---|---|
| ⌘K | Search pages and dashboard destinations |
| ⌘1, ⌘2, ⌘3, ⌘4 | Home, Capabilities, Projects, Ask |
| ⌘R | Refresh |
| ⌘D | Open the current page in the dashboard |
| J / K, ↓ / ↑ | Select the next or previous row |
| ↩ | Run the selected row's main action: Allow, Start, or open the calendar in the dashboard |
| ⌘⌫ | Run the selected row's costly action: Deny or Stop |
| Esc | Clear the selection |

A selected row shows the actions that a mouse sees on hover, for example Stop.

In a note, ↩ starts a block and ⇧↩ a new line; ⌫ at a block's start turns a list item or heading into
a paragraph, then joins it to the block above; `# `, `- `, `[] `, `1. ` or `> ` typed at a
paragraph's start changes its kind. `⌘S` saves, `⌘[` goes back to the project, and `/` in an empty
paragraph opens the block menu (also on the `⋮⋮` handle, which also drags). Only the block with the
caret shows its markdown; every other block shows its formatting with the markers hidden
(`src/inline.rs`).

A project page shows its tasks as a table above its notes (`src/tasks.rs`): open first, then due
date, then priority. A click on the due date edits it (↩ saves, leaving keeps the old date), a click
on the priority cycles P1, P2, P3, and the last row adds a task. `SJEL_GPUI_NO_SAVE` covers the note
editor only; the table always writes.

## Running it

The default endpoint comes from `capabilities/sjel-status/service.toml`. `SJEL_STATUS_URL` can point
to another loopback port. The app refuses any endpoint that is not `http` on 127.0.0.1 or
localhost.

```sh
cd apps/gpui
cargo run          # development
./install          # ~/Applications/Sjel Desktop.app, opened by the menu bar's Open App
```

`./install` also registers the `sjel://` scheme. Run it again after a change.

## Testing without taking focus

These checks run while you keep working in other apps. None of them activates the app.

| Command | What it checks |
|---|---|
| `cargo test --workspace` | Routes, calendar rules, the shell client and its stream parser, and keyboard navigation in a headless GPUI window |
| `./preview [page] [light\|dark]` | Screenshot of the installed app, through Interceptor |
| `./bench-scroll [page]` | CPU cost of one scroll frame, from the app's own scroll run |
| `SJEL_GPUI_BENCH_NOTE=<record path> ./bench-scroll projects` | CPU cost of one keystroke in that note; saving is off |
| `ios/run-sim` | Screenshot of the iOS app on a headless simulator |

`preview` and `bench-scroll` show the window for a few seconds in the bottom-right corner, above
other apps. GPUI paints nothing while another window covers it, so a hidden window cannot be
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

GPUI's blurred window is not Apple's Liquid Glass: the public API has no general AppKit child-view
host, so one narrow bridge (`src/macos_glass.rs`) puts an `NSGlassEffectView` and a tint strip
behind GPUI's view and leaves the sidebar unpainted. In light mode a faint seam shows where the
strip meets GPUI's tint, because GPUI's translucent colours blend lighter over an AppKit layer than
over its own. `SJEL_GPUI_NO_GLASS=1` draws the normal sidebar on any macOS. This is an experiment,
not a supported GPUI hosting API.

## Evaluation scope

- Does a GPU-native Rust shell feel like a better desktop home for Sjel?
- Is the build and macOS packaging burden acceptable beside Tauri, iOS, and the Swift menu-bar
  companion?
- Can richer views use the existing HTTP contracts without adding direct database access or
  exposing credentials to the client?

The app is an evaluation surface, not a dashboard port. A destination gets a native page when the
native form does something the browser cannot do as well. Until then, the palette opens it in the
dashboard.
