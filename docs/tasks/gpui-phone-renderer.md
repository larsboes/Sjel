---
id: "gpui-phone-renderer"
title: "Move the phone UI to GPUI"
state: "verify"
area: "gpui"
kind: "maintenance"
priority: 4
spec: "docs/architecture/native-ui-boundaries.md"
---

# Move the phone UI to GPUI

## Outcome

The normal phone routes and navigation use a single GPUI and Ely window in the Swift app.
Swift continues to provide the system integrations.

## Acceptance

- Check the shared host on an iPhone for touch, text input, safe areas, background and foreground
  transitions, and return from native system presentation.
- Carry the existing Today, Capture, Projects, and Documents data paths through a read and
  write or offline case on the phone.
- Check People and Places against the cached copy, including search, contact and place cards,
  and switching between the two registers.
- Check Inventory and Wishlist on the phone for search, owned categories, counted budgets,
  and comparison of alternative wishes.
- Check Finance on the phone for balances and spend goals, transaction search and filters,
  subscriptions, and signed candidate review including a failed request.
- Check Calendar on the phone against its replica and read-only copy. Verify the list, Ely
  day grid, free time, creation, move and resize, undo, and sync.
- Check Settings on the phone for pairing, Health sync, room scans, reminders, theme changes,
  and terminal preferences.
- Check Mail on the phone for the cached reader, triage confirmations, offline outbox,
  and Gmail retry and cancel.
- Check Travel, Map, and Interior against the corresponding phone acceptance contracts.
- Check the Scratchpad and Activity panels and Soundscape playback, presets, levels, and pause
  on dismissal on the phone. Scratchpad has no editor in the existing phone workflow.
- Verify the Agent transcript and approval flow on the phone, including blocked calls and
  keeping a draft until the Mac accepts it.
- Check Terminal authentication, keyboard input, selection, clipboard, and touch controls.
- Check the tab bar, panels, back stack, menus, and deep links on the device.

## Evidence

- The normal Today, Capture, Projects, and Documents routes use a single GPUI application
  and window. Projects uses the shared GPUI view and reads registry objects through the
  phone replica.
- The focused simulator-target build linked the app and XCTest runner. CoreSimulator refused
  connections and Xcode reported no available iPhone. Runtime acceptance is open.
- People and Places reads the existing Mac copy and Inventory and Wishlist uses the paired
  Mac inventory source. The Rust check for the physical device target passes for both routes.
  XCTest covers surface mounting. Feature interactions still require device verification.
- Finance uses the paired projection and signed review calls. The Rust target check and the
  existing host fixture pass. Finance decisions remain pending until acknowledged by the Mac.
- Calendar uses the shared Ely day grid and the existing replica and Mac copy. Settings uses the
  existing Swift-owned actions. The shared GPUI phone window applies the appearance selected in
  Settings to its routes and Ely controls.
- Scratchpad, Activity, and Soundscape use GPUI under the existing panel navigation. Soundscape
  keeps the native audio session, presets, mixer, and analyser.
- Mail uses the existing offline thread cache and signed triage flow with GPUI. The reader
  retains confirmation prompts and skips embedded message images.
- Agent uses the existing signed session stream, prompt acknowledgments, and approval checks
  with GPUI controls.
- USB pairing and Developer Mode both validate. Device Hub lists the iPhone as available.
  The command-line CoreDevice service still times out from the workspace. The paired USB
  installation service confirmed installation of the signed development build. Build 2323
  remained running on the phone after launch. No screen or interaction assertions ran because
  the device automation and screenshot services were unavailable.
- A developer-bundle install and separate device lookup confirmed build 2326 with Calendar,
  Settings, Scratchpad, Activity, and Soundscape.
- Build 2328 added Mail and Agent and was confirmed installed by device lookup. The signed
  archive, iPhone Rust check, desktop NSH compatibility check, and eight host tests passed.
  Launch and interaction checks for build 2328 remain open.
- The final cutover moves Travel, Map, Interior, Terminal, and navigation to the shared window.
  All 18 routes use GPUI, and the iPhone Cargo dependency tree contains no NSH packages.
  The Mac NSH trial target, 23 UI regression tests, and phone deep-link routing unit test pass.
- Signed development build 2330 archived successfully. Its signature, package integrity, and
  URL scheme were verified. USB installation and a separate device lookup confirmed build 2330.
  CoreDevice timed out when asked to launch it. XCTest interaction checks remain open.
