# Unattended local work

Use Pi's native headless execution for an approved task queue. This is not permission to choose
new work, make product decisions, publish, deploy, send messages or change access rules.

## Preflight with the user present

1. Resolve the task queue, owning area plans, acceptance checks, writable files and exclusions.
   Agree TDD test seams before starting. A missing spec or product decision blocks that task.
2. Complete authentication, project trust and required dependency installations. Package changes
   still require approval. Keep all secret guards and capability permission modes enabled.
3. Agree the deadline and spending limit. Verify how each will be enforced before launching.
   Native Pi session persistence, compaction and retries do not enforce a wall-clock or cost cap.
   If enforcement is unavailable, report that and obtain approval before an uncapped run.
4. Use a named, persistent session and an approved task file. Keep transcripts in private local
   state, not a published artifact. Do not use `--no-session` or disable extensions.

Pi's native entry point is `pi --mode json --session-id <id> @<approved-task-file>`.
It exits when the supplied work settles; it is not a scheduler or an automatic crash-restart loop.
Use `pi --session-id <id>` from the same directory to resume that session.

## Execute only the approved queue

- Implement and verify local changes. Create scoped local commits only when authorised, with
  explicit messages so no editor opens. Never stage another session's work.
- When approval or a new decision is needed, record the blocked task and continue independent
  approved work. In headless mode the command policy refuses Ask and Block calls without waiting
  for a card. Do not retry a refused action through another tool or command spelling.
- Do not call interactive question tools or invent answers. Leave the decision for the next
  attended session. Do not convert a recommended answer into consent.
- Check progress against executable evidence. After three attempts with the same failure and no
  new evidence, stop that task, record the failed assumption and continue only independent work.
- Checkpoint the owning area plan after each completed task. Finish when the queue is complete,
  the agreed limit is reached, or every remaining task is blocked. Report checks that did not run.

## Skill boundaries

Use Sjel's owned coding guidance and the owning area plan for implementation and verification.
Load named skills through Pi's file reader. Upstream skill snapshots are reference material,
not runtime instructions; do not load or reactivate held units. Sjel's placement, data,
verification and Git rules remain in force. Learning tasks use the owned vault-based `teach`
and `practice` skills.
