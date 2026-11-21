---
id: task-ce2fbbcd-b2ff-4c15-8aa9-25bf8debceef
title: "Redesign People around the network"
state: "done"
area: gpui
kind: feature
priority: 2
depends_on: ["task-411e5df2-1cde-4271-9a94-54903cbaa90a"]
spec: docs/architecture/native-ui-boundaries.md
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Redesign People around the network

## Outcome

The Mac People page finds a contact, shows the evidence for a connection, and identifies overdue
follow-ups.

## Acceptance

- In the installed Mac app with imported contacts, search by alias, combine company and overdue
  filters, and check the existing origin counts and date sorts. A contact becomes overdue only
  after the due day, computed from the stored `last_contact` and a positive interval in weeks.
- Record an event or project name under How we met. Add and remove an introduction to another
  contact; the Entities graph stores it as a directed link, not as a name string.
- Open Network and select a contact by node and keyboard. Verify that introductions and imported
  group conversations show their source. Second-degree paths use two recorded links; a shared
  company groups contacts but does not create a link.
- Confirm the existing LinkedIn, Connected, Last contact and Sources rows against the real import
  on Mac and phone.

## Evidence

- The Mac package passes a locked Cargo check. The focused People suites pass, including
  headless node selection and keyboard navigation and a synthetic 500-contact layout.
- The Entities and Entities API suites pass, including the introduction HTTP round trip and
  cadence validation.
- The installed app, live Entities refresh and real-import checks have not run.

## Scope

Entities owns introduction edges. The model lives in shared core; the Network view is Mac-only.
Calendar links and phone graph rendering are deferred. The graph does not infer LinkedIn
connections of connections, and no scraping is in scope.
