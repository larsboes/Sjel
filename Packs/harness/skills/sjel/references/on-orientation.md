# Orient to repository work

Run this sequence before planning, reviewing, or editing:

1. Run `git status --short --branch`. Preserve unrelated changes and identify the active branch.
2. Run `tools/doctor`. Treat its findings independently and keep unrelated failures outside the
   change scope.
3. Run `tools/axon-context on [unit-or-path]`.
4. Read only the paths it returns plus the reference selected in `SKILL.md`.

Use `tools/self explain <unit>` for ownership and local coupling, and `tools/self coupling` for a
relationship question. Use `rg` and tracked source for exact implementation evidence. Use
Graphify only for symbol-level traversal when a current graph is available.

Before you conclude that Sjel does not do something, read `capabilities = [...]` in the
overlay's `config/machine.toml` (resolved by `tools/lib/paths.sh`) and list
`<overlay>/capabilities/`. Many capabilities live only in the overlay, so a search of this
repository gives false negatives. In a repository checkout (a cloud container, where `tools/doctor`
reports "repository checkout" and skips deployment sections) there is no overlay to read: state
that the overlay was unavailable instead of concluding that Sjel lacks the capability.

State the intended file and change scope before editing. Do not absorb nearby cleanup merely
because doctor, status, or a search exposed it.
