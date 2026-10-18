# projects

Projects, tasks and project notes. Sjel owns them since the import from the Obsidian vault; the
decision and its claims are in [`ISA.md`](../../ISA.md), F14 and Decisions 2026-10-17 / 2026-10-09.

One row in the shared store is one markdown file: its path and its bytes. Kind, project, title,
done, due, priority and archived are derived from those two on every write, so SQL can query them
and a store rebuilt from the files equals the live one. Every write also renders the file under
`<overlay>/data/projects/`, whole file, temp-plus-rename, mode 0600. Nobody edits that tree; it is
the second copy Principle 8 asks for, the way finance renders its decisions.

The API binds to loopback, needs the operator token and refuses foreign browser origins. The CLI
and the API call the same functions in `src/ops.rs`.

## Interface

- `GET /api/tasks?status=open|done`: tasks of live projects, in the shape `capabilities/vault`
  serves, so decision-ladder band 620 reads it unchanged.
- `GET /api/projects`, `GET /api/project?name=`: the list and one project's records.
- `POST /api/tasks`, `POST /api/tasks/done`, `POST /api/projects`, `POST /api/notes`: create and
  complete.
- `POST /api/record {path, content, base}`: replace a record. A `base` that no longer matches the
  store returns 409 with the current content and writes nothing.
- CLI `projects`: `list`, `projects`, `add`, `new-project`, `new-note`, `edit` (opens `$EDITOR`),
  `done`, `reopen`, `import`, `freeze-vault`, `verify`, `render`.

## The vault import

`projects import <vault>/Projects` copies every `.md` of each project folder. `Archive/` comes in
as archived. From `Sjel/` only notes typed `project` or `task` come in; its `Knowledge-Base/` is
vault law and stays. Binaries stay in the vault. A row changed in Sjel after its import is never
overwritten.

`projects freeze-vault <vault>/Projects` then adds `moved_to:` and a banner to each imported note
whose bytes still equal its row. The vault reader skips notes with `moved_to:`, so no task is
served twice. The frozen notes are the backup until the owner is final.

`projects verify` compares the store with the rendered tree byte for byte; `projects render`
repairs a stale file.
