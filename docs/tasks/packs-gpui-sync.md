---
id: packs-gpui-sync
title: "Refresh GPUI guidance copies"
state: backlog
area: packs
kind: investigation
priority: 1
spec: docs/plans/packs.md
---

# Refresh GPUI guidance copies

## Outcome

Materialized GPUI skill copies match the owned source.

## Acceptance

- From an unrestricted terminal, sync coding with drift/profile guards to Claude, Codex, OpenCode and Copilot.
- Status marks all four GPUI copies current; install no upstream skill.
- Run the three defined behavioral scenarios separately from revision lookup checks.

## Evidence

- Ran `tools/harnesses sync coding` and `tools/harnesses drift coding`: `gpui` synced cleanly to Claude Code, Codex, opencode, and GitHub Copilot CLI with zero remaining drift against `Packs/coding/skills/gpui`.

## Unrun checks

- Execute the three defined behavioral scenarios separately from revision lookup checks.
