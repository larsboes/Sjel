---
id: "gpui-phone-navigation"
title: "Accept shared phone navigation and lifecycle"
state: "verify"
area: "gpui"
kind: "maintenance"
priority: 4
spec: "docs/architecture/native-ui-boundaries.md"
---

# Accept shared phone navigation and lifecycle

## Outcome

Accept shared phone navigation and lifecycle through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Check Today, Capture, Projects, People/Places, Settings/pairing, Interior and all side-panel seats on a paired phone, including writes/sync, capture/upload/preview and saved selections.
- Verify audio routing/pause-on-panel-close, background terminal locking, safe areas, runtime light/dark changes and VoiceOver.
- Run a linked host build and native simulator/device path; Rust/Swift compilation does not establish runtime acceptance.
- Review legacy RoomPlan exports and retired app installation explicitly; do not delete a legacy app holding unverified exports.
