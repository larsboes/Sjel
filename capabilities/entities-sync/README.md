# entities-sync

The scheduled Obsidian sync for [`capabilities/entities`](../entities/). Every 6 hours it runs
`entities-server sync-obsidian`, which reads the people notes through vault's HTTP surface and
applies them to the entity store. Because it reads through vault, this job needs no Full Disk
Access of its own.

The job is separate because `entities` is an always-on HTTP service, and Sjel refuses a manifest
that declares both `autostart` and `schedule`. `entities` owns the data and the sync rule. This
manifest only schedules the run.

The rule is in `capabilities/entities/src/sync.rs`: a source changes a value only while that source
owns it. A value that the operator edited on the People page stays as it is.

Run it by hand, without writing:

```
target/release/entities-server sync-obsidian --dry-run
```
