---
id: tool-archive-cleanup-review
title: "Review archive cleanup candidates"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Review archive cleanup candidates

## Outcome

An archive operator can review exact duplicate files before changing an archive source or retiring a backup.

## Acceptance

- Derive the source from the archive capability configuration and group files by size and SHA-256 without following symlinks.
- Keep candidate paths in a private local report. Report logical bytes separately from estimated backup savings.
- Reject changed files during scanning. Leave the source, backups, receipts, and scheduling untouched.

## Evidence

A manual read-only hash inventory with a disposable local report supplies the review until the archive owner exposes a repeatable command.
