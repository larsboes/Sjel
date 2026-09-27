# Failure policy

Degrade honestly. A missing optional layer narrows the claim; it does not turn into success and
does not automatically block unrelated work.

| Missing or failing layer | Response |
| --- | --- |
| Axon checkout | Work inside the checkout or set the existing `SJEL_ROOT`; do not guess a path. |
| Private overlay | Continue only with public-tree work; do not invent machine or personal values. |
| Dirty worktree | Preserve all changes, identify ownership, and avoid switching or rewriting overlapping files. |
| Live capability | Use registry and logs to diagnose; never substitute a remembered port or claim an API check passed. |
| `tools/self` or stale `self.json` | Read the target manifests and tracked files directly; report the unavailable self-model layer. |
| Graphify | Use `tools/self`, `rg`, manifests, and source navigation. Graphify is optional. |
| GitHub metadata | Continue local analysis, mark issue or PR state unverified, and do not infer it from commits. |
| Build tool or dependency | Run narrower available checks and report exactly what remains unvalidated. |
| Doctor failure | Treat each finding independently; fix only findings inside the issue scope. |

Stop for direction when continuing would require a destructive action, secret handling, unrelated
cleanup, expansion into private data, or a product decision not fixed by the current issue.
