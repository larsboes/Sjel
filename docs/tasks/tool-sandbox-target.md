---
id: tool-sandbox-target
title: "Select a writable build target"
state: done
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
evidence:
  - "tools/cargo-hermetic.test.sh passes: a denied inherited cache falls back to a stable checkout-keyed target, an explicit unwritable target is refused, and the live checkout targets stay protected."
  - "The fixture checks CARGO_TARGET_DIR at Cargo execution and in scratch-mode --print-env; tools/cargo-hermetic also reports the fallback path when redirecting a build."
---

# Select a writable build target

## Outcome

A sandboxed build reports and reuses its selected writable target.

## Acceptance

- Select through existing build tooling rather than hardcoded scratch paths.
- Preserve live shared targets and report the actual target in verification evidence.
