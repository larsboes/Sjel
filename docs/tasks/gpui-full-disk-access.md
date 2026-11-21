---
id: "gpui-full-disk-access"
title: "Accept Full Disk Access setup from Sjel Desktop"
state: "verify"
area: "gpui"
kind: "maintenance"
priority: 2
---

# Accept Full Disk Access setup from Sjel Desktop

## Outcome

Sjel Desktop opens the Full Disk Access pane and offers the installed signed helper as a native file drag. Finder provides a fallback. Only macOS grants access.

## Acceptance

- With Sjel and System Settings visible side by side, drag the helper into the Full Disk Access list and confirm that the pane accepts the executable.
- Confirm **Show in Finder** selects the same executable as `sjel fda-launcher path`. With no helper installed, the section shows the install command instead of a drag control.
- With owner approval, enable the grant and confirm `sjel fda-launcher verify` succeeds. Do not change other grants or service routing.

## Evidence

The desktop type check and five focused Settings and permissions tests pass, including headless drag initiation. The helper lifecycle and packaged path resolver tests pass. Installed-app placement, the System Settings drop, and the live grant probe remain unverified.
