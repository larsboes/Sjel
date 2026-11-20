---
id: tool-ios-host
title: "Validate native iOS screens"
state: verify
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Validate native iOS screens

## Outcome

Physical-screen acceptance and simulator access no longer require a manual workaround.

## Acceptance

- Confirm the updated physical build for Documents, Mail and navigation in light/dark.
- Provide shell access to host simulator services without weakening existing screen assertions.
- Regenerate the Xcode project on repeat builds. XcodeGen can leave an incomplete project
  directory after replacement fails. Generating the same spec in a fresh directory links the app.

## Evidence

- Xcode 27.2 lists the paired iPhone under Xcode → Open Developer Tool → Device Hub.
  Command-line CoreDevice discovery times out and CoreSimulator rejects connections.
- The paired USB installation service reads the installed Sjel bundle version and accepts a
  signed developer app bundle while CoreDevice is unavailable. A package update can be staged
  while Sjel is running. The developer bundle completed installation and a separate lookup
  confirmed the new version. Build 2323 ran on the iPhone; automated launch, screenshots, and
  XCTest remain unavailable from the workspace. A separate device lookup confirmed build 2330
  after the signed developer bundle was installed over USB.
- The archive command can reuse an explicit Cargo target directory on retry while still staging
  fresh Swift sources and a new Xcode project. Its `--development-ipa` option packages the
  signed app for local installation in Device Hub under Apps → Add. The package excludes
  macOS resource metadata that the phone would otherwise interpret as an extra bundle.
