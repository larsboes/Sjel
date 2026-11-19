---
id: cli-preview-acceptance
title: "Accept checkout-free preview artifacts"
state: verify
area: tooling
kind: maintenance
priority: 2
spec: docs/plans/tooling.md
---

# Accept checkout-free preview artifacts

## Outcome

Publishable CLI artifacts pass independent installation and notice checks.

## Acceptance

- Fresh CI checks corrected notice/source-bearing archives on Linux x86-64/ARM64 and macOS ARM64 against the exact committed inputs.
- Real binaries pass isolated-home install/update/rollback; installed Pi loading passes on macOS.
- Review production dependency source/licence obligations and keep friend_ready false until clean-user acceptance.
- Publication needs separate approval; only afterward verify official-channel discovery and a real download/update.
- Do not modify existing tags or claim older glibc/musl support.
