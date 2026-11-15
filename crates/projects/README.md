# projects

Sjel's record store for projects, tasks, notes and their attachments. The CLI and API share the same write operations.

## Why this shape: records and rendered files

A record's Markdown path and bytes are stored together, with queryable properties derived from that content. Writes render the complete record under the overlay's `data/projects/` tree. The store remains authoritative; the rendered tree provides ordinary files for inspection and backup.

Attachments stay as files rather than database rows. Projects also owns reviewed references to Inventory items; those references record what was checked, not a guarantee that the item remains unchanged.

## Run

```sh
projects list
projects projects
projects add <project> <title>
projects new-project <name>
projects new-note <project> <title>
projects verify
```

The service is managed through `tools/service-runner.sh` and its `service.toml` manifest. The live HTTP contract is available at `GET /routes`.

## Private delivery workspaces

`projects workspace` prepares projects from Markdown templates, adopts private project indexes and
delivery assets, reports checkout and optional GitHub CI state, and stages reusable lessons for
review. Personal values and delivery libraries remain in the active private overlay. The commands
never provision accounts, publish client products or introduce a shared customer runtime.

```sh
projects workspace adopt <private-manifest.json>
projects workspace setup <name> <template-directory>
projects workspace status [--github]
projects workspace lesson draft <project> <title> <evidence> [--proposal <concept.md>]
```

Adoption copies explicit asset roots into the overlay's `resources/` directory and creates private
project hubs. Repeating it compares bytes and refuses to replace edited records or assets. Setup
reads one template directory, substitutes `{{project}}`, and creates exactly one hub plus its notes
and tasks with class `c2`. A template contains no accepted provider or publication choice.
New tasks and notes inherit an explicitly private hub's class; record edits cannot lower it.

### One task authority

The overlay's optional `config/projects-sources.json` declares external task sources. The
[example](projects-sources.example.json) shows both formats. `notes` reads typed task notes, including
`typ: Aufgabe`, `wer`, `naechster_schritt` and the statuses `Offen`, `Wartet` and `Erledigt`.
`checklist` reads numbered actions and Markdown checkboxes; unmarked bullets and fenced code are
not tasks. Patterns select one file or a directory's `*.md` children, without recursive discovery.

The existing project, record and task reads include these live views. They never insert source
tasks into the store. Each view carries `read_only`, `source_path` and the source modification time.
Checklist identities include their source line, so reordering can change an identity. Source views
inherit at least the configured class; note frontmatter can raise it. A missing or invalid source
fails the read rather than reporting an empty backlog. Archived projects stop reading their sources.

Source tasks are read-only on both devices. The backend also refuses a parallel local task list
for a project with a declared source. The client repository's own workflow handles source edits,
completion and approval. The Mac reads project views through the capability; the phone keeps its
existing replica and refuses offline mutations of source tasks.

### Lessons remain proposals until reviewed

A lesson draft is a private project note with its evidence, mechanism, assumptions and limits.
The owner reviews a separate concept note after searching the vault for an existing owner. New
concepts carry `type: concept`, `maturity: seedling`, `summary`, `categories`, `sources` and a `c0`
or `c1` class. The vault's writing contract determines content and placement.

```sh
projects workspace lesson publish <draft-id> <reviewed-note.md> <vault-root> <Knowledge/path.md> --declassify <owner-rationale>
```

Publication requires an explicit confidentiality-review rationale, creates only a new note, and
refuses different existing bytes. Provenance and the review stay in the private project record.
Existing concept notes use the vault's editing workflow. No automatic extraction, class lowering,
note merging or claim of learner mastery occurs.

## Considered and declined

- **No Knowledge import.** Knowledge remains in its owning knowledge base; Projects imports project records and their attachments.
