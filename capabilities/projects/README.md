# projects

Projects, tasks and project notes. Sjel owns them since the import from the Obsidian vault on
2026-10-18. The open claims are in the projects unit of [`ISA.md`](../../ISA.md).

One row in the shared store is one markdown file: its path and its bytes. Kind, project, title,
done, due, priority and archived are derived from those two on every write, so SQL can query them
and a store rebuilt from the files equals the live one. Every write also renders the file under
`<overlay>/data/projects/`, whole file, temp-plus-rename, mode 0600. Nobody edits that tree; it is
the second copy Principle 8 asks for, the way finance renders its decisions.

The API binds to loopback, needs the operator token and refuses foreign browser origins. The CLI
and the API call the same functions in `src/ops.rs`.

## Why this shape: Sjel owns projects, tasks and habits

Decided 2026-10-09 and revised 2026-10-17. It reverses the PRD's Q48. Sjel owns projects, tasks
and habits. Obsidian keeps Knowledge, Atlas, Journal and Reflections. The 2026-10-17 revision
stores the records as tables in the shared store instead of keeping markdown canonical, after
the operator compared both. Sjel's CLI and API are the only writers.

Attachments stay files in the rendered tree, not rows. At 986 MB on 2026-10-18 they would grow
every backup of the shared database. The page that listed apps under `/projects` moved to
`/apps`, and `/projects` is this capability.

Habits are not built. Two questions are open: which `calendar` rhythm fields carry a habit, and
whether the daily metrics in the vault's `Journal/01. Daily Notes` (read by the vault's
`Habits.base`) move here or stay journal entries. The vault's Journal domain note says nothing is
written back into a journal entry.

## Interface

- `GET /api/tasks?status=open|done`: tasks of live projects. Decision-ladder band 620 reads it;
  the shape is the one the retired vault task reader served. Two keys are added: `project`, the
  folder, and `fields`, every frontmatter key with lists flattened to `a, b`.
- `GET /api/projects`, `GET /api/project?name=`: the list and one project's records.
- `GET /api/records?kind=project|task|note`: every live record of one kind in the task shape,
  with `kind` and all its properties. The dashboard's tables and note embeds read it.
- `POST /api/tasks`, `POST /api/tasks/done`, `POST /api/projects`, `POST /api/notes`: create and
  complete.
- `POST /api/record/property {path, key, value}`: set one frontmatter key; an empty value clears
  it. Only that key's lines change. Any key is allowed except `type`, `done` and `moved_to`.
  The server checks `priority` (1 to 3), `due` and `scheduled` (dates), and a task's `status`.
- `POST /api/record {path, content, base}`: replace a record. A `base` that no longer matches the
  store returns 409 with the current content and writes nothing.
- CLI `projects`: `list`, `projects`, `add`, `new-project`, `new-note`, `edit` (opens `$EDITOR`),
  `done`, `reopen`, `set`, `normalize`, `import`, `freeze-vault`, `verify`, `render`.

## Task schema

A task's `status` is `open`, `doing`, `waiting` or `done`. `done:` follows `status`; setting
`status: done` also writes `completed:`. `projects normalize` brings old notes onto this
schema, and `done:` decides their status. It ran on 2026-10-18 and changed 27 tasks. A project's
`status` is `active`, `done` or `cancelled`; archiving is the move to `Archive/`.

## Embedded bases

A project note that embeds a vault base (`![[Tasks.base#…]]`) shows a live table in the
dashboard when the view reads Sjel's records: tasks that link the note, projects in its folder,
records one folder down. `dashboard/src/lib/projects/database.ts` translates those views from the
vault's `Resources/Bases/*.base`. Every other base reads vault data and stays a link to the vault.

## The vault import

`projects import <vault>/Projects` copies every `.md` of each project folder. `Archive/` comes in
as archived. Nothing comes in from `Sjel/Knowledge-Base/`: it is vault law and stays in the vault.
Every other file of a project folder comes in as an attachment, kept as a file under
`<overlay>/data/projects/` and served by `GET /api/file`. A row or attachment changed in Sjel
after its import is never overwritten.

`projects freeze-vault <vault>/Projects` then adds `moved_to:` and a banner to each imported note
whose bytes still equal its row. The vault reader skips notes with `moved_to:`, so no task is
served twice. The vault removed the frozen copies on 2026-10-20, when this capability became the
final owner.

`projects verify` compares the store with the rendered tree byte for byte; `projects render`
repairs a stale file.
