---
id: posts-drafts
title: "Create post draft folders"
state: verify
area: posts
kind: feature
priority: 3
spec: docs/plans/posts.md
---

# Create post draft folders

## Outcome

Keep current text, revisions and ordered media in one post folder.

## Acceptance

- posts new creates a schema-valid folder.
- posts status lists every post and its state.
- The installed Mac app's Posts page lists posts and saves text through sjel-status.

## Evidence

`crates/posts/check` passes every verb against a throwaway overlay. Live acceptance in the
installed app has not run.
