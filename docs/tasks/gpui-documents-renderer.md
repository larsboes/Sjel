---
id: "gpui-documents-renderer"
title: "Move phone Documents to shared GPUI"
state: "verify"
area: "gpui"
kind: "maintenance"
priority: 4
spec: "docs/architecture/native-ui-boundaries.md"
---

# Move phone Documents to shared GPUI

## Outcome

The normal iPhone Documents route mounts the shared GPUI UI with Ely controls in the Swift host.
Remove the phone route's NSH renderer when the GPUI route takes over.

## Acceptance

- Resolve one GPUI engine in the iPhone target. Use the existing GPUI Mobile fork to embed the
  view, with the same `gpui-pre` snapshot that GPUI Kit and Ely resolve.
- Preserve signed pairing, paging, offline originals, file protection, and the native PDF
  interaction in [phone Documents acceptance](gpui-documents-phone.md).
- Check the normal route on a physical iPhone for scrolling, text composition, selection,
  safe areas, navigation back to Today, and background and foreground transitions.
- Cover the migrated route's visible rendering and actions in iPhone UI tests. Record the
  current VoiceOver limitation. VoiceOver parity is deferred for this migration.

## Evidence

- The normal Docs tab and the Documents deep link resolve to the same GPUI surface. The shared
  phone window owns the surrounding navigation.
- The arm64 iOS Simulator app and XCTest runner built and linked with Xcode 27.2 in a
  focused checkout. The iOS dependency tree resolves one GPUI engine.
- iPhone UI test cases cover initial launch into Documents and switching from Mail into
  Documents and back. CoreSimulator refused connections, and CoreDevice enumeration timed out.
  Runtime tests and physical-device acceptance remain open.
