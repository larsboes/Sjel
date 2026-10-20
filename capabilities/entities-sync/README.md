# entities-sync

The scheduled Journal sync for [`capabilities/entities`](../entities/). Every 6 hours it runs
`entities-server sync-journal`, which reads the Journal through the `projects` HTTP surface and
sets each person's `last_contact` to the latest dated entry that links their name. The date only
moves forward, so a later day typed on the People page stays.

Until 2026-10-20 this job ran `sync-obsidian` against the vault's `Atlas/People` notes. Those
notes and the Journal left the vault that day (`capabilities/entities/README.md`, "Import").

The job is separate because `entities` is an always-on HTTP service, and Sjel refuses a manifest
that declares both `autostart` and `schedule`. `entities` owns the data and the rule
(`capabilities/entities/src/journal.rs`). This manifest only schedules the run.

Run it by hand, without writing:

```
target/release/entities-server sync-journal --dry-run
```
