---
id: posts-history
depends_on: ["posts-reader-owner"]
title: "Import past LinkedIn posts"
state: backlog
area: posts
kind: feature
priority: 3
spec: docs/plans/posts.md
---

# Import past LinkedIn posts

## Outcome

Reuse the existing LinkedIn reader for past-post comparisons.

## Acceptance

- A complete Shares.csv and Comments.csv export imports idempotently.
- The corresponding Member Data Portability API domain uses the same reader.
- Each post retains its date and source.
