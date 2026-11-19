---
id: posts-media
depends_on: ["posts-drafts"]
title: "Export ordered post photos"
state: verify
area: posts
kind: feature
priority: 3
spec: docs/plans/posts.md
---

# Export ordered post photos

## Outcome

Export full-resolution Photos items in post order and build a named collage.

## Acceptance

- Photos export downloads iCloud originals and applies edits without changing the library.
- Outputs use a 2400 px long edge, numbered in media order.
- A named grid layout crops to fill and produces the collage.
- The Mac Posts page adds the Photos selection, reorders slots and exports.

## Evidence

`POSTS_CHECK_PHOTOS` run of `crates/posts/check` exported a two-photo `lowpoly` collage from
the Photos library into a throwaway overlay. Live acceptance in the installed app has not run.
