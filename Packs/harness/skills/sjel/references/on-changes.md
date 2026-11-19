# Branch, commit, and push workflow

One focused change per commit. **A tracker entry is never a precondition for starting work.**

## Choose the path

Local changes currently go directly to `main` from the primary checkout. A maintainer activates
the work-package path by updating this section on `main` to make it the local default. Release
tags and task completion are independent of that switch. Tag creation requires its own authorization.

A cloud session receives its branch from the harness (`claude/`, `codex/`, `copilot/` and so on).
That branch is its work branch: the session opens a draft PR from it rather than pushing to
`main`.

## Work package

1. Create the branch and worktree from current `main`:
   `sjel worktree new <area>-<slug>`. The worktree is
   `<primary checkout>/.claude/worktrees/<area>-<slug>`, and it receives the primary checkout's
   `sjel.local.toml`, `axon.local.toml` and `.claude/settings.local.json`.
2. Work, commit and run checks only inside that worktree. Its first build compiles into its own
   `target/`. Never build a release binary into the primary checkout from a worktree: the
   primary checkout's `target/release/` is what the services run.
3. Push with `git push -u origin <area>-<slug>`, then follow
   [the pull request procedure](#prepare-a-pull-request).
4. The PR lands as a **squash merge**: one commit on `main` per work package, which is the commit
   the area plan names. The agent merges only when the user said so for that work package or in
   the current request. Otherwise it stops at a green, ready PR.
5. After the merge, `sjel worktree remove <area>-<slug>` removes the worktree and the local branch.
   It refuses uncommitted changes, and commits on no remote unless their PR was merged.

## Prepare a pull request

Open a draft pull request from the work branch. State the outcome and scope. List the completed
validation commands, their results, and any remaining verification gaps. Keep separate follow-ups
out of the pull request. Mark it ready once CI passes.

## Small change on `main`

1. Other agent sessions can work in the primary checkout at the same time. If the tree is dirty,
   preserve it. Do not reset, clean, stash, revert, or absorb another session's changes.
2. Touch only the files of your task. If a file you need has another session's uncommitted
   edits, wait for that commit or ask the user.
3. Immediately before `git push`, run `git log --oneline origin/main..HEAD` again. A concurrent
   session can add or amend commits after your last look, and a fast-forward push publishes
   them silently. Push only what you described.
4. GitHub prints `Bypassed rule violations for refs/heads/main` on a direct push. That is
   expected for this path, not a finding.

## Every change

- Keep the diff inside one coherent boundary.
- Stage by path. Never `git add -A` or `git commit -a` in a shared checkout.
- Write the commit message as the outcome, not the activity.
- Surface separable discoveries instead of silently widening the change.
- Run the proportionate checks in `references/on-verification.md`.
- Never rewrite history on a branch another session or the user works on.

## Hand off unfinished work

Keep the area plan current as work progresses and before stopping: built work, open work,
decisions and checks that did not run belong there. Do not automatically queue handoff prompts.
When the user requests a handoff:

1. Write the durable part into the area plan first: decisions, work that is built, work that is
   open, and checks that did not run. The prompt points at the plan. It does not repeat it.
2. Put only the volatile part into the prompt: commits that are not pushed, the state of the
   build, files that another session holds, and the first question for the user.
3. Queue the prompt with `comms prompts add --title "Sjel handoff: <area> (<date>)"`, and pass
   the body on stdin. If `comms` is not on `PATH`, use `target/debug/comms`. Confirm the
   `queued draft <id>` receipt. The user copies the prompt from Today ("Prompts for you") into
   the next session.
4. If the outbox is not available, show the prompt in one code block and say why.

Keep handoff prompts out of tracked files.

## When an issue is still worth filing

Only when something must outlive the change itself: a defect being left unfixed, a decision that
needs a record, or work handed to someone else. Read live issue and PR metadata when a task
already references one. Never infer its state from local commits.
