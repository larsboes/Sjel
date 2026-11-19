---
id: "gpui-platforms"
title: "Verify additional desktop platforms"
state: "backlog"
area: "gpui"
kind: "investigation"
priority: 3
spec: "docs/architecture/native-ui-boundaries.md"
---

# Verify additional desktop platforms

## Outcome

Verify additional desktop platforms through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Review Windows keychain integration and a non-macOS PDF renderer through existing platform boundaries.
- Launch/package on a real Linux desktop; cross-compilation alone does not prove credentials/rendering work.
- Android host work remains deferred phone expansion and must reuse NSH/shared rules rather than a second business layer.
