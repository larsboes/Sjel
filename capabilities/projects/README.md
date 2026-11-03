# projects

Sjel's record store for projects, tasks, notes and their attachments. The CLI and API share the same write operations.

## Why this shape: records with rendered files

A record's Markdown path and bytes are stored together, with queryable properties derived from that content. Writes render the complete record under the overlay's `data/projects/` tree. The store remains authoritative; the rendered tree provides ordinary files for inspection and backup.

Attachments stay as files rather than database rows. Projects also owns reviewed references to Inventory items; those references record what was checked, not a guarantee that the item remains unchanged.

## Run

```sh
projects list
projects projects
projects add
projects new-project
projects new-note
projects verify
```

The service is managed through `tools/service-runner.sh` and its `service.toml` manifest. The live HTTP contract is available at `GET /routes`.

## Considered and declined

- **No Knowledge import.** Knowledge remains in its owning knowledge base; Projects imports project records and their attachments.
