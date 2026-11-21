---
id: tool-cli-preview-selection
title: "Select CLI installer previews by their inputs"
state: verify
area: tooling
kind: maintenance
priority: 3
spec: docs/operations/index.md#existing-draft-and-manual-installation
evidence:
  - "Local planner and workflow tests cover a Clipper-only update, shipped Pack and helper files, transitive Rust dependencies, and conservative fallback."
---

# Select CLI installer previews by their inputs

## Outcome

Unrelated pull requests do not allocate CLI installer runners. Changes to packaged inputs and
CLI build dependencies still rehearse installation on every supported preview platform.

## Acceptance

- A hosted run for a Clipper-only dependency change skips the three installer jobs.
- A hosted run for a packaging change or manual dispatch runs the install, update, and rollback
  checks on Linux x86-64, Linux ARM64, and macOS ARM64.
- An updated pull request cancels its superseded preview without interrupting main or manual runs.
