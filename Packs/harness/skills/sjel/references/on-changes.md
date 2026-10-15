# Branch, commit, and push workflow

One focused change per commit. **A tracker entry is never a precondition for starting work.**

## Start

1. Work on `main`. Until the official release, do not create or switch branches unless the
   user asks. After the release, feature branches and a `dev` branch replace this rule.
2. Other agent sessions work in this checkout at the same time. If the tree is dirty, preserve
   it. Do not reset, clean, stash, revert, or absorb another session's changes.
3. Touch only the files of your task. If a file you need has another session's uncommitted
   edits, wait for that commit or ask the user.

## Work

- Keep the diff inside one coherent boundary.
- Stage by path. Never `git add -A` or `git commit -a` in a shared checkout.
- Write the commit message as the outcome, not the activity.
- Surface separable discoveries instead of silently widening the change.
- Run the proportionate checks in `references/on-verification.md`.

## Publish

1. Review the focused diff and final status.
2. Immediately before `git push`, run `git log --oneline origin/main..HEAD` again. A concurrent
   session can add or amend commits after your last look, and a fast-forward push publishes
   them silently. Push only what you described.
3. Push a small, verified change straight to `main`. GitHub prints
   `Bypassed rule violations for refs/heads/main`. That is expected until the release, not a
   finding. Open a PR only when the change wants review.

## When an issue is still worth filing

Only when something must outlive the change itself: a defect being left unfixed, a decision that
needs a record, or work handed to someone else. Read live issue and PR metadata when a task
already references one. Never infer its state from local commits.
