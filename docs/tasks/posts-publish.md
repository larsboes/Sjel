---
id: posts-publish
depends_on: ["posts-drafts","posts-media","posts-history","posts-review","posts-people"]
title: "Publish approved LinkedIn posts"
state: backlog
area: posts
kind: feature
priority: 3
spec: docs/plans/posts.md
---

# Publish approved LinkedIn posts

## Outcome

Add the Posts API write only after drafting, export, ingest and review are accepted.

## Acceptance

- Use w_member_social with explicit owner approval per publish.
- A refused approval sends nothing.
- A successful publish records its URL in post.md.
