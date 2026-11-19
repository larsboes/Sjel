---
id: tool-ci-timings
title: "Report CI job time and cache hits"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Report CI job time and cache hits

## Outcome

One command shows, for a CI run, how long each job took and whether its Rust cache step hit, missed
or restored a partial key.

## Need

Finding the cache split behind the slow Windows GPUI jobs and the uncached release build in
`bun test` took raw `gh run view --log` parsing. `tools/ci-cache` reports what the cache holds,
but not what each run read from it.

## Acceptance

- Extend `tools/ci-cache` with a run report instead of adding a second tool.
- For each job, show its duration, the restored cache key, and `full match` or `No cache found`.
- Show the runner image version per job, because the cache key follows the rustc on that image.
- Read only. The report deletes nothing.
