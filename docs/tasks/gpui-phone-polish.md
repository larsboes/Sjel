---
id: "gpui-phone-polish"
title: "Fit shared views to the phone width"
state: "done"
area: "gpui"
kind: "fix"
priority: 4
spec: "docs/architecture/native-ui-boundaries.md"
evidence:
  - "Confirmed on a physical iPhone by the maintainer on TestFlight build 2384 (352b3c11) on 2026-11-21."
---

# Fit shared views to the phone width

## Outcome

Shared GPUI views read correctly at mobile density on a physical iPhone, with each fix checked on the device.

## Acceptance

- Calendar's week strip shows each day's label instead of `…`.
- The Projects search field keeps a usable width, and `+ New Project` stays inside the screen.
- The note editor shows no desktop key hints (`⌘S`, `⌘[`, `Esc to close`) at mobile density.
- The maintainer confirms each fix on a physical iPhone.

## Evidence

- Seen on build 2357 from `apps/gpui/ios/run-device` on 2026-11-21: all four defects above.
- The phone exposes no accessibility tree to `interceptor ios tree`, so device checks tap by coordinate.
- Code fixes and headless width checks implemented:
  1. **Calendar week strip (`apps/gpui/ui/src/calendar/mod.rs`, `apps/gpui/ui/src/calendar/phone.rs`):** Replaced single-line `Button` day cells (which applied `Ellipsis` inside `px_2p5` horizontal padding and truncated `"M\n21•"` to `"…"` at 375–390pt phone width) with `mobile_week_strip` two-line compact day pills. Headless test `calendar::tests::mobile_week_strip_renders_all_seven_day_labels_within_phone_width` verifies all 7 day cells and labels render within a 366pt strip.
  2. **Projects search field & `+ New Project` (`apps/gpui/ui/src/projects/render.rs`, `apps/gpui/ui/src/projects/tables.rs`):** Updated `toolbar` in `Hubs` and `filter_bar` in `AllTasks` and `AllNotes` to stack vertically (`flex_col().items_stretch()`) with `w_full()` on the search box and `flex_wrap()` on the action buttons when `density == Density::Mobile`. Headless test `projects::tests::mobile_projects_toolbar_keeps_search_wide_and_new_project_on_screen` verifies `projects-hub-search-box` and `new-project-btn-host` stay inside a 375pt phone viewport.
  3. **Note editor desktop key hints (`apps/gpui/ui/src/editor/mod.rs`, `apps/gpui/ui/src/projects/mod.rs`):** Propagated `Density` into `NoteEditor` and hid both `"Esc to close · / for blocks"` and the footer `⌘S` / `⌘[` / `⌘⇧↑/↓` key bar when at `Density::Mobile` or on iOS. Headless test `editor::tests::mobile_density_hides_desktop_key_hints_in_editor` verifies both selectors are absent at mobile density.
- The maintainer checked all three fixes on TestFlight build 2384 from `352b3c11` and accepted them
  on 2026-11-21. No device screenshots were recorded.
