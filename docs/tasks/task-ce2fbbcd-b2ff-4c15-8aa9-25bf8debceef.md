---
id: task-ce2fbbcd-b2ff-4c15-8aa9-25bf8debceef
title: "Redesign People around the network"
state: backlog
area: gpui
kind: feature
priority: 2
depends_on: ["task-411e5df2-1cde-4271-9a94-54903cbaa90a"]
spec: docs/architecture/native-ui-boundaries.md
---

# Redesign People around the network

## Outcome

The Mac People page finds a person fast and shows how you reach them: through whom you met,
when you last spoke, and who is due for a follow-up.

## Acceptance

- Done in code: the list filters by origin (All, LinkedIn, Google, Added by you) with counts,
  and sorts by name, last contact or connected day (`view_people` in
  `apps/gpui/core/src/people.rs`). The row shows the day that the sort uses.
- Done in code: Mac and phone detail show LinkedIn, Connected, Last contact and Sources rows.
- Open: check both in the installed app with the real import. Not run.
- Open: search also matches aliases. Filter by company and by overdue follow-up.
- How we met: a person can name the event, project or person that introduced them. An
  introduction by a person is an edge to that entity, not free text.
- Last contact reads the existing `last_contact` value (`crates/entities/src/journal.rs`). Mail
  and calendar links add to it only through their owners.
- Follow-up cadence: a per-person interval in weeks. The list marks a person as overdue when
  `last_contact` plus the interval is before today.
- A graph view draws people as nodes. Edges are introductions, a shared current company, and
  shared calendar events. Each edge names its source.
- Second-degree paths come only from these edges. LinkedIn's export holds no connections of
  connections. Scraping LinkedIn is out of scope.

## Open decisions

- Where introduction edges live: a new Entities fact predicate, or `sjel-links`.
- Whether the graph view is a Mac-only view or a shared `apps/gpui/core` model for the phone.
