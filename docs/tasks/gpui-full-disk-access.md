---
id: "gpui-full-disk-access"
title: "Accept one branded Full Disk Access entry for Sjel"
state: "done"
area: "gpui"
kind: "maintenance"
priority: 2
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
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
- Enable the new **Sjel** entry without changing other grants. Run `sjel fda-launcher migrate`
  from a logged-in desktop session. Confirm it proves protected-folder access under launchd
  and switches the existing storage-history job before removing the obsolete helper row.

## Evidence

The helper lifecycle, nine distribution tests, five Desktop settings and permissions tests,
and 23 menu-bar tests pass. The installed background bundle passes strict signature verification.
System Settings shows the enabled entry as **Sjel.app** with the Sjel icon. The live launchd
probe passed. Both the installed and loaded storage-history job point to the bundle executable,
and persistence selects it for future installs.

On macOS 27.2.1, System Settings did not recognize a drag from Desktop. The **+** picker works.
The headless test checks drag initiation inside Desktop, not delivery to System Settings.
A Finder drag of the same bundle would distinguish a Desktop-specific failure from a rejection
of the bundle or drop target. The menu shortcut still needs a live check.
