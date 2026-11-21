---
id: "gpui-full-disk-access"
title: "Accept one branded Full Disk Access entry for Sjel"
state: "verify"
area: "gpui"
kind: "maintenance"
priority: 2
---

# Accept one branded Full Disk Access entry for Sjel

## Outcome

The menu-bar app opens the macOS permissions section of Sjel Desktop. Desktop opens System
Settings and offers the signed background `Sjel.app` as a native file drag, with Finder as a
fallback. Only macOS grants access. The UI apps are not granted Full Disk Access.

## Acceptance

- Drag the bundle into the Full Disk Access list and confirm the entry appears as **Sjel**
  with the Sjel icon. Confirm **Show in Finder** reveals the bundle from
  `sjel fda-launcher app-path`. With no bundle installed, Desktop shows the install command.
- Open **macOS Permissions…** from the menu-bar app and confirm it opens the same Desktop section.
- Enable only the new **Sjel** entry. Run `sjel fda-launcher migrate` from a logged-in desktop
  session. Confirm it proves protected-folder access under launchd and switches the existing
  storage-history job, then remove the obsolete helper row. Leave other grants and job routing
  unchanged.

## Evidence

The helper lifecycle, nine distribution tests, five Desktop settings and permissions tests,
and 23 menu-bar tests pass. The installed background bundle passes strict signature
verification, contains the menu-bar icon, and leaves the previous executable selected for
persistence. The menu shortcut, System Settings drop, displayed icon and name, and real
launchd permission probe require live acceptance.
