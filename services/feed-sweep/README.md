# feed-sweep

The scheduled source collector for [`crates/comms`](../../crates/comms/). Every 6 hours it runs
the `feed-sweep` verb of [`crates/sjel-cli`](../../crates/sjel-cli/README.md), which calls comms'
`POST /sources/scan` once and exits. Before this job
existed, only manual triggers called that route. On 2026-08-30 the newest feed item was nine days
old on a machine that reported healthy.

The job talks to comms over HTTP and never opens comms' database
([CONTRIBUTING.md](../../CONTRIBUTING.md#schemas-and-dependency-direction)). comms owns the feed.
This manifest owns only the interval. `services/feed-sweep/service.toml` records why the
schedule cannot live on comms' own manifest, and why the interval is 6 hours.

Run it by hand:

```
tools/feed-sweep
```
