# storage-reclaim

The scheduled reclaim pass for [`crates/storage`](../../crates/storage/README.md). Every 6 hours
it runs `sjel-storage reclaim --scheduled`. The pass acts only on classes the overlay's storage
policy bounds with `cap_gb` or `older_than_days`, and it refuses anything the policy protects or
a live process holds.

It runs four times a day, not once. A build often holds the target directory, and a daily run
then misses whole days. The reclaim waits out a running build itself, so the schedule only gives
it another chance soon.

The job runs the built binary, not `tools/storage/storage`. That launcher builds on demand, and
the job's PATH has `cargo` only because `build` names it.

Preview by hand, without deleting:

```
sjel storage reclaim --dry-run
```
