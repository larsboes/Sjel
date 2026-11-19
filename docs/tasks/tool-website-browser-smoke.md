---
id: tool-website-browser-smoke
title: Reuse the website browser smoke check
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/website.md
---

# Reuse the website browser smoke check

## Outcome

One repository command checks keyboard navigation and layout against a built static site.

## Acceptance

- Verify the skip link, task navigation, theme changes, and horizontal overflow at phone and desktop widths.
- Run in an isolated browser session against the local build. Report browser launch failure separately from a page assertion failure.

## Current workaround

A disposable Rex session can run an installed browser automation runner when the command shell
cannot start Chromium. The smoke check serves only generated site files under a synthetic origin.
