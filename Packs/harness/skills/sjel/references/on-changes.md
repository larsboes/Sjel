# Branch, commit, and push workflow

One focused change per commit. **A tracker entry is never a precondition for starting work.**

## Choose the path

Until `PLAN.md` is done, every change goes straight to `main` from the primary checkout. There
are no work-package branches, worktrees or pull requests. When the plan is done, `main` is tagged
`v0.1.0`, and the work-package path below applies from then on. Decided 2026-10-31.

A cloud session receives its branch from the harness (`claude/`, `codex/`, `copilot/` and so on).
That branch is its work branch: the session opens a draft PR from it rather than pushing to
`main`.

## Work package (from `v0.1.0`)

1. Create the branch and worktree from current `main`:
   `sjel worktree new <area>-<slug>`. The worktree is
   `<primary checkout>/.claude/worktrees/<area>-<slug>`, and it receives the primary checkout's
   `sjel.local.toml`, `axon.local.toml` and `.claude/settings.local.json`.
2. Work, commit and run checks only inside that worktree. Its first build compiles into its own
   `target/`. Never build a release binary into the primary checkout from a worktree: the
   primary checkout's `target/release/` is what the services run.
3. Push with `git push -u origin <area>-<slug>` and open a draft PR (CONTRIBUTING.md, "Open the
   pull request"). Drive CI green, then mark it ready.
4. The PR lands as a **squash merge**: one commit on `main` per work package, which is the commit
   the area plan names. The agent merges only when the user said so for that work package or in
   the current request. Otherwise it stops at a green, ready PR.
5. After the merge, `sjel worktree remove <area>-<slug>` removes the worktree and the local branch.
   It refuses uncommitted changes, and commits on no remote unless their PR was merged.

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

## When an issue is still worth filing

Only when something must outlive the change itself: a defect being left unfixed, a decision that
needs a record, or work handed to someone else. Read live issue and PR metadata when a task
already references one. Never infer its state from local commits.
