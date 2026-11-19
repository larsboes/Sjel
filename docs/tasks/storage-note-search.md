---
id: storage-note-search
title: "Accept installed note-path search"
state: verify
area: storage
kind: fix
priority: 1
spec: docs/plans/storage.md
blocked_by: "The installed app is present, but the native inspection CLI exits without a window tree."
---

# Accept installed note-path search

## Outcome

Accept installed note-path search through the existing owner, without treating implementation as live acceptance.

## Acceptance

- In the installed app, title and relative-path search display a note's path and open that note.
- Resolve native-window inspection failure before claiming acceptance; body search is outside this task.

## Verification

The installed desktop bundle was found. Native inspection returned no result, so title search,
path display, and opening the result remain unverified in the installed app.
