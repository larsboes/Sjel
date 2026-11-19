---
id: tool-deploy-preflight
title: "Probe atomic deployment capability"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Probe atomic deployment capability

## Outcome

A denied destination rename is found before materializing a large Pack.

## Acceptance

- Probe required rename capability before deployment.
- Report the failing destination and retained stage; never publish a partial copy as success.
