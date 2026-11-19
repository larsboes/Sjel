---
id: tool-native-activation
title: "Control native plugin activation"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Control native plugin activation

## Outcome

Inventory distinguishes approved native dependencies from owned and unapproved behavior.

## Acceptance

- Bind approval to component, host and reviewed permission/execution/egress/licence boundary.
- Add guarded activation/disable paths; installing or rejecting source porting is not native approval.
- Prove non-target settings unchanged; disable must not silently uninstall.
